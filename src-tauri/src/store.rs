//! What is kept on this computer: the accounts (accounts.rs) and a cache, so the app opens at once and messages that
//! were read once can be read again without a connection. All in the app's data folder:
//! cache/<account>/folders.json, cache/<account>/<folder>.json (the list), cache/<account>/<folder>/<validity>-<uid>.eml.

use crate::imapc::{Folder, Page};
use std::path::{Path, PathBuf};

/// The app's data folder (the same one Tauri uses; also found without a window, for the MCP server).
pub fn data_dir() -> PathBuf {
    #[cfg(debug_assertions)]
    if let Some(d) = std::env::var_os("MAIL_STUDIO_DIR") { return PathBuf::from(d); }
    const ID: &str = "com.studiofamily.mailstudio";
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_default();
    if cfg!(windows) {
        return std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or(home).join(ID);
    }
    if cfg!(target_os = "macos") { return home.join("Library").join("Application Support").join(ID); }
    std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local").join("share")).join(ID)
}

/// A folder name as a file name (hex of its bytes keeps it short-ish, safe and unique).
fn key(folder: &str) -> String {
    let mut s: String = folder.bytes().take(60).map(|b| format!("{b:02x}")).collect();
    if folder.len() > 60 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        folder.hash(&mut h);
        s.push_str(&format!("-{:x}", h.finish()));
    }
    s
}

fn acct_dir(dir: &Path, account: &str) -> PathBuf {
    dir.join("cache").join(account.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>())
}

fn write(path: &Path, bytes: &[u8]) {
    if let Some(p) = path.parent() { let _ = std::fs::create_dir_all(p); }
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, bytes).is_ok() { let _ = std::fs::rename(&tmp, path); }
}

pub fn save_folders(dir: &Path, account: &str, f: &[Folder]) {
    write(&acct_dir(dir, account).join("folders.json"), serde_json::to_string(f).unwrap_or_default().as_bytes());
}

pub fn folders(dir: &Path, account: &str) -> Vec<Folder> {
    std::fs::read_to_string(acct_dir(dir, account).join("folders.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn save_page(dir: &Path, account: &str, folder: &str, p: &Page) {
    write(&acct_dir(dir, account).join(format!("{}.json", key(folder))), serde_json::to_string(p).unwrap_or_default().as_bytes());
}

pub fn page(dir: &Path, account: &str, folder: &str) -> Option<Page> {
    std::fs::read_to_string(acct_dir(dir, account).join(format!("{}.json", key(folder)))).ok().and_then(|s| serde_json::from_str(&s).ok())
}

fn eml(dir: &Path, account: &str, folder: &str, validity: u32, uid: u32) -> PathBuf {
    acct_dir(dir, account).join(key(folder)).join(format!("{validity}-{uid}.eml"))
}

pub fn save_message(dir: &Path, account: &str, folder: &str, validity: u32, uid: u32, raw: &[u8]) {
    write(&eml(dir, account, folder, validity, uid), raw);
}

pub fn message(dir: &Path, account: &str, folder: &str, validity: u32, uid: u32) -> Option<Vec<u8>> {
    std::fs::read(eml(dir, account, folder, validity, uid)).ok()
}

/// Removes everything kept for an account (when it is removed from the app).
pub fn forget(dir: &Path, account: &str) {
    let d = acct_dir(dir, account);
    if d.starts_with(dir.join("cache")) && d != dir.join("cache") { let _ = std::fs::remove_dir_all(d); }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_round_trip() {
        let dir = std::env::temp_dir().join(format!("mail-store-{}", std::process::id()));
        let f = vec![Folder { id: "INBOX".into(), name: "INBOX".into(), role: "inbox".into(), unseen: 2, ..Default::default() }];
        save_folders(&dir, "a1", &f);
        assert_eq!(folders(&dir, "a1"), f);
        let p = Page { total: 5, first: 1, uid_validity: 9, messages: vec![crate::parse::Summary { uid: 4, subject: "Hi".into(), ..Default::default() }] };
        save_page(&dir, "a1", "Entw&APw-rfe", &p);
        assert_eq!(page(&dir, "a1", "Entw&APw-rfe").unwrap().messages[0].subject, "Hi");
        assert!(page(&dir, "a1", "INBOX").is_none());
        save_message(&dir, "a1", "INBOX", 9, 4, b"raw");
        assert_eq!(message(&dir, "a1", "INBOX", 9, 4).unwrap(), b"raw");
        assert!(message(&dir, "a1", "INBOX", 10, 4).is_none(), "a new UIDVALIDITY means other messages");
        assert_ne!(key("a/b"), key("a.b"));
        assert!(key(&"x".repeat(300)).len() < 150);
        forget(&dir, "a1");
        assert!(folders(&dir, "a1").is_empty());
        forget(&dir, "../..");   // never leaves the cache folder
        let _ = std::fs::remove_dir_all(dir);
    }
}
