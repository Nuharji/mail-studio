//! Writing: a message from what was typed in the compose window (lettre), sending it over SMTP, and the same
//! message as bytes for the "Sent" and "Drafts" folders.

use crate::accounts::{is_local, Account, Security};
use lettre::message::header::{ContentType, HeaderName, HeaderValue};
use lettre::message::{Attachment, Mailbox, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::client::{Tls, TlsParameters};
use lettre::{Message, SmtpTransport, Transport};
use serde::Deserialize;
use std::path::Path;
use std::time::Duration;

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Draft {
    pub account_id: String,
    pub to: String,
    pub cc: String,
    pub bcc: String,
    pub subject: String,
    pub body: String,
    /// Full paths of files to attach
    pub attachments: Vec<String>,
    /// Message-ID of the message answered (without <>)
    pub in_reply_to: String,
    pub references: Vec<String>,
    /// The draft this one replaces in the Drafts folder (removed after sending / saving again)
    pub draft_uid: Option<u32>,
    /// The message answered (gets the "answered" flag after sending)
    pub answer_folder: String,
    pub answer_uid: Option<u32>,
}

/// Splits "Ana <a@x.de>, \"Doe, John\" <j@x.de>; b@x.de" into single addresses (commas inside quotes stay).
pub fn split_addresses(s: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let (mut quoted, mut angle) = (false, false);
    for c in s.chars() {
        match c {
            '"' => { quoted = !quoted; cur.push(c); }
            '<' if !quoted => { angle = true; cur.push(c); }
            '>' if !quoted => { angle = false; cur.push(c); }
            ',' | ';' | '\n' if !quoted && !angle => { if !cur.trim().is_empty() { out.push(cur.trim().to_string()); } cur.clear(); }
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() { out.push(cur.trim().to_string()); }
    out
}

fn mailboxes(s: &str) -> Result<Vec<Mailbox>, String> {
    split_addresses(s).into_iter().map(|a| a.parse::<Mailbox>().map_err(|_| format!("ADDR|{a}"))).collect()
}

fn angle(id: &str) -> String {
    let id = id.trim().trim_matches(|c| c == '<' || c == '>');
    format!("<{id}>")
}

fn mime_of(path: &Path) -> ContentType {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let m = match ext.as_str() {
        "pdf" => "application/pdf", "png" => "image/png", "jpg" | "jpeg" => "image/jpeg", "gif" => "image/gif", "webp" => "image/webp",
        "heic" => "image/heic", "svg" => "image/svg+xml", "txt" | "md" => "text/plain; charset=utf-8", "csv" => "text/csv; charset=utf-8",
        "html" | "htm" => "text/html; charset=utf-8", "zip" => "application/zip", "json" => "application/json",
        "doc" => "application/msword", "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel", "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint", "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "mp3" => "audio/mpeg", "mp4" => "video/mp4", "mov" => "video/quicktime", "ics" => "text/calendar; charset=utf-8",
        _ => "application/octet-stream",
    };
    ContentType::parse(m).unwrap_or(ContentType::TEXT_PLAIN)
}

/// The finished message. `for_sending`: there must be a recipient (a draft may have none).
pub fn build(a: &Account, d: &Draft, for_sending: bool) -> Result<Message, String> {
    let from: Mailbox = if a.name.trim().is_empty() { a.email.parse() } else { format!("\"{}\" <{}>", a.name.replace('"', "'"), a.email).parse() }
        .map_err(|_| format!("ADDR|{}", a.email))?;
    let (to, cc, bcc) = (mailboxes(&d.to)?, mailboxes(&d.cc)?, mailboxes(&d.bcc)?);
    if for_sending && to.is_empty() && cc.is_empty() && bcc.is_empty() { return Err("NORCPT|".into()); }
    let mut b = Message::builder().from(from).subject(d.subject.trim()).date_now()
        .user_agent(format!("Mail Studio {}", env!("CARGO_PKG_VERSION")));
    let none = to.is_empty() && cc.is_empty() && bcc.is_empty();
    for m in to { b = b.to(m); }
    for m in cc { b = b.cc(m); }
    for m in bcc { b = b.bcc(m); }
    if none {
        // A draft without recipients: lettre wants an envelope, the message itself gets no To line.
        let me: lettre::Address = a.email.parse().map_err(|_| format!("ADDR|{}", a.email))?;
        b = b.envelope(lettre::address::Envelope::new(Some(me.clone()), vec![me]).map_err(|e| format!("BUILD|{e}"))?);
    }
    if !d.in_reply_to.trim().is_empty() {
        b = b.in_reply_to(angle(&d.in_reply_to));
        let mut refs: Vec<String> = d.references.iter().filter(|r| !r.trim().is_empty()).map(|r| angle(r)).collect();
        let parent = angle(&d.in_reply_to);
        if !refs.contains(&parent) { refs.push(parent); }
        b = b.references(refs.join(" "));
    }
    let text = SinglePart::builder().header(ContentType::TEXT_PLAIN).body(d.body.replace("\r\n", "\n"));
    let msg = if d.attachments.is_empty() {
        b.singlepart(text)
    } else {
        let mut mp = MultiPart::mixed().singlepart(text);
        for p in &d.attachments {
            let path = Path::new(p);
            let bytes = std::fs::read(path).map_err(|e| format!("FILE|{}: {e}", path.display()))?;
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
            mp = mp.singlepart(Attachment::new(name).body(bytes, mime_of(path)));
        }
        b.multipart(mp)
    };
    msg.map_err(|e| format!("BUILD|{e}"))
}

/// The message as it goes into the Drafts folder (with the Bcc line, which only the sender sees).
pub fn draft_bytes(a: &Account, d: &Draft) -> Result<Vec<u8>, String> {
    let mut m = build(a, d, false)?;
    // keep_bcc is a builder option; for drafts the Bcc line goes in as a plain header.
    if !d.bcc.trim().is_empty() {
        m.headers_mut().insert_raw(HeaderValue::new(HeaderName::new_from_ascii_str("Bcc"), d.bcc.trim().to_string()));
    }
    Ok(m.formatted())
}

fn transport(a: &Account, pass: &str) -> Result<SmtpTransport, String> {
    let host = a.smtp_host.trim();
    let local = is_local(host);
    let tls = || TlsParameters::builder(host.to_string()).dangerous_accept_invalid_certs(local).build().map_err(|e| format!("TLS|{e}"));
    let tls = match a.smtp_security {
        Security::Tls => Tls::Wrapper(tls()?),
        Security::Starttls => Tls::Required(tls()?),
        Security::Plain if local => Tls::None,
        Security::Plain => return Err("PLAIN|".into()),
    };
    Ok(SmtpTransport::builder_dangerous(host).port(a.smtp_port).tls(tls).timeout(Some(Duration::from_secs(60)))
        .credentials(Credentials::new(a.user.clone(), pass.to_string())).build())
}

fn smtp_error(e: lettre::transport::smtp::Error) -> String {
    let text = e.to_string();
    let lower = text.to_lowercase();
    if lower.contains("auth") || lower.contains("535") || lower.contains("credentials") { format!("AUTH|{text}") }
    else if e.is_permanent() { format!("REJECT|{text}") }
    else { format!("SMTP|{text}") }
}

/// Sends the message; gives back its bytes (for the Sent folder).
pub fn send(a: &Account, pass: &str, d: &Draft) -> Result<Vec<u8>, String> {
    let m = build(a, d, true)?;
    transport(a, pass)?.send(&m).map_err(smtp_error)?;
    Ok(m.formatted())
}

/// Tries the SMTP sign-in without sending anything.
pub fn test_login(a: &Account, pass: &str) -> Result<(), String> {
    match transport(a, pass)?.test_connection() {
        Ok(true) => Ok(()),
        Ok(false) => Err("SMTP|no answer".into()),
        Err(e) => Err(smtp_error(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acc() -> Account {
        Account { id: "a".into(), name: "Jamie \"J\" B".into(), email: "jamie@example.org".into(), user: "jamie@example.org".into(), ..Default::default() }
    }

    #[test]
    fn addresses() {
        assert_eq!(split_addresses("Ana <a@x.de>, \"Doe, John\" <j@x.de>; b@x.de\n"), vec!["Ana <a@x.de>", "\"Doe, John\" <j@x.de>", "b@x.de"]);
        assert!(split_addresses(" , ;").is_empty());
        assert!(mailboxes("kein-at").is_err());
    }

    #[test]
    fn builds_messages() {
        let d = Draft { to: "Ana <ana@example.org>".into(), bcc: "geheim@example.org".into(), subject: "Grüße".into(),
            body: "Hallo Ana,\r\nbis bald".into(), in_reply_to: "<p1@x>".into(), references: vec!["r0@x".into()], ..Default::default() };
        let raw = String::from_utf8(build(&acc(), &d, true).unwrap().formatted()).unwrap();
        assert!(raw.contains("To: Ana <ana@example.org>"), "{raw}");
        assert!(raw.contains("In-Reply-To: <p1@x>"));
        assert!(raw.contains("References: <r0@x> <p1@x>"));
        assert!(!raw.contains("geheim@"), "Bcc is not in the sent message");
        assert!(raw.contains("Subject: =?utf-8?"), "non-ASCII subject is encoded");
        let parsed = crate::parse::view(raw.as_bytes()).unwrap();
        assert_eq!(parsed.subject, "Grüße");
        assert_eq!(parsed.text.replace("\r\n", "\n").trim(), "Hallo Ana,\nbis bald");
        assert_eq!(parsed.from[0].email, "jamie@example.org");

        let draft = String::from_utf8(draft_bytes(&acc(), &d).unwrap()).unwrap();
        assert!(draft.contains("Bcc: geheim@example.org"), "the draft keeps Bcc");
        assert_eq!(build(&acc(), &Draft::default(), true).unwrap_err(), "NORCPT|");
        assert!(build(&acc(), &Draft::default(), false).is_ok(), "a draft may have no recipient");

        let dir = std::env::temp_dir().join(format!("mail-compose-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("Plan.pdf");
        std::fs::write(&f, b"%PDF-1.4").unwrap();
        let with = Draft { to: "a@x.de".into(), attachments: vec![f.to_string_lossy().into()], ..Default::default() };
        let v = crate::parse::view(&build(&acc(), &with, true).unwrap().formatted()).unwrap();
        assert_eq!(v.attachments.len(), 1);
        assert_eq!((v.attachments[0].name.as_str(), v.attachments[0].mime.as_str()), ("Plan.pdf", "application/pdf"));
        let _ = std::fs::remove_dir_all(dir);
        let missing = Draft { to: "a@x.de".into(), attachments: vec!["/nope/x.pdf".into()], ..Default::default() };
        assert!(build(&acc(), &missing, true).unwrap_err().starts_with("FILE|"));
    }
}
