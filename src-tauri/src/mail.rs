//! The work behind the commands, shared by the window and the MCP server: one open IMAP connection per account is
//! kept and reused (a broken one is replaced once), everything runs with a time limit, and the cache is kept up to date.

use crate::accounts::{self, Account};
use crate::compose::{self, Draft};
use crate::imapc::{self, Folder, Page, Session};
use crate::store;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

const LIMIT: Duration = Duration::from_secs(120);

/// Runs `f` on its own thread and gives up after two minutes (a server that stops answering must not hang the app).
pub fn with_timeout<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || { let _ = tx.send(f()); });
    rx.recv_timeout(LIMIT).unwrap_or_else(|_| Err("TIMEOUT|".into()))
}

fn pool() -> &'static Mutex<HashMap<String, Session>> {
    static P: OnceLock<Mutex<HashMap<String, Session>>> = OnceLock::new();
    P.get_or_init(Default::default)
}

pub fn drop_session(id: &str) {
    if let Ok(mut p) = pool().lock() { p.remove(id); }
}

pub fn account(id: &str) -> Result<Account, String> {
    accounts::load(&store::data_dir()).into_iter().find(|a| a.id == id).ok_or_else(|| "NOACCOUNT|".to_string())
}

/// A broken connection: worth one more try with a new one.
fn broken(e: &str) -> bool { e.starts_with("CONNECT|") || e.starts_with("IMAP|") || e.starts_with("BAD|") }

/// Runs `f` with the account's connection.
pub fn session<T>(id: &str, f: impl Fn(&mut Session) -> Result<T, String>) -> Result<T, String> {
    let kept = pool().lock().ok().and_then(|mut p| p.remove(id));
    if let Some(mut s) = kept {
        match f(&mut s) {
            Err(e) if broken(&e) => {}
            r => { if let Ok(mut p) = pool().lock() { p.insert(id.to_string(), s); } return r; }
        }
    }
    let a = account(id)?;
    let pass = accounts::password(id)?;
    let mut s = imapc::connect(&a, &pass)?;
    let r = f(&mut s);
    if !matches!(&r, Err(e) if broken(e)) { if let Ok(mut p) = pool().lock() { p.insert(id.to_string(), s); } }
    r
}

pub fn folders(id: &str) -> Result<Vec<Folder>, String> {
    let f = session(id, imapc::folders)?;
    store::save_folders(&store::data_dir(), id, &f);
    Ok(f)
}

/// The folders, from the cache when known (to find Sent, Trash …).
fn known_folders(id: &str) -> Result<Vec<Folder>, String> {
    let f = store::folders(&store::data_dir(), id);
    if f.is_empty() { folders(id) } else { Ok(f) }
}

pub fn role_folder(id: &str, role: &str) -> Result<Option<String>, String> {
    Ok(known_folders(id)?.into_iter().find(|f| f.role == role && f.selectable).map(|f| f.id))
}

pub fn page(id: &str, folder: &str, limit: u32, before: Option<u32>) -> Result<Page, String> {
    let p = session(id, |s| imapc::page(s, folder, limit.clamp(1, 1000), before))?;
    if before.is_none() { store::save_page(&store::data_dir(), id, folder, &p); }
    Ok(p)
}

/// The whole message: from the cache, else from the server (and then kept).
pub fn raw(id: &str, folder: &str, uid: u32, validity: u32) -> Result<Vec<u8>, String> {
    let dir = store::data_dir();
    if let Some(r) = store::message(&dir, id, folder, validity, uid) { return Ok(r); }
    let (r, v) = session(id, |s| imapc::message(s, folder, uid))?;
    store::save_message(&dir, id, folder, v, uid, &r);
    Ok(r)
}

pub fn move_to_role(id: &str, folder: &str, uids: &[u32], role: &str) -> Result<String, String> {
    let target = match role_folder(id, role)? {
        Some(t) => Some(t),
        None if role == "archive" => {
            session(id, |s| imapc::create_folder(s, "Archive"))?;
            folders(id)?;
            Some("Archive".to_string())
        }
        None if role == "trash" => None,
        None => return Err(format!("NOFOLDER|{role}")),
    };
    match target {
        Some(t) if t != folder => { session(id, |s| imapc::move_to(s, folder, uids, &t))?; Ok(t) }
        _ if role == "trash" => { session(id, |s| imapc::delete_now(s, folder, uids))?; Ok(String::new()) }
        _ => Ok(folder.to_string()),
    }
}

/// Sends; then the copy in Sent, the answered flag, and the old draft removed. Gives back what happened to the copy.
pub fn send(d: &Draft) -> Result<String, String> {
    let a = account(&d.account_id)?;
    let pass = accounts::password(&a.id)?;
    let bytes = compose::send(&a, &pass, d)?;
    // After this point the message is out: problems only show as a note.
    let copy = if a.server_saves_sent { "server".to_string() } else {
        match role_folder(&a.id, "sent").and_then(|f| f.ok_or_else(|| "NOFOLDER|sent".into())).and_then(|f| session(&a.id, |s| imapc::append(s, &f, &bytes, false))) {
            Ok(_) => "saved".into(),
            Err(e) => format!("failed: {e}"),
        }
    };
    if let (Some(uid), false) = (d.answer_uid, d.answer_folder.is_empty()) {
        let _ = session(&a.id, |s| imapc::set_flag(s, &d.answer_folder, &[uid], "answered", true));
    }
    if let Some(uid) = d.draft_uid { remove_draft(&a.id, uid); }
    Ok(copy)
}

fn remove_draft(id: &str, uid: u32) {
    if let Ok(Some(f)) = role_folder(id, "drafts") { let _ = session(id, |s| imapc::delete_now(s, &f, &[uid])); }
}

/// Puts the draft into the Drafts folder (replacing the earlier version); gives back its new UID when the server says.
pub fn save_draft(d: &Draft) -> Result<Option<u32>, String> {
    let a = account(&d.account_id)?;
    let bytes = compose::draft_bytes(&a, d)?;
    let f = match role_folder(&a.id, "drafts")? {
        Some(f) => f,
        None => { session(&a.id, |s| imapc::create_folder(s, "Drafts"))?; folders(&a.id)?; "Drafts".into() }
    };
    let uid = session(&a.id, |s| imapc::append(s, &f, &bytes, true))?;
    if let Some(old) = d.draft_uid { remove_draft(&a.id, old); }
    Ok(uid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_limit() {
        assert_eq!(with_timeout(|| Ok(5)).unwrap(), 5);
        assert_eq!(with_timeout(|| Err::<(), _>("NO|x".to_string())).unwrap_err(), "NO|x");
        assert!(broken("CONNECT|reset") && !broken("AUTH|wrong"));
    }
}
