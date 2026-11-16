//! Reading messages (no network here, so all of it is tested): the line in the message list from the header, the
//! whole message for the reader (text, HTML with its embedded pictures, attachments), and folder names, which IMAP
//! sends in "modified UTF-7" (RFC 3501 5.1.3, e.g. "Entw&APw-rfe" = "Entwürfe").

use base64::Engine;
use mail_parser::{Address, MessageParser, MimeHeaders};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Person {
    pub name: String,
    pub email: String,
}

/// One line in the message list.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Summary {
    pub uid: u32,
    pub seq: u32,
    pub from: Person,
    pub to: Vec<Person>,
    pub subject: String,
    /// Seconds since 1970 (UTC)
    pub date: i64,
    pub size: u32,
    pub seen: bool,
    pub flagged: bool,
    pub answered: bool,
    pub draft: bool,
    /// multipart/mixed: most likely has attachments
    pub attach: bool,
    pub message_id: String,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub index: usize,
    pub name: String,
    pub mime: String,
    pub size: usize,
    /// Shown inside the HTML (a picture referenced by cid:)
    pub inline: bool,
}

/// A whole message for the reader.
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub subject: String,
    pub from: Vec<Person>,
    pub to: Vec<Person>,
    pub cc: Vec<Person>,
    pub reply_to: Vec<Person>,
    pub date: i64,
    pub message_id: String,
    pub references: Vec<String>,
    pub text: String,
    /// The HTML part with cid: pictures put in as data: URLs (shown in a sandboxed frame without scripts)
    pub html: String,
    pub attachments: Vec<Attachment>,
    /// List-Unsubscribe (a mailto: or https: link), when given
    pub unsubscribe: String,
}

fn people(a: Option<&Address>) -> Vec<Person> {
    a.map(|a| a.iter().map(|x| Person { name: x.name().unwrap_or("").trim().to_string(), email: x.address().unwrap_or("").trim().to_string() })
        .filter(|p| !p.email.is_empty() || !p.name.is_empty()).collect()).unwrap_or_default()
}

/// The list line from the message header (BODY.PEEK[HEADER]); `internal` is the server's arrival time, used when
/// the Date header is missing or broken.
pub fn summary(header: &[u8], internal: i64) -> Summary {
    let Some(m) = MessageParser::default().parse_headers(header) else { return Summary { date: internal, ..Default::default() } };
    // multipart/mixed in the Content-Type line (read from the bytes: a header-only parse keeps no MIME parts)
    let lower = String::from_utf8_lossy(header).to_ascii_lowercase();
    let mixed = lower.split("\ncontent-type:").skip(1).chain(lower.strip_prefix("content-type:")).any(|v| v.trim_start().starts_with("multipart/mixed"));
    let date = m.date().map(|d| d.to_timestamp()).filter(|d| *d > 0).unwrap_or(internal);
    Summary {
        from: people(m.from()).into_iter().next().unwrap_or_default(),
        to: people(m.to()),
        subject: m.subject().unwrap_or("").trim().to_string(),
        date,
        attach: mixed,
        message_id: m.message_id().unwrap_or("").to_string(),
        ..Default::default()
    }
}

fn text_list(v: &mail_parser::HeaderValue) -> Vec<String> {
    match v {
        mail_parser::HeaderValue::Text(t) => vec![t.to_string()],
        mail_parser::HeaderValue::TextList(l) => l.iter().map(|t| t.to_string()).collect(),
        mail_parser::HeaderValue::Address(a) => a.iter().filter_map(|x| x.address().map(String::from)).collect(),
        _ => vec![],
    }
}

/// Pictures up to this size are put into the HTML; bigger ones stay attachments only.
const INLINE_MAX: usize = 8 << 20;

pub fn view(raw: &[u8]) -> Option<View> {
    let m = MessageParser::default().parse(raw)?;
    let mut html = m.body_html(0).map(|h| h.into_owned()).unwrap_or_default();
    let has_html = m.html_bodies().next().is_some_and(|p| p.is_content_type("text", "html"));
    if !has_html { html.clear(); }
    let text = m.body_text(0).map(|t| t.into_owned()).unwrap_or_default();
    let mut attachments = vec![];
    for (index, part) in m.attachments().enumerate() {
        let mime = part.content_type().map(|c| format!("{}/{}", c.ctype(), c.subtype().unwrap_or("octet-stream"))).unwrap_or_else(|| "application/octet-stream".into()).to_ascii_lowercase();
        let cid = part.content_id().map(|c| c.trim_matches(|ch| ch == '<' || ch == '>').to_string()).unwrap_or_default();
        let bytes = part.contents();
        let mut inline = false;
        if !cid.is_empty() && mime.starts_with("image/") && bytes.len() <= INLINE_MAX && !html.is_empty() {
            let needle = format!("cid:{cid}");
            if html.contains(&needle) {
                let data = format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes));
                html = html.replace(&needle, &data);
                inline = true;
            }
        }
        let name = part.attachment_name().map(String::from).unwrap_or_else(|| {
            if part.is_message() { "message.eml".into() } else { format!("attachment-{}.{}", index + 1, mime.rsplit('/').next().unwrap_or("bin")) }
        });
        attachments.push(Attachment { index, name, mime, size: bytes.len(), inline });
    }
    let unsubscribe = m.header("List-Unsubscribe").map(text_list).unwrap_or_default().into_iter()
        .flat_map(|s| s.split(',').map(String::from).collect::<Vec<_>>())
        .map(|s| s.trim().trim_matches(|c| c == '<' || c == '>').to_string())
        .find(|s| s.starts_with("https://") || s.starts_with("mailto:")).unwrap_or_default();
    Some(View {
        subject: m.subject().unwrap_or("").trim().to_string(),
        from: people(m.from()),
        to: people(m.to()),
        cc: people(m.cc()),
        reply_to: people(m.reply_to()),
        date: m.date().map(|d| d.to_timestamp()).unwrap_or(0),
        message_id: m.message_id().unwrap_or("").to_string(),
        references: text_list(m.references()),
        text,
        html,
        attachments,
        unsubscribe,
    })
}

/// The bytes and name of attachment `index` (as numbered in `view`).
pub fn attachment(raw: &[u8], index: usize) -> Option<(String, Vec<u8>)> {
    let m = MessageParser::default().parse(raw)?;
    let part = m.attachments().nth(index)?;
    let name = part.attachment_name().map(String::from).unwrap_or_else(|| format!("attachment-{}", index + 1));
    Some((name, part.contents().to_vec()))
}

/// Attachments that would run a program when opened: they can only be saved, never opened from the app.
pub fn risky(name: &str) -> bool {
    const RISKY: [&str; 24] = ["exe", "com", "bat", "cmd", "msi", "msix", "scr", "pif", "vbs", "vbe", "js", "jse", "wsf", "ps1", "jar",
        "sh", "command", "app", "pkg", "dmg", "lnk", "reg", "hta", "cpl"];
    let ext = name.rsplit_once('.').map(|(_, e)| e.trim().to_ascii_lowercase()).unwrap_or_default();
    ext.is_empty() || RISKY.contains(&ext.as_str())
}

/// A file name that is safe on every system (attachments come from strangers).
pub fn safe_name(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_control() || "/\\:*?\"<>|".contains(c) { '_' } else { c }).collect();
    let s = s.trim().trim_matches('.').to_string();
    if s.is_empty() { "attachment".into() } else { s.chars().take(120).collect() }
}

/* ---------------- Folder names (modified UTF-7) ---------------- */

pub fn decode_folder(name: &str) -> String {
    let mut out = String::new();
    let mut rest = name;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let Some(end) = after.find('-') else { out.push_str(&rest[i..]); return out; };
        let chunk = &after[..end];
        if chunk.is_empty() {
            out.push('&');
        } else {
            let b64 = chunk.replace(',', "/");
            match base64::engine::general_purpose::STANDARD_NO_PAD.decode(b64.as_bytes()) {
                Ok(bytes) if bytes.len() % 2 == 0 => {
                    let units: Vec<u16> = bytes.chunks(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
                    out.push_str(&String::from_utf16_lossy(&units));
                }
                _ => { out.push('&'); out.push_str(chunk); out.push('-'); }
            }
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

pub fn encode_folder(name: &str) -> String {
    let mut out = String::new();
    let mut pending: Vec<u16> = vec![];
    let flush = |pending: &mut Vec<u16>, out: &mut String| {
        if pending.is_empty() { return; }
        let bytes: Vec<u8> = pending.iter().flat_map(|u| u.to_be_bytes()).collect();
        out.push('&');
        out.push_str(&base64::engine::general_purpose::STANDARD_NO_PAD.encode(bytes).replace('/', ","));
        out.push('-');
        pending.clear();
    };
    for c in name.chars() {
        if (' '..='~').contains(&c) {
            flush(&mut pending, &mut out);
            if c == '&' { out.push_str("&-"); } else { out.push(c); }
        } else {
            let mut buf = [0u16; 2];
            pending.extend_from_slice(c.encode_utf16(&mut buf));
        }
    }
    flush(&mut pending, &mut out);
    out
}

/// What a folder is for: from the server's special-use flag, else from its usual names (several languages).
pub fn folder_role(name: &str, special: Option<&str>) -> &'static str {
    if name.eq_ignore_ascii_case("INBOX") { return "inbox"; }
    if let Some(s) = special { return match s { "sent" => "sent", "drafts" => "drafts", "trash" => "trash", "junk" => "junk", "archive" => "archive", "all" => "all", "flagged" => "flagged", _ => "" }; }
    let last = decode_folder(name).rsplit(['/', '.']).next().unwrap_or("").to_lowercase();
    match last.as_str() {
        "sent" | "sent messages" | "sent mail" | "sent items" | "gesendet" | "gesendete objekte" | "gesendete elemente" | "gesendete nachrichten" => "sent",
        "drafts" | "draft" | "entwürfe" | "entwurf" => "drafts",
        "trash" | "deleted messages" | "deleted items" | "bin" | "papierkorb" | "gelöschte elemente" | "gelöscht" | "gelöschte objekte" => "trash",
        "junk" | "spam" | "junk e-mail" | "junk-e-mail" | "werbung" => "junk",
        "archive" | "archiv" | "archives" => "archive",
        _ => "",
    }
}

/* ---------------- Writing ---------------- */

/// The text of an HTML-only message, for quoting in a reply (tags removed, paragraphs kept).
pub fn html_to_text(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    let mut tag = String::new();
    let mut skip = false;   // inside <style> / <script> / <head>
    for c in html.chars() {
        if in_tag {
            if c == '>' {
                in_tag = false;
                let t = tag.trim().to_ascii_lowercase();
                let name: String = t.trim_start_matches('/').chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
                if matches!(name.as_str(), "style" | "script" | "head" | "title") { skip = !t.starts_with('/'); }
                if matches!(name.as_str(), "br" | "p" | "div" | "tr" | "li" | "h1" | "h2" | "h3" | "h4" | "blockquote" | "table") && !out.ends_with('\n') { out.push('\n'); }
                tag.clear();
            } else { tag.push(c); }
        } else if c == '<' { in_tag = true; } else if !skip { out.push(c); }
    }
    let out = out.replace("&nbsp;", " ").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&amp;", "&");
    let lines: Vec<&str> = out.lines().map(str::trim_end).collect();
    let mut res = String::new();
    let mut blank = 0;
    for l in lines {
        if l.trim().is_empty() { blank += 1; if blank > 1 { continue; } } else { blank = 0; }
        res.push_str(l);
        res.push('\n');
    }
    res.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIL: &str = "From: =?utf-8?q?J=C3=BCrgen?= <juergen@example.org>\r\nTo: Ana <ana@example.org>, bob@example.org\r\nCc: Carl <c@example.org>\r\n\
Subject: =?utf-8?b?R3LDvMOfZSBhdXMgQmVybGlu?=\r\nDate: Tue, 10 Nov 2026 09:30:00 +0100\r\nMessage-ID: <abc@example.org>\r\n\
References: <r1@x> <r2@x>\r\nList-Unsubscribe: <https://example.org/u?1>, <mailto:u@example.org>\r\nMIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"B1\"\r\n\r\n--B1\r\nContent-Type: multipart/related; boundary=\"B2\"\r\n\r\n--B2\r\n\
Content-Type: text/html; charset=utf-8\r\n\r\n<p>Hallo <b>Ana</b></p><img src=\"cid:pic1@x\"><style>p{}</style>\r\n--B2\r\nContent-Type: image/png\r\n\
Content-ID: <pic1@x>\r\nContent-Transfer-Encoding: base64\r\n\r\niVBORw0KGgo=\r\n--B2--\r\n--B1\r\nContent-Type: application/pdf; name=\"Plan.pdf\"\r\n\
Content-Disposition: attachment; filename=\"Plan.pdf\"\r\nContent-Transfer-Encoding: base64\r\n\r\nJVBERi0xLjQ=\r\n--B1--\r\n";

    #[test]
    fn list_line_from_header() {
        let head = MAIL.split("\r\n\r\n").next().unwrap();
        let s = summary(head.as_bytes(), 5);
        assert_eq!(s.from, Person { name: "Jürgen".into(), email: "juergen@example.org".into() });
        assert_eq!(s.subject, "Grüße aus Berlin");
        assert_eq!(s.to.len(), 2);
        assert_eq!(s.to[1].email, "bob@example.org");
        assert_eq!(s.date, 1794299400);
        assert!(s.attach);
        assert_eq!(s.message_id, "abc@example.org");
        assert_eq!(summary(b"Subject: x\r\n", 77).date, 77, "no Date header: the arrival time");
    }

    #[test]
    fn whole_message() {
        let v = view(MAIL.as_bytes()).unwrap();
        assert_eq!(v.subject, "Grüße aus Berlin");
        assert_eq!(v.cc[0].name, "Carl");
        assert!(v.html.contains("<b>Ana</b>"));
        assert!(v.html.contains("src=\"data:image/png;base64,iVBORw0KGgo=\""), "cid pictures are put in: {}", v.html);
        assert_eq!(v.attachments.len(), 2);
        let pdf = v.attachments.iter().find(|a| a.name == "Plan.pdf").unwrap();
        assert_eq!((pdf.mime.as_str(), pdf.inline, pdf.size), ("application/pdf", false, 8));
        assert!(v.attachments.iter().any(|a| a.inline && a.mime == "image/png"));
        assert_eq!(v.references, vec!["r1@x", "r2@x"]);
        assert_eq!(v.unsubscribe, "https://example.org/u?1");
        assert!(v.text.contains("Hallo Ana"), "text made from the HTML: {:?}", v.text);
        let (name, bytes) = attachment(MAIL.as_bytes(), pdf.index).unwrap();
        assert_eq!((name.as_str(), bytes.as_slice()), ("Plan.pdf", b"%PDF-1.4".as_slice()));

        let plain = view(b"From: a@b.de\r\nSubject: Hi\r\n\r\nNur Text\r\n").unwrap();
        assert_eq!(plain.html, "", "a text-only message has no HTML");
        assert_eq!(plain.text.trim(), "Nur Text");
    }

    #[test]
    fn folder_names() {
        assert_eq!(decode_folder("Entw&APw-rfe"), "Entwürfe");
        assert_eq!(decode_folder("Gel&APY-schte Elemente"), "Gelöschte Elemente");
        assert_eq!(decode_folder("A&-B"), "A&B");
        assert_eq!(decode_folder("&ZeVnLIqe-"), "日本語");
        for n in ["Entwürfe", "A&B", "日本語/Ordner", "Plain", "Gelöschte Elemente"] { assert_eq!(decode_folder(&encode_folder(n)), n); }
        assert_eq!(encode_folder("Entwürfe"), "Entw&APw-rfe");
        assert_eq!(folder_role("INBOX", None), "inbox");
        assert_eq!(folder_role("Entw&APw-rfe", None), "drafts");
        assert_eq!(folder_role("[Gmail]/Sent Mail", None), "sent");
        assert_eq!(folder_role("INBOX.Papierkorb", None), "trash");
        assert_eq!(folder_role("Whatever", Some("junk")), "junk");
        assert_eq!(folder_role("Rechnungen", None), "");
    }

    #[test]
    fn helpers() {
        assert!(risky("rechnung.pdf.exe") && risky("x.JS") && risky("ohne") && !risky("Plan.pdf") && !risky("foto.JPG"));
        assert_eq!(safe_name("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(safe_name("a:b?.txt"), "a_b_.txt");
        assert_eq!(html_to_text("<html><head><style>x{}</style></head><body><p>Hallo&nbsp;du</p><p>Zeile&amp;2<br>3</p></body></html>"), "Hallo du\nZeile&2\n3");
    }
}
