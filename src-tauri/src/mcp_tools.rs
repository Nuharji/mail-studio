//! What Mail Studio offers over MCP (mcp.rs): reading your mail and writing drafts. Nothing is ever sent from here —
//! a draft lands in the Drafts folder, where you check and send it yourself in the app.

use crate::{accounts, compose::Draft, imapc, mail, mcp, parse, store};
use serde_json::{json, Value};

pub fn run() { mcp::serve("mail-studio", env!("CARGO_PKG_VERSION"), &tools(), &|n, a| mail::with_timeout({ let (n, a) = (n.to_string(), a.clone()); move || call(&n, &a) })) }

fn tools() -> Vec<Value> {
    let acc = json!({"type": "string", "description": "Account id (from mail_accounts)"});
    let folder = json!({"type": "string", "description": "Folder id (from mail_folders), e.g. INBOX"});
    vec![
        mcp::tool("mail_accounts", "Lists the mail accounts set up in Mail Studio (id, name, address).", json!({}), &[]),
        mcp::tool("mail_folders", "Lists the folders of an account with unread and total counts.", json!({"account": acc}), &["account"]),
        mcp::tool("mail_list", "The newest messages in a folder: uid, sender, subject, date, read or not.", json!({"account": acc, "folder": folder,
            "limit": {"type": "integer", "description": "How many (default 30, at most 200)"}}), &["account", "folder"]),
        mcp::tool("mail_read", "Reads one message: headers, text (from the HTML when there is no text part) and attachment names. Does not mark it as read.",
            json!({"account": acc, "folder": folder, "uid": {"type": "integer"}}), &["account", "folder", "uid"]),
        mcp::tool("mail_search", "Finds messages in a folder that contain all the words (sender, subject or text).",
            json!({"account": acc, "folder": folder, "query": {"type": "string"}}), &["account", "folder", "query"]),
        mcp::tool("mail_draft", "Writes a draft into the account's Drafts folder. It is NOT sent: the user reviews and sends it in Mail Studio.",
            json!({"account": acc, "to": {"type": "string", "description": "Recipients, comma-separated"}, "cc": {"type": "string"},
                "subject": {"type": "string"}, "body": {"type": "string", "description": "Plain text"},
                "inReplyTo": {"type": "string", "description": "Message-ID of the message answered (optional)"}}), &["account", "subject", "body"]),
    ]
}

fn person(p: &parse::Person) -> String {
    if p.name.is_empty() { p.email.clone() } else { format!("{} <{}>", p.name, p.email) }
}

fn summary(s: &parse::Summary) -> Value {
    json!({"uid": s.uid, "from": person(&s.from), "subject": s.subject, "date": s.date, "read": s.seen, "flagged": s.flagged, "attachments": s.attach})
}

fn call(name: &str, args: &Value) -> Result<Value, String> {
    let account = || mcp::text(args, "account");
    let folder = || mcp::text(args, "folder");
    match name {
        "mail_accounts" => Ok(Value::Array(accounts::load(&store::data_dir()).iter()
            .map(|a| json!({"id": a.id, "name": a.name, "email": a.email, "label": a.label})).collect())),
        "mail_folders" => Ok(Value::Array(mail::folders(&account()?)?.iter()
            .map(|f| json!({"id": f.id, "name": f.name, "role": f.role, "unread": f.unseen, "total": f.total})).collect())),
        "mail_list" => {
            let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(30).clamp(1, 200) as u32;
            Ok(Value::Array(mail::page(&account()?, &folder()?, limit, None)?.messages.iter().map(summary).collect()))
        }
        "mail_read" => {
            let (a, f) = (account()?, folder()?);
            let uid = args.get("uid").and_then(Value::as_u64).ok_or("uid is needed")? as u32;
            let raw = mail::raw(&a, &f, uid, u32::MAX)?;   // validity unknown here: read from the server, never a stale copy
            let v = parse::view(&raw).ok_or("could not read the message")?;
            let text = if v.text.trim().is_empty() { parse::html_to_text(&v.html) } else { v.text.clone() };
            Ok(json!({"subject": v.subject, "from": v.from.iter().map(person).collect::<Vec<_>>(), "to": v.to.iter().map(person).collect::<Vec<_>>(),
                "cc": v.cc.iter().map(person).collect::<Vec<_>>(), "date": v.date, "messageId": v.message_id,
                "text": text.chars().take(60_000).collect::<String>(), "attachments": v.attachments.iter().filter(|x| !x.inline).map(|x| &x.name).collect::<Vec<_>>()}))
        }
        "mail_search" => {
            let (a, f, q) = (account()?, folder()?, mcp::text(args, "query")?);
            Ok(Value::Array(mail::session(&a, |s| imapc::search(s, &f, &q, 100))?.iter().map(summary).collect()))
        }
        "mail_draft" => {
            let d = Draft { account_id: account()?, to: mcp::opt_text(args, "to"), cc: mcp::opt_text(args, "cc"), subject: mcp::opt_text(args, "subject"),
                body: mcp::text(args, "body")?, in_reply_to: mcp::opt_text(args, "inReplyTo"), ..Default::default() };
            mail::save_draft(&d)?;
            Ok(Value::String("The draft is in the Drafts folder. Open Mail Studio to check and send it.".into()))
        }
        _ => Err(format!("unknown tool: {name}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_and_offline_answers() {
        let names: Vec<String> = tools().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect();
        assert!(names.contains(&"mail_draft".to_string()) && !names.iter().any(|n| n.contains("send")), "nothing is sent over MCP");
        let dir = std::env::temp_dir().join(format!("mail-mcp-{}", std::process::id()));
        std::env::set_var("MAIL_STUDIO_DIR", &dir);
        assert_eq!(call("mail_accounts", &json!({})).unwrap(), json!([]));
        assert_eq!(call("mail_folders", &json!({"account": "x"})).unwrap_err(), "NOACCOUNT|");
        assert!(call("mail_list", &json!({"account": "x"})).is_err(), "folder is needed");
        let _ = std::fs::remove_dir_all(dir);
    }
}
