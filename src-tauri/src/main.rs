// Hide the console window in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod accounts;
mod appicon;
mod compose;
mod imapc;
mod mail;
mod mcp;
mod mcp_tools;
mod parse;
mod shared;
mod store;

use accounts::Account;
use compose::Draft;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Emitter};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tauri_plugin_opener::OpenerExt;

/// Runs blocking mail work off the UI thread.
async fn work<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || mail::with_timeout(f)).await.map_err(|e| e.to_string())?
}

/// The app's texts in English and German (i18n/*.json, built into the program).
#[tauri::command]
fn texts() -> serde_json::Value {
    let parse = |s: &str| serde_json::from_str::<serde_json::Value>(s).unwrap_or_default();
    serde_json::json!({"en": parse(include_str!("../i18n/en.json")), "de": parse(include_str!("../i18n/de.json"))})
}

/* ---------------- Accounts ---------------- */

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AccountInfo {
    #[serde(flatten)]
    account: Account,
    has_password: bool,
}

#[tauri::command]
async fn accounts_list() -> Result<Vec<AccountInfo>, String> {
    work(|| Ok(accounts::load(&store::data_dir()).into_iter()
        .map(|a| { let has_password = accounts::password(&a.id).is_ok(); AccountInfo { account: a, has_password } }).collect())).await
}

#[tauri::command]
fn account_preset(email: String) -> Option<accounts::Preset> { accounts::preset(&email) }

/// Checks the sign-in at both servers (with the typed password, or the stored one).
#[tauri::command]
async fn account_test(account: Account, password: Option<String>) -> Result<(), String> {
    work(move || {
        if let Some(e) = accounts::check(&account) { return Err(format!("FIELD|{e}")); }
        let pass = match password.filter(|p| !p.is_empty()) { Some(p) => p, None => accounts::password(&account.id)? };
        let mut s = imapc::connect(&account, &pass)?;
        let _ = s.logout();
        compose::test_login(&account, &pass).map_err(|e| format!("SMTP-{e}"))
    }).await
}

#[tauri::command]
async fn account_save(account: Account, password: Option<String>) -> Result<Account, String> {
    work(move || {
        let mut a = account;
        if let Some(e) = accounts::check(&a) { return Err(format!("FIELD|{e}")); }
        let dir = store::data_dir();
        let mut list = accounts::load(&dir);
        if a.id.is_empty() { a.id = accounts::new_id(); }
        if let Some(p) = password.filter(|p| !p.is_empty()) { accounts::set_password(&a.id, &p)?; }
        match list.iter_mut().find(|x| x.id == a.id) { Some(x) => *x = a.clone(), None => list.push(a.clone()) }
        accounts::store(&dir, &list)?;
        mail::drop_session(&a.id);
        Ok(a)
    }).await
}

#[tauri::command]
async fn account_remove(id: String) -> Result<(), String> {
    work(move || {
        let dir = store::data_dir();
        let list: Vec<Account> = accounts::load(&dir).into_iter().filter(|a| a.id != id).collect();
        accounts::store(&dir, &list)?;
        accounts::forget_password(&id);
        store::forget(&dir, &id);
        mail::drop_session(&id);
        Ok(())
    }).await
}

#[tauri::command]
async fn accounts_reorder(ids: Vec<String>) -> Result<(), String> {
    work(move || {
        let dir = store::data_dir();
        let mut list = accounts::load(&dir);
        list.sort_by_key(|a| ids.iter().position(|i| *i == a.id).unwrap_or(usize::MAX));
        accounts::store(&dir, &list)
    }).await
}

/* ---------------- Folders and messages ---------------- */

#[tauri::command]
async fn folders(account_id: String, cached: Option<bool>) -> Result<Vec<imapc::Folder>, String> {
    if cached.unwrap_or(false) { return Ok(store::folders(&store::data_dir(), &account_id)); }
    work(move || mail::folders(&account_id)).await
}

#[tauri::command]
async fn messages(account_id: String, folder: String, limit: Option<u32>, before: Option<u32>, cached: Option<bool>) -> Result<Option<imapc::Page>, String> {
    if cached.unwrap_or(false) { return Ok(store::page(&store::data_dir(), &account_id, &folder)); }
    work(move || mail::page(&account_id, &folder, limit.unwrap_or(100), before).map(Some)).await
}

#[tauri::command]
async fn message_read(account_id: String, folder: String, uid: u32, validity: u32) -> Result<parse::View, String> {
    work(move || {
        let raw = mail::raw(&account_id, &folder, uid, validity)?;
        parse::view(&raw).ok_or_else(|| "PARSE|".into())
    }).await
}

#[tauri::command]
async fn message_source(account_id: String, folder: String, uid: u32, validity: u32) -> Result<String, String> {
    work(move || Ok(String::from_utf8_lossy(&mail::raw(&account_id, &folder, uid, validity)?).into_owned())).await
}

#[tauri::command]
async fn set_flag(account_id: String, folder: String, uids: Vec<u32>, flag: String, on: bool) -> Result<(), String> {
    work(move || mail::session(&account_id, |s| imapc::set_flag(s, &folder, &uids, &flag, on))).await
}

#[tauri::command]
async fn move_messages(account_id: String, folder: String, uids: Vec<u32>, target: String) -> Result<(), String> {
    work(move || mail::session(&account_id, |s| imapc::move_to(s, &folder, &uids, &target))).await
}

/// "archive", "trash" or "junk": moves to the folder with that role (Archive is made when missing; delete without
/// trash, or in the trash, deletes for good). Gives back the folder the messages went to ("" = deleted).
#[tauri::command]
async fn move_to_role(account_id: String, folder: String, uids: Vec<u32>, role: String) -> Result<String, String> {
    work(move || mail::move_to_role(&account_id, &folder, &uids, &role)).await
}

#[tauri::command]
async fn search(account_id: String, folder: String, query: String) -> Result<Vec<parse::Summary>, String> {
    work(move || mail::session(&account_id, |s| imapc::search(s, &folder, &query, 300))).await
}

#[tauri::command]
async fn create_folder(account_id: String, name: String) -> Result<(), String> {
    work(move || mail::session(&account_id, |s| imapc::create_folder(s, &name))).await
}

/* ---------------- Writing ---------------- */

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Sent {
    /// The copy in the Sent folder: "server" (the provider does it), "saved", or "failed: …"
    copy: String,
}

#[tauri::command]
async fn send(draft: Draft) -> Result<Sent, String> {
    work(move || mail::send(&draft).map(|copy| Sent { copy })).await
}

#[tauri::command]
async fn save_draft(draft: Draft) -> Result<Option<u32>, String> {
    work(move || mail::save_draft(&draft)).await
}

#[derive(Serialize)]
struct Picked {
    path: String,
    name: String,
    size: u64,
}

#[tauri::command]
async fn pick_attachments(app: AppHandle) -> Vec<Picked> {
    let files = shared::file_dialog(&app).set_title("Attach files").blocking_pick_files().unwrap_or_default();
    files.into_iter().filter_map(|f| f.into_path().ok()).map(|p| Picked {
        size: std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0),
        name: p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        path: p.to_string_lossy().into_owned(),
    }).collect()
}

/// Files dropped onto the compose window (paths from the drag-and-drop event).
#[tauri::command]
fn file_infos(paths: Vec<String>) -> Vec<Picked> {
    paths.into_iter().map(PathBuf::from).filter(|p| p.is_file()).map(|p| Picked {
        size: std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0),
        name: p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        path: p.to_string_lossy().into_owned(),
    }).collect()
}

/* ---------------- Attachments ---------------- */

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AttRef {
    account_id: String,
    folder: String,
    uid: u32,
    validity: u32,
    index: usize,
}

fn attachment_of(r: &AttRef) -> Result<(String, Vec<u8>), String> {
    let raw = mail::raw(&r.account_id, &r.folder, r.uid, r.validity)?;
    parse::attachment(&raw, r.index).ok_or_else(|| "GONE|".into())
}

/// Saves an attachment where the user chooses (None = cancelled).
#[tauri::command]
async fn attachment_save(app: AppHandle, att: AttRef) -> Result<Option<String>, String> {
    let (name, bytes) = work(move || attachment_of(&att)).await?;
    let Some(f) = shared::file_dialog(&app).set_file_name(parse::safe_name(&name)).blocking_save_file() else { return Ok(None) };
    let p = f.into_path().map_err(|e| e.to_string())?;
    std::fs::write(&p, bytes).map_err(|e| format!("FILE|{}: {e}", p.display()))?;
    Ok(Some(p.to_string_lossy().into_owned()))
}

/// Opens an attachment with the program the system uses for it (never programs or scripts: those can only be saved).
#[tauri::command]
async fn attachment_open(app: AppHandle, att: AttRef) -> Result<(), String> {
    let (name, bytes) = work(move || attachment_of(&att)).await?;
    let name = parse::safe_name(&name);
    if parse::risky(&name) { return Err(format!("RISKY|{name}")); }
    let dir = std::env::temp_dir().join("Mail Studio").join(format!("{:x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let p = dir.join(&name);
    std::fs::write(&p, bytes).map_err(|e| format!("FILE|{}: {e}", p.display()))?;
    app.opener().open_path(p.to_string_lossy(), None::<&str>).map_err(|e| e.to_string())
}

/// The attachments of a message as files (for forwarding it with them).
#[tauri::command]
async fn forward_files(account_id: String, folder: String, uid: u32, validity: u32) -> Result<Vec<Picked>, String> {
    work(move || {
        let raw = mail::raw(&account_id, &folder, uid, validity)?;
        let v = parse::view(&raw).ok_or("PARSE|")?;
        let dir = std::env::temp_dir().join("Mail Studio").join(format!("fwd-{uid}-{:x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis()));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let mut out = vec![];
        for a in v.attachments.iter().filter(|a| !a.inline) {
            let Some((name, bytes)) = parse::attachment(&raw, a.index) else { continue };
            let mut p = dir.join(parse::safe_name(&name));
            if p.exists() { p = dir.join(format!("{}-{}", a.index, parse::safe_name(&name))); }
            std::fs::write(&p, &bytes).map_err(|e| format!("FILE|{e}"))?;
            out.push(Picked { path: p.to_string_lossy().into_owned(), name: parse::safe_name(&name), size: bytes.len() as u64 });
        }
        Ok(out)
    }).await
}

/// Saves the whole message as an .eml file (None = cancelled).
#[tauri::command]
async fn message_save(app: AppHandle, account_id: String, folder: String, uid: u32, validity: u32, name: String) -> Result<Option<String>, String> {
    let raw = work(move || mail::raw(&account_id, &folder, uid, validity)).await?;
    let Some(f) = shared::file_dialog(&app).set_file_name(format!("{}.eml", parse::safe_name(&name))).add_filter("E-Mail", &["eml"]).blocking_save_file() else { return Ok(None) };
    let p = f.into_path().map_err(|e| e.to_string())?;
    std::fs::write(&p, raw).map_err(|e| format!("FILE|{}: {e}", p.display()))?;
    Ok(Some(p.to_string_lossy().into_owned()))
}

/* ---------------- Window helpers (as in every Studio app) ---------------- */

#[tauri::command]
async fn confirm(app: AppHandle, title: String, message: String, ok_label: String, cancel_label: String) -> bool {
    app.dialog().message(message).title(title).kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(ok_label, cancel_label)).blocking_show()
}

#[tauri::command]
fn open_url(app: AppHandle, url: String) -> Result<(), String> {
    let allowed = ["https://", "http://", "mailto:"].iter().any(|p| url.starts_with(p));
    if !allowed { return Err(format!("Refusing to open {url}")); }
    app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
}

#[derive(Deserialize)]
struct MenuEntry {
    id: Option<String>,
    label: Option<String>,
    enabled: Option<bool>,
    checked: Option<bool>,
    accel: Option<String>,
}

/// Shows a native (OS) context menu at the mouse position; the chosen id reaches the UI as a "context-menu" event.
#[tauri::command]
fn context_menu(app: AppHandle, window: tauri::Window, items: Vec<MenuEntry>) -> Result<(), String> {
    use tauri::menu::{CheckMenuItemBuilder, MenuBuilder, MenuItemBuilder};
    let mut menu = MenuBuilder::new(&app);
    for entry in items {
        let Some(id) = entry.id else { menu = menu.separator(); continue; };
        let label = entry.label.unwrap_or_default();
        let enabled = entry.enabled.unwrap_or(true);
        if let Some(checked) = entry.checked {
            let item = CheckMenuItemBuilder::with_id(id, label).enabled(enabled).checked(checked).build(&app).map_err(|e| e.to_string())?;
            menu = menu.item(&item);
            continue;
        }
        let plain = || MenuItemBuilder::with_id(id.clone(), label.clone()).enabled(enabled);
        let item = match entry.accel.as_deref() {
            Some(a) => plain().accelerator(a).build(&app).or_else(|_| plain().build(&app)),
            None => plain().build(&app),
        }.map_err(|e| e.to_string())?;
        menu = menu.item(&item);
    }
    let menu = menu.build().map_err(|e| e.to_string())?;
    window.popup_menu(&menu).map_err(|e| e.to_string())
}

#[tauri::command]
fn clipboard_read(app: AppHandle) -> String { app.clipboard().read_text().unwrap_or_default() }

#[tauri::command]
fn clipboard_write(app: AppHandle, text: String) -> Result<(), String> { app.clipboard().write_text(text).map_err(|e| e.to_string()) }

/// A mailto: link the app was started with (e.g. `mail-studio mailto:ana@example.org?subject=Hi`).
#[tauri::command]
fn initial_mailto() -> Option<String> {
    std::env::args().skip(1).find(|a| a.to_ascii_lowercase().starts_with("mailto:"))
}

fn main() {
    if mcp::wanted() { return mcp_tools::run(); }
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .on_menu_event(|app, event| { let _ = app.emit("context-menu", event.id().as_ref()); })
        .invoke_handler(tauri::generate_handler![
            shared::shared_get, shared::shared_set, texts, appicon::set_dock_icon,
            accounts_list, account_preset, account_test, account_save, account_remove, accounts_reorder,
            folders, messages, message_read, message_source, set_flag, move_messages, move_to_role, search, create_folder,
            send, save_draft, pick_attachments, file_infos, attachment_save, attachment_open, forward_files, message_save,
            confirm, open_url, context_menu, clipboard_read, clipboard_write, initial_mailto
        ])
        .build(tauri::generate_context!())
        .expect("error while building Mail Studio");
    app.run(|_app, _event| {
        // macOS hands over mailto: links as an event.
        #[cfg(target_os = "macos")]
        if let tauri::RunEvent::Opened { urls } = _event {
            for u in urls.into_iter().filter(|u| u.scheme() == "mailto") { let _ = _app.emit("mailto", u.to_string()); }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texts_parse_and_match() {
        let t = texts();
        let (en, de) = (t["en"].as_object().unwrap(), t["de"].as_object().unwrap());
        assert!(en.len() > 100);
        let missing: Vec<&String> = en.keys().filter(|k| !de.contains_key(*k)).collect();
        assert!(missing.is_empty(), "every English text has a German one: {missing:?}");
        let extra: Vec<&String> = de.keys().filter(|k| !en.contains_key(*k)).collect();
        assert!(extra.is_empty(), "no German text without an English one: {extra:?}");
    }
}
