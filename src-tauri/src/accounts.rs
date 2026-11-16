//! Mail accounts: where the servers are and who you are. Kept in accounts.json in the app's data folder — without
//! passwords: those go into the system's keychain (macOS Keychain, Windows Credential Manager, Secret Service on Linux).
//! For the common providers the servers are filled in from the e-mail address (`preset`).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const KEYCHAIN_SERVICE: &str = "Mail Studio";

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Security {
    /// TLS from the start (IMAP 993, SMTP 465)
    #[default]
    Tls,
    /// Plain connection that switches to TLS (IMAP 143, SMTP 587)
    Starttls,
    /// No encryption — only allowed for servers on this computer (e.g. Proton Mail Bridge)
    Plain,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    /// The sender name ("Jamie B.")
    pub name: String,
    pub email: String,
    /// The login name (most providers: the e-mail address)
    pub user: String,
    pub imap_host: String,
    pub imap_port: u16,
    pub imap_security: Security,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_security: Security,
    /// Added below new messages ("-- \n" is put in front)
    pub signature: String,
    /// The server puts sent mail into "Sent" itself (Gmail, Outlook, Proton); else the app does it.
    pub server_saves_sent: bool,
    /// Short label in the sidebar ("Privat", "Schule")
    pub label: String,
}

/// Server settings for a provider.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Preset {
    pub provider: &'static str,
    pub imap_host: String,
    pub imap_port: u16,
    pub imap_security: Security,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_security: Security,
    pub server_saves_sent: bool,
    /// What the user has to know: "appPassword" (needs an app-specific password), "enableImap" (switch IMAP on in the
    /// web mail first), "bridge" (Proton Mail Bridge must run), "oauth" (password sign-in not offered by the provider),
    /// "guess" (unknown provider: the usual names were guessed).
    pub note: &'static str,
}

fn p(provider: &'static str, imap: &str, smtp: &str, smtp_port: u16, saves: bool, note: &'static str) -> Preset {
    let smtp_security = if smtp_port == 465 { Security::Tls } else { Security::Starttls };
    Preset { provider, imap_host: imap.into(), imap_port: 993, imap_security: Security::Tls, smtp_host: smtp.into(), smtp_port,
        smtp_security, server_saves_sent: saves, note }
}

/// The servers for an e-mail address: known providers by their domain, else a guess (imap.<domain> / smtp.<domain>).
pub fn preset(email: &str) -> Option<Preset> {
    let domain = email.rsplit_once('@')?.1.trim().to_ascii_lowercase();
    if domain.is_empty() || !domain.contains('.') { return None; }
    let d = domain.as_str();
    Some(match d {
        "icloud.com" | "me.com" | "mac.com" => p("iCloud", "imap.mail.me.com", "smtp.mail.me.com", 587, false, "appPassword"),
        "gmail.com" | "googlemail.com" => p("Gmail", "imap.gmail.com", "smtp.gmail.com", 465, true, "appPassword"),
        "outlook.com" | "outlook.de" | "hotmail.com" | "hotmail.de" | "live.com" | "live.de" | "msn.com" =>
            p("Outlook.com", "outlook.office365.com", "smtp-mail.outlook.com", 587, true, "oauth"),
        "yahoo.com" | "yahoo.de" | "ymail.com" => p("Yahoo", "imap.mail.yahoo.com", "smtp.mail.yahoo.com", 465, false, "appPassword"),
        "gmx.de" | "gmx.net" | "gmx.at" | "gmx.ch" | "gmx.com" => p("GMX", "imap.gmx.net", "mail.gmx.net", 587, false, "enableImap"),
        "web.de" => p("WEB.DE", "imap.web.de", "smtp.web.de", 587, false, "enableImap"),
        "t-online.de" | "magenta.de" => p("Telekom", "secureimap.t-online.de", "securesmtp.t-online.de", 465, false, "appPassword"),
        "posteo.de" | "posteo.net" => p("Posteo", "posteo.de", "posteo.de", 587, false, ""),
        "mailbox.org" => p("mailbox.org", "imap.mailbox.org", "smtp.mailbox.org", 465, false, ""),
        "aol.com" | "aol.de" => p("AOL", "imap.aol.com", "smtp.aol.com", 465, false, "appPassword"),
        "freenet.de" => p("freenet", "mx.freenet.de", "mx.freenet.de", 587, false, "enableImap"),
        "proton.me" | "protonmail.com" | "protonmail.ch" | "pm.me" => Preset {
            provider: "Proton Mail", imap_host: "127.0.0.1".into(), imap_port: 1143, imap_security: Security::Starttls,
            smtp_host: "127.0.0.1".into(), smtp_port: 1025, smtp_security: Security::Starttls, server_saves_sent: true, note: "bridge" },
        _ => p("", &format!("imap.{d}"), &format!("smtp.{d}"), 587, false, "guess"),
    })
}

/// A server on this computer (Proton Mail Bridge, test servers): its self-made certificate is accepted, plain
/// connections are allowed. Nothing else gets that.
pub fn is_local(host: &str) -> bool {
    matches!(host.trim().to_ascii_lowercase().as_str(), "127.0.0.1" | "localhost" | "::1" | "[::1]")
}

/// What is wrong with an account before connecting (None = fine).
pub fn check(a: &Account) -> Option<String> {
    let email_ok = a.email.split_once('@').is_some_and(|(l, d)| !l.is_empty() && d.contains('.') && !a.email.contains(char::is_whitespace));
    if !email_ok { return Some("email".into()); }
    if a.imap_host.trim().is_empty() || a.imap_port == 0 { return Some("imap".into()); }
    if a.smtp_host.trim().is_empty() || a.smtp_port == 0 { return Some("smtp".into()); }
    if a.user.trim().is_empty() { return Some("user".into()); }
    if a.imap_security == Security::Plain && !is_local(&a.imap_host) { return Some("plainImap".into()); }
    if a.smtp_security == Security::Plain && !is_local(&a.smtp_host) { return Some("plainSmtp".into()); }
    None
}

/* ---------------- The file ---------------- */

pub fn file(dir: &Path) -> PathBuf { dir.join("accounts.json") }

pub fn load(dir: &Path) -> Vec<Account> {
    std::fs::read_to_string(file(dir)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn store(dir: &Path, list: &[Account]) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let path = file(dir);
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(list).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

pub fn new_id() -> String {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    format!("a{:x}{:x}", t.as_secs(), t.subsec_nanos() & 0xffff)
}

/* ---------------- Passwords (keychain) ---------------- */

/// Debug builds: tests use a file instead of the real keychain (MAIL_STUDIO_SECRETS = path of a JSON file).
#[cfg(debug_assertions)]
fn test_secrets() -> Option<PathBuf> { std::env::var_os("MAIL_STUDIO_SECRETS").map(PathBuf::from) }
#[cfg(not(debug_assertions))]
fn test_secrets() -> Option<PathBuf> { None }

pub fn set_password(id: &str, pass: &str) -> Result<(), String> {
    if let Some(f) = test_secrets() {
        let mut m: serde_json::Map<String, serde_json::Value> = std::fs::read_to_string(&f).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        m.insert(id.into(), pass.into());
        return std::fs::write(f, serde_json::to_string(&m).unwrap_or_default()).map_err(|e| e.to_string());
    }
    keyring::Entry::new(KEYCHAIN_SERVICE, id).and_then(|e| e.set_password(pass)).map_err(|e| format!("KEYCHAIN|{e}"))
}

pub fn password(id: &str) -> Result<String, String> {
    if let Some(f) = test_secrets() {
        let m: serde_json::Value = std::fs::read_to_string(&f).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        return m[id].as_str().map(String::from).ok_or_else(|| "NOPASS|".into());
    }
    match keyring::Entry::new(KEYCHAIN_SERVICE, id).and_then(|e| e.get_password()) {
        Ok(p) => Ok(p),
        Err(keyring::Error::NoEntry) => Err("NOPASS|".into()),
        Err(e) => Err(format!("KEYCHAIN|{e}")),
    }
}

pub fn forget_password(id: &str) {
    if test_secrets().is_some() { return; }
    if let Ok(e) = keyring::Entry::new(KEYCHAIN_SERVICE, id) { let _ = e.delete_credential(); }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_by_domain() {
        let i = preset("jamie@iCloud.com").unwrap();
        assert_eq!((i.imap_host.as_str(), i.imap_port, i.smtp_port, i.smtp_security), ("imap.mail.me.com", 993, 587, Security::Starttls));
        assert_eq!(i.note, "appPassword");
        let g = preset("a@gmail.com").unwrap();
        assert_eq!((g.smtp_port, g.smtp_security, g.server_saves_sent), (465, Security::Tls, true));
        let pr = preset("x@proton.me").unwrap();
        assert!(is_local(&pr.imap_host) && pr.note == "bridge");
        let u = preset("x@example.org").unwrap();
        assert_eq!((u.imap_host.as_str(), u.smtp_host.as_str(), u.note), ("imap.example.org", "smtp.example.org", "guess"));
        assert!(preset("nope").is_none() && preset("a@localhost").is_none());
    }

    #[test]
    fn checks_and_file() {
        let mut a = Account { id: "a1".into(), email: "a@b.de".into(), user: "a@b.de".into(), imap_host: "imap.b.de".into(), imap_port: 993,
            smtp_host: "smtp.b.de".into(), smtp_port: 587, smtp_security: Security::Starttls, ..Default::default() };
        assert_eq!(check(&a), None);
        a.imap_security = Security::Plain;
        assert_eq!(check(&a).as_deref(), Some("plainImap"), "no plain connections to other computers");
        a.imap_host = "127.0.0.1".into();
        assert_eq!(check(&a), None);
        a.email = "kein at".into();
        assert_eq!(check(&a).as_deref(), Some("email"));

        let dir = std::env::temp_dir().join(format!("mail-acc-{}", std::process::id()));
        store(&dir, std::slice::from_ref(&a)).unwrap();
        assert_eq!(load(&dir), vec![a]);
        let text = std::fs::read_to_string(file(&dir)).unwrap();
        assert!(!text.to_lowercase().contains("pass"), "no passwords in the file");
        let _ = std::fs::remove_dir_all(dir);
    }
}
