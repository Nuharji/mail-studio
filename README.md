# Mail Studio (Rust / Tauri)

> **Status: Pre-Alpha (0.1)** — works against real IMAP / SMTP servers, but has not yet been tried in everyday use.

E-mail program of the Studio family — all your mail accounts in one window, with the same chrome and controls as the
other Studio apps. The native shell is Rust ([Tauri 2](https://tauri.app)); the UI is plain HTML/JS in `ui/`.

## Features (0.1.0)
- **Accounts:** any IMAP / SMTP account; settings filled in from the address for iCloud, Gmail, Outlook / Hotmail,
  Yahoo, GMX, WEB.DE, Telekom (magenta.de), Posteo, mailbox.org, AOL, freenet and Proton Mail (through Proton Mail
  Bridge), with hints when the provider needs an app password or web-mail setting first; "Test connection" before saving
- **Passwords in the system keychain** (macOS Keychain, Windows Credential Manager, Secret Service on Linux) — never in
  a file
- **Folders** with unread counts, sorted by role (Inbox, Drafts, Sent, Archive, Junk, Trash), new folders
- **Message list:** newest first, loads more while scrolling; read / unread, flag, archive, junk, delete, move (also
  several at once), search in the folder (sender, subject, text)
- **Reading:** HTML mail in a locked-down frame (no scripts; remote images only when you allow them), plain text,
  attachments to open or save, message source, save as `.eml`
- **Writing:** reply, reply all, forward (with attachments), To / Cc / Bcc, attachments by button or drag & drop,
  drafts saved into the account's Drafts folder and restored after a restart; sent mail is put into "Sent" when the
  server doesn't do it itself
- **Offline:** folders, lists and opened messages are cached and readable without a connection
- `mailto:` links (start argument, and on macOS when Mail Studio is the default mail app)
- All Studio themes and layouts, logo follows the theme, German / English, customizable shortcuts, auto-hide bars

## MCP
Mail Studio is also an MCP server, so Claude (Desktop or Code), ChatGPT and other MCP clients can use your mail:
started with `--mcp`, the program opens no window and speaks MCP on stdin / stdout.

| Tool | What it does |
|---|---|
| `mail_accounts` | the accounts set up in Mail Studio |
| `mail_folders` | folders of an account with unread / total counts |
| `mail_list` | the newest messages of a folder |
| `mail_read` | one message (headers, text, attachment names) — does not mark it as read |
| `mail_search` | messages in a folder that contain all the words |
| `mail_draft` | writes a draft into the Drafts folder — **never sends**; you review and send it in Mail Studio |

## Build
```
cargo install tauri-cli --version "^2"
cd src-tauri && cargo tauri build
```
Releases for Windows, macOS and Linux are built by GitHub Actions (`.github/workflows/release.yml`) when a `v*` tag is
pushed.
