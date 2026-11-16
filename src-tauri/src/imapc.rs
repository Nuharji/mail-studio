//! The IMAP side: signing in, folders, the message list, whole messages, flags, moving, deleting, searching and
//! putting messages into a folder (Sent, Drafts). Errors start with a kind the UI translates ("AUTH|…", "CONNECT|…").

use crate::accounts::{is_local, Account, Security};
use crate::parse::{self, Summary};
use imap::{ClientBuilder, ConnectionMode, TlsKind};
use serde::{Deserialize, Serialize};

pub type Session = imap::Session<imap::Connection>;

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Folder {
    /// The name on the server (modified UTF-7) — used for every command
    pub id: String,
    /// The name to show
    pub name: String,
    pub delimiter: String,
    /// inbox, sent, drafts, trash, junk, archive, all, flagged or ""
    pub role: String,
    pub unseen: u32,
    pub total: u32,
    pub selectable: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Page {
    pub messages: Vec<Summary>,
    /// Messages in the folder
    pub total: u32,
    /// The sequence number of the oldest message on this page (1 = there are no older ones)
    pub first: u32,
    pub uid_validity: u32,
}

pub fn error(e: imap::Error) -> String {
    use imap::Error as E;
    match e {
        E::No(r) => format!("NO|{}", r.information),
        E::Bad(r) => format!("BAD|{}", r.information),
        E::Io(e) => format!("CONNECT|{e}"),
        E::ConnectionLost => "CONNECT|connection lost".into(),
        other => {
            let t = other.to_string();
            if t.to_lowercase().contains("tls") || t.to_lowercase().contains("certificate") { format!("TLS|{t}") } else { format!("IMAP|{t}") }
        }
    }
}

pub fn connect(a: &Account, pass: &str) -> Result<Session, String> {
    let host = a.imap_host.trim();
    let local = is_local(host);
    let mode = match a.imap_security {
        Security::Tls => ConnectionMode::Tls,
        Security::Starttls => ConnectionMode::StartTls,
        Security::Plain if local => ConnectionMode::Plaintext,
        Security::Plain => return Err("PLAIN|".into()),
    };
    let client = ClientBuilder::new(host, a.imap_port).mode(mode).tls_kind(TlsKind::Rust).danger_skip_tls_verify(local).connect().map_err(error)?;
    client.login(a.user.trim(), pass).map_err(|(e, _)| match error(e) {
        s if s.starts_with("NO|") => format!("AUTH|{}", &s[3..]),
        s => s,
    })
}

/// Quotes a string for an IMAP command.
pub fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// A list of UIDs as an IMAP set ("3,7,9").
pub fn uid_set(uids: &[u32]) -> String {
    uids.iter().map(u32::to_string).collect::<Vec<_>>().join(",")
}

fn special(attrs: &[imap_proto::NameAttribute]) -> (Option<&'static str>, bool) {
    use imap_proto::NameAttribute as N;
    let mut role = None;
    let mut selectable = true;
    for a in attrs {
        match a {
            N::NoSelect => selectable = false,
            N::Sent => role = Some("sent"),
            N::Drafts => role = Some("drafts"),
            N::Trash => role = Some("trash"),
            N::Junk => role = Some("junk"),
            N::Archive => role = Some("archive"),
            N::All => role = Some("all"),
            N::Flagged => role = Some("flagged"),
            N::Extension(x) if x.eq_ignore_ascii_case("\\NonExistent") => selectable = false,
            _ => {}
        }
    }
    (role, selectable)
}

const ORDER: [&str; 8] = ["inbox", "drafts", "sent", "archive", "junk", "trash", "flagged", "all"];

pub fn folders(s: &mut Session) -> Result<Vec<Folder>, String> {
    let names = s.list(Some(""), Some("*")).map_err(error)?;
    let mut out: Vec<Folder> = names.iter().map(|n| {
        let (sp, selectable) = special(n.attributes());
        Folder { id: n.name().to_string(), name: parse::decode_folder(n.name()), delimiter: n.delimiter().unwrap_or("/").to_string(),
            role: parse::folder_role(n.name(), sp).to_string(), selectable, ..Default::default() }
    }).collect();
    // Only one folder per role: the first one the server marked keeps it.
    let mut seen = std::collections::HashSet::new();
    for f in out.iter_mut() { if !f.role.is_empty() && !seen.insert(f.role.clone()) { f.role.clear(); } }
    for f in out.iter_mut().filter(|f| f.selectable) {
        if let Ok(m) = s.status(&f.id, "(MESSAGES UNSEEN)") { f.unseen = m.unseen.unwrap_or(0); f.total = m.exists; }
    }
    out.sort_by_key(|f| (ORDER.iter().position(|r| *r == f.role).unwrap_or(ORDER.len()), f.name.to_lowercase()));
    Ok(out)
}

const SUMMARY_ITEMS: &str = "(UID FLAGS INTERNALDATE RFC822.SIZE BODY.PEEK[HEADER])";

fn summaries(fetches: &imap::types::Fetches) -> Vec<Summary> {
    use imap::types::Flag;
    let mut out: Vec<Summary> = fetches.iter().filter_map(|f| {
        let uid = f.uid?;
        let internal = f.internal_date().map(|d| d.timestamp()).unwrap_or(0);
        let mut s = parse::summary(f.header().unwrap_or(b""), internal);
        s.uid = uid;
        s.seq = f.message;
        s.size = f.size.unwrap_or(0);
        for fl in f.flags() {
            match fl {
                Flag::Seen => s.seen = true,
                Flag::Flagged => s.flagged = true,
                Flag::Answered => s.answered = true,
                Flag::Draft => s.draft = true,
                _ => {}
            }
        }
        Some(s)
    }).collect();
    out.sort_by(|a, b| b.seq.cmp(&a.seq));
    out
}

/// The newest `limit` messages of a folder, or — with `before` (a sequence number) — the ones before it.
pub fn page(s: &mut Session, folder: &str, limit: u32, before: Option<u32>) -> Result<Page, String> {
    let mb = s.examine(folder).map_err(error)?;
    let total = mb.exists;
    let end = before.map(|b| b.saturating_sub(1)).unwrap_or(total).min(total);
    if end == 0 { return Ok(Page { total, first: 1, uid_validity: mb.uid_validity.unwrap_or(0), messages: vec![] }); }
    let first = end.saturating_sub(limit.max(1) - 1).max(1);
    let fetches = s.fetch(format!("{first}:{end}"), SUMMARY_ITEMS).map_err(error)?;
    Ok(Page { messages: summaries(&fetches), total, first, uid_validity: mb.uid_validity.unwrap_or(0) })
}

/// The whole message (RFC 822 bytes), without marking it as read.
pub fn message(s: &mut Session, folder: &str, uid: u32) -> Result<(Vec<u8>, u32), String> {
    let mb = s.examine(folder).map_err(error)?;
    let f = s.uid_fetch(uid.to_string(), "BODY.PEEK[]").map_err(error)?;
    let body = f.iter().find_map(|x| x.body().map(<[u8]>::to_vec)).ok_or_else(|| "GONE|".to_string())?;
    Ok((body, mb.uid_validity.unwrap_or(0)))
}

/// Sets or removes a flag: seen, flagged, answered.
pub fn set_flag(s: &mut Session, folder: &str, uids: &[u32], flag: &str, on: bool) -> Result<(), String> {
    let f = match flag { "seen" => "\\Seen", "flagged" => "\\Flagged", "answered" => "\\Answered", _ => return Err(format!("IMAP|unknown flag {flag}")) };
    if uids.is_empty() { return Ok(()); }
    s.select(folder).map_err(error)?;
    s.uid_store(uid_set(uids), format!("{}FLAGS.SILENT ({f})", if on { "+" } else { "-" })).map_err(error)?;
    Ok(())
}

fn can(s: &mut Session, cap: &str) -> bool {
    s.capabilities().map(|c| c.has_str(cap)).unwrap_or(false)
}

/// Moves messages into another folder (MOVE, else copy + delete).
pub fn move_to(s: &mut Session, folder: &str, uids: &[u32], target: &str) -> Result<(), String> {
    if uids.is_empty() || folder == target { return Ok(()); }
    let move_ok = can(s, "MOVE");
    let uidplus = can(s, "UIDPLUS");
    s.select(folder).map_err(error)?;
    let set = uid_set(uids);
    if move_ok { return s.uid_mv(&set, target).map_err(error); }
    s.uid_copy(&set, target).map_err(error)?;
    s.uid_store(&set, "+FLAGS.SILENT (\\Deleted)").map_err(error)?;
    if uidplus { s.uid_expunge(&set).map_err(error)?; } else { s.expunge().map_err(error)?; }
    Ok(())
}

/// Deletes for good (used in the trash, or when there is none).
pub fn delete_now(s: &mut Session, folder: &str, uids: &[u32]) -> Result<(), String> {
    if uids.is_empty() { return Ok(()); }
    let uidplus = can(s, "UIDPLUS");
    s.select(folder).map_err(error)?;
    let set = uid_set(uids);
    s.uid_store(&set, "+FLAGS.SILENT (\\Deleted)").map_err(error)?;
    if uidplus { s.uid_expunge(&set).map_err(error)?; } else { s.expunge().map_err(error)?; }
    Ok(())
}

/// The search criteria for some words: every word must be somewhere in the message (header or text).
pub fn criteria(query: &str) -> String {
    let words: Vec<&str> = query.split_whitespace().take(8).collect();
    if words.is_empty() { return "ALL".into(); }
    let ascii = query.is_ascii();
    let q = words.iter().map(|w| format!("TEXT {}", quote(w))).collect::<Vec<_>>().join(" ");
    if ascii { q } else { format!("CHARSET UTF-8 {q}") }
}

/// Messages in a folder that contain the words (the newest `limit`).
pub fn search(s: &mut Session, folder: &str, query: &str, limit: usize) -> Result<Vec<Summary>, String> {
    s.examine(folder).map_err(error)?;
    let mut uids: Vec<u32> = s.uid_search(criteria(query)).map_err(error)?.into_iter().collect();
    uids.sort_unstable_by(|a, b| b.cmp(a));
    uids.truncate(limit);
    if uids.is_empty() { return Ok(vec![]); }
    let fetches = s.uid_fetch(uid_set(&uids), SUMMARY_ITEMS).map_err(error)?;
    let mut out = summaries(&fetches);
    out.sort_by(|a, b| b.uid.cmp(&a.uid));
    Ok(out)
}

/// Puts a message into a folder (Sent: marked as read; Drafts: marked as draft).
pub fn append(s: &mut Session, folder: &str, bytes: &[u8], draft: bool) -> Result<Option<u32>, String> {
    use imap::types::Flag;
    let mut cmd = s.append(folder, bytes);
    cmd.flag(Flag::Seen);
    if draft { cmd.flag(Flag::Draft); }
    let r = cmd.finish().map_err(error)?;
    Ok(r.uids.and_then(|u| u.into_iter().next()).and_then(|r| match r {
        imap_proto::UidSetMember::Uid(u) => Some(u),
        imap_proto::UidSetMember::UidRange(r) => Some(*r.start()),
    }))
}

pub fn create_folder(s: &mut Session, name: &str) -> Result<(), String> {
    s.create(parse::encode_folder(name)).map_err(error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands() {
        assert_eq!(quote("a\"b\\c"), "\"a\\\"b\\\\c\"");
        assert_eq!(uid_set(&[3, 7, 9]), "3,7,9");
        assert_eq!(criteria("  "), "ALL");
        assert_eq!(criteria("Rechnung april"), "TEXT \"Rechnung\" TEXT \"april\"");
        assert_eq!(criteria("Grüße"), "CHARSET UTF-8 TEXT \"Grüße\"");
    }
}
