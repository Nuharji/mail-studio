/* Mail Studio — e-mail for all your accounts. Accounts in the rail on the left, their folders next to it, the message
   list and the reader. The mail work is in Rust (src-tauri/src: IMAP, SMTP, cache, keychain); this file is the window. */
'use strict';
const $ = s => document.querySelector(s), $$ = s => [...document.querySelectorAll(s)];
const STORE = 'mail-studio.v1';
const invoke = window.__TAURI__ ? window.__TAURI__.core.invoke : null;
const esc = s => String(s ?? '').replace(/[&<>"']/g, c => ({'&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;'}[c]));
const el = (tag, props = {}, ...kids) => { const n = Object.assign(document.createElement(tag), props); n.append(...kids); return n; };
const call = (cmd, args) => invoke ? invoke(cmd, args) : Promise.reject('NOAPP|');

/* ================= Texts (src-tauri/i18n/*.json, served by Rust) ================= */
let I18N = {en: {}, de: {}}, lang = 'en';
function t(key, vars) {
  let s = (I18N[lang] && I18N[lang][key]) ?? I18N.en[key] ?? key;
  if (vars) for (const k in vars) s = s.split('{' + k + '}').join(vars[k]);
  return s;
}
const fmtKey = k => lang === 'de' ? k.replace(/Ctrl/g, 'Strg').replace(/Shift/g, 'Umschalt') : k;
async function loadTexts() {
  if (invoke) { I18N = await invoke('texts'); return; }
  const get = l => fetch(`../src-tauri/i18n/${l}.json`).then(r => r.json()).catch(() => ({}));
  I18N = {en: await get('en'), de: await get('de')};
}
/* Errors from Rust start with a kind ("AUTH|server text"). */
function errText(e) {
  const s = String(e && e.message || e);
  const i = s.indexOf('|');
  if (i < 0) return s;
  let kind = s.slice(0, i), detail = s.slice(i + 1), pre = '';
  if (kind.startsWith('SMTP-')) { pre = t('err.smtpPart') + ' '; kind = kind.slice(5); }
  if (kind === 'FIELD') return t('field.' + detail);
  const key = 'err.' + kind;
  const base = t(key) === key ? kind : t(key, {x: detail});
  return pre + base + (detail && !t(key).includes('{x}') ? ` (${detail})` : '');
}

/* ================= Settings and state ================= */
const THEMES = ['dark', 'light', 'github', 'github-dark', 'cli', 'cli-amber', 'cli-cyan', 'cli-magenta', 'cli-white', 'midnight', 'dusk', 'graphite', 'contrast', 'sand', 'rose'];
const DEF = {keys: {}, autoHide: false, lang: 'auto', theme: 'dark', interval: 5, markRead: 'open', images: 'ask', trusted: [], listW: 400,
  account: '', folder: 'INBOX', pageSize: 100};
let settings = {...DEF}, booted = false;
let listAccount = '', accounts = [], folders = {}, page = null, list = [], sel = new Set(), cur = null, view = null, search = null, loading = 0, offline = false, lastSync = 0;
const effectiveLang = () => settings.lang === 'auto' ? ((navigator.language || 'en').toLowerCase().startsWith('de') ? 'de' : 'en') : settings.lang;
const save = () => { try { localStorage.setItem(STORE, JSON.stringify({settings})); } catch (e) {} };
const acc = () => accounts.find(a => a.id === settings.account) || null;
const fl = () => (folders[settings.account] || []).find(f => f.id === settings.folder) || null;
const role = () => (fl() || {}).role || (settings.folder === 'INBOX' ? 'inbox' : '');
const outgoing = () => ['sent', 'drafts'].includes(role());

let toastT;
function toast(msg, error = false) {
  const n = $('#toast'); n.textContent = msg; n.classList.add('show'); n.style.borderLeftColor = error ? 'var(--danger)' : '';
  clearTimeout(toastT); toastT = setTimeout(() => n.classList.remove('show'), error ? 6000 : 2600);
}
function busy(d) {
  loading = Math.max(0, loading + d);
  const n = $('#stNet');
  n.className = 'st-net' + (loading ? ' busy' : offline ? ' off' : '');
  n.textContent = loading ? t('st.loading') : offline ? t('st.offline') : lastSync ? t('st.synced', {time: fmtTime(lastSync / 1000)}) : '';
}
async function net(promise, quiet = false) {
  busy(1);
  try { const r = await promise; offline = false; lastSync = Date.now(); return r; }
  catch (e) {
    const s = String(e);
    if (/^(CONNECT|TIMEOUT|TLS)\|/.test(s)) offline = true;
    if (!quiet) toast(errText(e), true);
    throw e;
  } finally { busy(-1); }
}

/* ================= Dates, names ================= */
function fmtTime(ts) { return new Date(ts * 1000).toLocaleTimeString(lang, {hour: '2-digit', minute: '2-digit'}); }
function fmtDate(ts, long = false) {
  if (!ts) return '';
  const d = new Date(ts * 1000), now = new Date();
  if (long) return d.toLocaleString(lang, {weekday: 'short', day: 'numeric', month: 'long', year: 'numeric', hour: '2-digit', minute: '2-digit'});
  if (d.toDateString() === now.toDateString()) return fmtTime(ts);
  const y = new Date(now); y.setDate(now.getDate() - 1);
  if (d.toDateString() === y.toDateString()) return t('date.yesterday');
  if (now - d < 6 * 864e5) return d.toLocaleDateString(lang, {weekday: 'short'});
  if (d.getFullYear() === now.getFullYear()) return d.toLocaleDateString(lang, {day: 'numeric', month: 'short'});
  return d.toLocaleDateString(lang, {day: '2-digit', month: '2-digit', year: 'numeric'});
}
const who = p => p ? (p.name || p.email || '') : '';
const fullAddr = p => p.name ? `"${p.name.replace(/"/g, "'")}" <${p.email}>` : p.email;
const initials = a => ((a.label || a.name || a.email || '?').replace(/[^\p{L}\p{N} ]/gu, ' ').trim().split(/\s+/).map(w => w[0]).join('').slice(0, 2) || '?').toUpperCase();
function colorOf(s) {
  let h = 0; for (const c of String(s)) h = (h * 31 + c.codePointAt(0)) >>> 0;
  return `hsl(${h % 360} 55% 45%)`;
}
const fmtSize = n => n < 1024 ? `${n} B` : n < 1048576 ? `${Math.round(n / 1024)} KB` : `${(n / 1048576).toFixed(1)} MB`;

/* ================= Icons ================= */
const SV = p => `<svg viewBox="0 0 24 24">${p}</svg>`;
const ICONS = {
  compose: SV('<path d="M12 20h9"/><path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4z"/>'),
  refresh: SV('<path d="M21 12a9 9 0 1 1-2.6-6.4"/><path d="M21 3v6h-6"/>'),
  reply: SV('<path d="M9 17 4 12l5-5"/><path d="M20 18v-2a4 4 0 0 0-4-4H4"/>'),
  replyAll: SV('<path d="M7 17 2 12l5-5"/><path d="M12 17l-5-5 5-5"/><path d="M22 18v-2a4 4 0 0 0-4-4H7"/>'),
  forward: SV('<path d="m15 17 5-5-5-5"/><path d="M4 18v-2a4 4 0 0 1 4-4h12"/>'),
  archive: SV('<rect x="2" y="4" width="20" height="5" rx="1"/><path d="M4 9v9a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9M10 13h4"/>'),
  delete: SV('<path d="M3 6h18M8 6V4h8v2M19 6l-1 14H6L5 6"/>'),
  junk: SV('<circle cx="12" cy="12" r="9"/><path d="m5.7 5.7 12.6 12.6"/>'),
  unread: SV('<circle cx="12" cy="12" r="4"/>'),
  flag: SV('<path d="M4 22V4M4 4h13l-2 4 2 4H4"/>'),
  move: SV('<path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/><path d="M10 13h6m-2-2 2 2-2 2"/>'),
  inbox: SV('<path d="M22 12h-6l-2 3h-4l-2-3H2"/><path d="M5.5 5h13L22 12v6a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2v-6z"/>'),
  sent: SV('<path d="m22 2-7 20-4-9-9-4z"/><path d="M22 2 11 13"/>'),
  drafts: SV('<path d="M14 3H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9z"/><path d="M14 3v6h6M8 13h8M8 17h5"/>'),
  trash: SV('<path d="M3 6h18M8 6V4h8v2M19 6l-1 14H6L5 6"/>'),
  folder: SV('<path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>'),
  all: SV('<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M3 9h18"/>'),
  flagged: SV('<path d="M4 22V4M4 4h13l-2 4 2 4H4"/>'),
  add: SV('<path d="M12 5v14M5 12h14"/>'),
  settings: SV('<circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z"/>'),
  attach: SV('<path d="m21 12-8.5 8.5a5 5 0 0 1-7-7L14 5a3.3 3.3 0 0 1 4.7 4.7l-8.5 8.5a1.7 1.7 0 0 1-2.4-2.4L15.5 8"/>'),
  save: SV('<path d="M12 3v12m-5-5 5 5 5-5"/><path d="M5 21h14"/>'),
  open: SV('<path d="M14 3h7v7M21 3l-9 9"/><path d="M19 14v5a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7a2 2 0 0 1 2-2h5"/>')
};
const ROLE_ICON = {inbox: 'inbox', sent: 'sent', drafts: 'drafts', trash: 'trash', junk: 'junk', archive: 'archive', all: 'all', flagged: 'flagged'};

/* ================= Accounts and folders ================= */
async function loadAccounts() {
  accounts = invoke ? await call('accounts_list').catch(e => { toast(errText(e), true); return []; }) : [];
  if (!acc() && accounts.length) { settings.account = accounts[0].id; settings.folder = 'INBOX'; }
  if (!accounts.length) settings.account = '';
  renderRail();
}
function renderRail() {
  const r = $('#tools'); r.innerHTML = '';
  for (const a of accounts) {
    const inbox = (folders[a.id] || []).find(f => f.role === 'inbox');
    const b = el('button', {className: 'acct' + (a.id === settings.account ? ' active' : ''), title: `${a.label || a.name}\n${a.email}`});
    b.style.setProperty('--c', colorOf(a.email));
    b.innerHTML = esc(initials(a)) + (inbox && inbox.unseen ? `<b>${inbox.unseen > 99 ? '99+' : inbox.unseen}</b>` : '');
    b.onclick = () => selectAccount(a.id);
    b.dataset.acct = a.id;
    r.append(b);
  }
  r.append(el('div', {className: 'sep'}));
  const add = el('button', {className: 'ibtn', title: t('a.addAccount')}); add.innerHTML = ICONS.add; add.onclick = () => accountDialog();
  r.append(add, el('div', {className: 'grow'}));
  const st = el('button', {className: 'ibtn', title: tip('settings')}); st.innerHTML = ICONS.settings; st.onclick = () => run('settings');
  r.append(st);
}
async function selectAccount(id) {
  if (id === settings.account) return;
  settings.account = id; settings.folder = 'INBOX'; save();
  renderRail(); await showFolders(); await openFolder('INBOX');
}
async function showFolders(fresh = true) {
  const a = acc();
  $('#filesHead').innerHTML = a ? `<b>${esc(a.label || a.name || a.email)}</b><span>${esc(a.email)}</span>` : '';
  if (!a) { $('#flist').innerHTML = ''; return; }
  if (!folders[a.id]) folders[a.id] = await call('folders', {accountId: a.id, cached: true}).catch(() => []);
  renderFolders();
  if (!fresh) return;
  try {
    folders[a.id] = await net(call('folders', {accountId: a.id}), true);
    renderFolders(); renderRail();
  } catch (e) {
    if (!folders[a.id].length) $('#flist').innerHTML = `<div class="empty">${esc(errText(e))}<br><button class="btn-sm" data-act="editAccount">${esc(t('a.editAccount'))}</button></div>`;
    else toast(errText(e), true);
  }
}
function folderName(f) {
  if (f.role && f.role !== 'all' && f.role !== 'flagged') return t('role.' + f.role);
  const parts = f.name.split(f.delimiter || '/');
  return parts[parts.length - 1] || f.name;
}
function depth(f) {
  if (f.role) return 0;
  const d = f.delimiter || '/';
  let n = f.name.split(d).length - 1;
  if (f.id.toUpperCase().startsWith('INBOX' + d)) n -= 1;   // INBOX.Sub on servers that put everything under INBOX
  return Math.max(0, n);
}
function renderFolders() {
  const box = $('#flist'); box.innerHTML = '';
  for (const f of folders[settings.account] || []) {
    const b = el('button', {className: 'fold' + (f.id === settings.folder && !search ? ' active' : '') + (f.selectable ? '' : ' dis')});
    b.style.paddingLeft = (8 + depth(f) * 14) + 'px';
    b.innerHTML = ICONS[ROLE_ICON[f.role] || 'folder'] + `<span class="n">${esc(folderName(f))}</span>` +
      (f.unseen && f.role !== 'sent' && f.role !== 'trash' ? `<span class="c">${f.unseen}</span>` : f.role === 'drafts' && f.total ? `<span class="c">${f.total}</span>` : '');
    b.title = f.name;
    b.dataset.folder = f.id;
    if (f.selectable) {
      b.onclick = () => openFolder(f.id);
      b.ondragover = e => { if (e.dataTransfer.types.includes('text/x-mail-uids') && f.id !== settings.folder) { e.preventDefault(); b.classList.add('drop'); } };
      b.ondragleave = () => b.classList.remove('drop');
      b.ondrop = e => { e.preventDefault(); b.classList.remove('drop'); const uids = JSON.parse(e.dataTransfer.getData('text/x-mail-uids') || '[]'); moveTo(uids, f.id); };
    }
    box.append(b);
  }
}

/* ================= Message list ================= */
async function openFolder(id, keepSel = false) {
  const a = acc(); if (!a) return;
  // Another folder (or account): the old list must not stay, nor be mixed into the new one by refreshList.
  if (id !== settings.folder || a.id !== listAccount) { cur = null; view = null; sel.clear(); page = null; list = []; }
  listAccount = a.id;
  settings.folder = id; search = null; $('#search').value = ''; save();
  renderFolders();
  if (!keepSel) {
    const c = await call('messages', {accountId: a.id, folder: id, cached: true}).catch(() => null);
    if (c && settings.folder === id && settings.account === a.id) { page = c; list = c.messages; }
    renderList();
  }
  if (!cur) renderReader();
  await refreshList();
}
async function refreshList(quiet = false) {
  const a = acc(); if (!a) return;
  const id = settings.folder;
  try {
    const p = await net(call('messages', {accountId: a.id, folder: id, limit: settings.pageSize}), quiet);
    if (settings.folder !== id || search || a.id !== settings.account) return;
    // Keep older pages that were loaded with "more".
    const older = page && page.uidValidity === p.uidValidity ? list.filter(m => m.seq < p.first) : [];
    page = p; list = p.messages.concat(older);
    if (older.length) page.first = Math.min(...older.map(m => m.seq));
    for (const u of [...sel]) if (!list.some(m => m.uid === u)) sel.delete(u);
    if (cur && !list.some(m => m.uid === cur)) { cur = null; view = null; renderReader(); }
    renderList();
  } catch (e) { if (!list.length) renderList(errText(e)); }
}
async function loadMore() {
  const a = acc(); if (!a || !page || page.first <= 1) return;
  const p = await net(call('messages', {accountId: a.id, folder: settings.folder, limit: settings.pageSize, before: page.first}));
  list = list.concat(p.messages); page.first = p.first; renderList();
}
async function runSearch(q) {
  const a = acc(); if (!a) return;
  if (!q.trim()) { search = null; renderFolders(); return openFolder(settings.folder); }
  search = {q, folder: settings.folder};
  renderFolders();
  $('#listHead').innerHTML = `<b>${esc(t('ui.searching', {q}))}</b>`;
  try {
    const r = await net(call('search', {accountId: a.id, folder: settings.folder, query: q}));
    if (!search || search.q !== q) return;
    list = r; sel.clear(); renderList();
  } catch (e) { renderList(errText(e)); }
}
function visible() {
  // Typing in the search field filters at once; Enter asks the server.
  const q = ($('#search').value || '').trim().toLowerCase();
  if (!q || search) return list;
  return list.filter(m => [m.subject, m.from.name, m.from.email, ...(m.to || []).map(x => x.name + ' ' + x.email)].join(' ').toLowerCase().includes(q));
}
function renderList(error) {
  const f = fl(), box = $('#list'), items = visible();
  const unread = list.filter(m => !m.seen).length;
  $('#listHead').innerHTML = search ? `<b>${esc(t('ui.results', {q: search.q, n: list.length}))}</b><button class="btn-sm" id="endSearch">${esc(t('ui.endSearch'))}</button>`
    : `<b>${esc(f ? folderName(f) : settings.folder)}</b><span>${esc(page ? t('ui.count', {n: page.total}) : '')}${unread ? ' · ' + esc(t('ui.unread', {n: unread})) : ''}</span>`;
  if ($('#endSearch')) $('#endSearch').onclick = () => { $('#search').value = ''; runSearch(''); };
  box.innerHTML = '';
  if (!acc()) return;
  if (!items.length) { box.innerHTML = `<div class="empty">${esc(error || (search ? t('ui.noResults') : t('ui.emptyFolder')))}</div>`; updateStatus(); return; }
  const frag = document.createDocumentFragment();
  const showTo = outgoing();
  for (const m of items) {
    const r = el('div', {className: 'row' + (m.seen ? '' : ' unseen') + (sel.has(m.uid) ? ' sel' : '') + (m.uid === cur ? ' cur' : ''), draggable: true});
    r.dataset.uid = m.uid;
    const name = showTo ? (m.to || []).map(who).join(', ') || t('ui.noRecipient') : who(m.from) || t('ui.unknown');
    r.innerHTML = `<span class="dot"></span><span class="who">${esc(showTo ? t('ui.toPrefix') + ' ' + name : name)}</span><span class="when">${esc(fmtDate(m.date))}</span>
      <span class="subj">${m.flagged ? '<span class="ic fl">&#9873;</span>' : ''}${m.answered ? '<span class="ic">&#8617;</span>' : ''}${m.attach ? '<span class="ic">&#128206;</span>' : ''}<span>${esc(m.subject || t('ui.noSubject'))}</span></span>`;
    frag.append(r);
  }
  box.append(frag);
  if (!search && page && page.first > 1 && !($('#search').value || '').trim()) {
    const b = el('button', {className: 'btn-sm more', textContent: t('ui.loadMore', {n: page.first - 1})}); b.onclick = loadMore; box.append(b);
  }
  updateStatus(); syncDisabled();
}
function updateStatus() {
  const a = acc();
  $('#stCount').textContent = page && !search ? t('ui.count', {n: page.total}) + (sel.size > 1 ? ' · ' + t('ui.selected', {n: sel.size}) : '') : '';
  $('#stAcct').textContent = a ? a.email : '';
  const inbox = a && (folders[a.id] || []).find(f => f.role === 'inbox');
  const f = fl();
  document.title = 'Mail Studio' + (f ? ` — ${folderName(f)}` : '') + (inbox && inbox.unseen ? ` (${inbox.unseen})` : '');
}
const byUid = uid => list.find(m => m.uid === uid);
let anchor = null;
$('#list').addEventListener('mousedown', e => {
  const r = e.target.closest('.row'); if (!r || e.button !== 0) return;
  const uid = +r.dataset.uid;
  if (e.metaKey || e.ctrlKey) { sel.has(uid) ? sel.delete(uid) : sel.add(uid); anchor = uid; renderList(); return; }
  if (e.shiftKey && anchor != null) {
    const ids = visible().map(m => m.uid), a = ids.indexOf(anchor), b = ids.indexOf(uid);
    if (a >= 0 && b >= 0) { sel = new Set(ids.slice(Math.min(a, b), Math.max(a, b) + 1)); renderList(); return; }
  }
  if (sel.has(uid) && sel.size > 1) return;   // dragging a multi-selection
  anchor = uid; openMessage(uid);
});
$('#list').addEventListener('click', e => {
  const r = e.target.closest('.row'); if (!r || e.metaKey || e.ctrlKey || e.shiftKey) return;
  const uid = +r.dataset.uid;
  if (sel.size > 1 && sel.has(uid)) openMessage(uid);
});
$('#list').addEventListener('dblclick', e => { const r = e.target.closest('.row'); if (r && role() === 'drafts') editDraft(+r.dataset.uid); });
$('#list').addEventListener('dragstart', e => {
  const r = e.target.closest('.row'); if (!r) return;
  const uid = +r.dataset.uid, uids = sel.has(uid) ? [...sel] : [uid];
  e.dataTransfer.setData('text/x-mail-uids', JSON.stringify(uids)); e.dataTransfer.effectAllowed = 'move';
});
function step(d) {
  const ids = visible().map(m => m.uid);
  if (!ids.length) return;
  const i = ids.indexOf(cur), n = ids[Math.max(0, Math.min(ids.length - 1, i < 0 ? 0 : i + d))];
  openMessage(n);
  const r = $(`.row[data-uid="${n}"]`); if (r) r.scrollIntoView({block: 'nearest'});
}

/* ================= Reader ================= */
let readSeq = 0, markT;
async function openMessage(uid) {
  const m = byUid(uid); if (!m) return;
  cur = uid; sel = new Set([uid]); renderList();
  const a = acc(), folder = settings.folder, mySeq = ++readSeq;
  $('#reader').innerHTML = `<div class="reader-in"><div class="empty">${esc(t('ui.loadingMsg'))}</div></div>`;
  try {
    const v = await net(call('message_read', {accountId: a.id, folder, uid, validity: page ? page.uidValidity : 0}));
    if (mySeq !== readSeq) return;
    view = Object.assign(v, {uid, folder, account: a.id});
    renderReader();
    clearTimeout(markT);
    if (!m.seen && settings.markRead !== 'never') markT = setTimeout(() => { if (cur === uid) setFlag([uid], 'seen', true, true); }, settings.markRead === 'delay' ? 2000 : 0);
  } catch (e) {
    if (mySeq === readSeq) $('#reader').innerHTML = `<div class="reader-in"><div class="empty">${esc(errText(e))}</div></div>`;
  }
}
const REMOTE = /(\bsrc|\bbackground|\bposter|\bsrcset)\s*=\s*["']?\s*(https?:)?\/\/|url\(\s*["']?\s*(https?:)?\/\//i;
function linkify(s) {
  return esc(s).replace(/\b(https?:\/\/[^\s<>"')\]]+)/g, '<a href="$1">$1</a>')
    .replace(/(^|\n)(&gt;[^\n]*)/g, (m, nl, q) => `${nl}<span class="q">${q}</span>`);
}
function addrHtml(list) {
  return list.map(p => `<span class="addr" data-mail="${esc(p.email)}" title="${esc(p.email)}">${esc(p.name || p.email)}${p.name ? ` <small>&lt;${esc(p.email)}&gt;</small>` : ''}</span>`).join(', ');
}
function renderReader() {
  const r = $('#reader');
  if (!accounts.length) {
    r.innerHTML = `<div class="empty-state welcome"><div class="big-tile">Ml</div><h1>${esc(t('ui.welcome'))}</h1><p>${esc(t('ui.welcomeHint'))}</p>
      <div class="empty-actions"><button class="btn-sm primary" data-act="addAccount">${esc(t('a.addAccount'))}</button></div>
      <div class="provs">${['iCloud', 'Gmail', 'GMX', 'WEB.DE', 'Proton Mail (Bridge)', 'Telekom', 'Posteo', 'mailbox.org', 'Yahoo', t('ui.anyImap')].map(p => `<span>${esc(p)}</span>`).join('')}</div></div>`;
    return;
  }
  if (!view || view.uid !== cur) {
    const n = sel.size;
    r.innerHTML = `<div class="empty-state"><div class="big-tile">Ml</div><p>${esc(n > 1 ? t('ui.selected', {n}) : t('ui.pickMessage'))}</p></div>`;
    return;
  }
  const v = view, m = byUid(v.uid) || {};
  const atts = v.attachments.filter(x => !x.inline);
  const html = v.html && v.html.trim();
  const sender = (v.from[0] || {}).email || '';
  const remote = html && REMOTE.test(v.html);
  const allow = remote && (settings.images === 'always' || settings.trusted.includes(sender.toLowerCase()) || v.showImages);
  const rows = [['from', v.from], ['to', v.to], ['cc', v.cc]].filter(([, l]) => l.length)
    .map(([k, l], i) => `<span class="k">${esc(t('rd.' + k))}</span><span class="v">${addrHtml(l)}</span>${i === 0 ? `<span class="d">${esc(fmtDate(v.date, true))}</span>` : ''}`).join('');
  r.innerHTML = `<div class="reader-in">
    <div class="rhead"><h1>${esc(v.subject || t('ui.noSubject'))}</h1><div class="rmeta">${rows}</div>
      <div class="ractions">${['reply', 'replyAll', 'forward'].map(a => `<button class="btn-sm" data-act="${a}">${esc(t('a.' + a))}</button>`).join('')}
        ${role() === 'drafts' ? `<button class="btn-sm primary" data-act="editDraft">${esc(t('a.editDraft'))}</button>` : ''}
        ${v.unsubscribe ? `<button class="btn-sm" id="unsub">${esc(t('rd.unsubscribe'))}</button>` : ''}
        <span class="flex"></span>${m.flagged ? `<span class="ic" style="color:var(--warn)">&#9873; ${esc(t('rd.flagged'))}</span>` : ''}</div></div>
    ${atts.length ? `<div class="atts">${atts.map(x => `<div class="att" data-att="${x.index}"><span class="nm" title="${esc(x.name)}">${esc(x.name)}</span><small>${fmtSize(x.size)}</small>
      <button class="ibtn" data-att-open="${x.index}" title="${esc(t('rd.open'))}">${ICONS.open}</button><button class="ibtn" data-att-save="${x.index}" title="${esc(t('rd.save'))}">${ICONS.save}</button></div>`).join('')}
      ${atts.length > 1 ? `<button class="btn-sm" id="saveAll">${esc(t('rd.saveAll'))}</button>` : ''}</div>` : ''}
    ${remote && !allow ? `<div class="imgbar"><span>${esc(t('rd.imagesBlocked'))}</span><button class="btn-sm" id="imgOnce">${esc(t('rd.loadImages'))}</button>
      <button class="btn-sm" id="imgAlways">${esc(t('rd.trustSender'))}</button></div>` : ''}
    ${html ? '<div class="paper"><iframe id="mailFrame" sandbox="allow-same-origin allow-popups" referrerpolicy="no-referrer"></iframe></div>'
      : `<div class="plain">${linkify(v.text || '')}</div>`}
  </div>`;
  if (html) frameHtml($('#mailFrame'), v.html, allow);
  const once = $('#imgOnce'); if (once) once.onclick = () => { view.showImages = true; renderReader(); };
  const always = $('#imgAlways'); if (always) always.onclick = () => { if (sender) settings.trusted.push(sender.toLowerCase()); save(); renderReader(); };
  const un = $('#unsub'); if (un) un.onclick = () => unsubscribe(v.unsubscribe);
  const all = $('#saveAll'); if (all) all.onclick = async () => { for (const x of atts) if (!await saveAttachment(x.index)) break; };
}
/* HTML mail in a sandboxed frame: no scripts, no forms, no outside requests unless allowed; links open outside. */
function frameHtml(frame, html, remote) {
  const csp = `default-src 'none'; img-src data: cid:${remote ? ' https: http:' : ''}; style-src 'unsafe-inline'${remote ? ' https:' : ''}; font-src data:${remote ? ' https:' : ''}; media-src data:`;
  frame.srcdoc = `<!DOCTYPE html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="${csp}"><meta name="referrer" content="no-referrer">` +
    `<base target="_blank"><style>html,body{margin:0;background:#fff;color:#1f1f1f}body{padding:16px 18px;font:14px/1.5 -apple-system,"Segoe UI",system-ui,sans-serif;overflow-wrap:anywhere}img{max-width:100%;height:auto}table{max-width:100%}pre{white-space:pre-wrap}</style></head><body>${html}</body></html>`;
  frame.onload = () => {
    const d = frame.contentDocument; if (!d) return;
    const fit = () => { frame.style.height = Math.max(120, d.documentElement.scrollHeight) + 'px'; };
    fit(); setTimeout(fit, 300); setTimeout(fit, 1500);
    d.querySelectorAll('img').forEach(i => i.addEventListener('load', fit));
    d.addEventListener('click', e => {
      const a = e.target.closest && e.target.closest('a[href]'); if (!a) return;
      e.preventDefault();
      const href = a.getAttribute('href') || '';
      if (/^mailto:/i.test(href)) composeMailto(href);
      else if (/^https?:/i.test(href)) call('open_url', {url: href}).catch(() => {});
    });
    d.addEventListener('keydown', e => window.dispatchEvent(new KeyboardEvent('keydown', e)));
  };
}
$('#reader').addEventListener('click', e => {
  const a = e.target.closest('.plain a, .rhead a');
  if (a) { e.preventDefault(); const h = a.getAttribute('href'); if (/^mailto:/i.test(h)) composeMailto(h); else call('open_url', {url: h}).catch(() => {}); return; }
  const ad = e.target.closest('.addr'); if (ad) { openCompose({to: ad.dataset.mail}); return; }
  const o = e.target.closest('[data-att-open]'); if (o) { openAttachment(+o.dataset.attOpen); return; }
  const s = e.target.closest('[data-att-save]'); if (s) saveAttachment(+s.dataset.attSave);
});
const attRef = index => ({accountId: view.account, folder: view.folder, uid: view.uid, validity: page ? page.uidValidity : 0, index});
async function openAttachment(i) {
  try { await call('attachment_open', {att: attRef(i)}); }
  catch (e) { if (String(e).startsWith('RISKY|')) { toast(t('err.RISKY', {x: String(e).slice(6)}), true); saveAttachment(i); } else toast(errText(e), true); }
}
async function saveAttachment(i) {
  try { const p = await call('attachment_save', {att: attRef(i)}); if (p) toast(t('toast.saved', {x: p})); return !!p; }
  catch (e) { toast(errText(e), true); return false; }
}
function unsubscribe(link) {
  if (/^mailto:/i.test(link)) return composeMailto(link);
  call('open_url', {url: link}).catch(e => toast(errText(e), true));
}

/* ================= Actions on messages ================= */
const targets = () => sel.size ? [...sel] : cur ? [cur] : [];
function patch(uids, f) { for (const u of uids) { const m = byUid(u); if (m) f(m); } }
async function setFlag(uids, flag, on, quiet = false) {
  if (!uids.length) return;
  const a = acc(), folder = settings.folder;
  patch(uids, m => { m[flag] = on; });
  if (flag === 'seen') {
    const f = fl(); if (f && f.unseen != null) { f.unseen = Math.max(0, f.unseen + (on ? -1 : 1) * uids.length); renderFolders(); renderRail(); }
  }
  renderList(); if (flag === 'flagged') renderReader();
  try { await net(call('set_flag', {accountId: a.id, folder, uids, flag, on}), quiet); }
  catch (e) { patch(uids, m => { m[flag] = !on; }); renderList(); }
}
function removeFromList(uids) {
  const ids = visible().map(m => m.uid), i = ids.indexOf(cur);
  list = list.filter(m => !uids.includes(m.uid));
  if (page) page.total = Math.max(0, page.total - uids.length);
  if (uids.includes(cur)) {
    const rest = ids.filter(u => !uids.includes(u));
    const next = rest[Math.min(Math.max(0, i), rest.length - 1)];
    sel.clear(); cur = null; view = null;
    if (next != null) openMessage(next); else { renderList(); renderReader(); }
  } else { uids.forEach(u => sel.delete(u)); renderList(); }
}
async function toRole(r) {
  const uids = targets(); if (!uids.length) return;
  const a = acc(), folder = settings.folder;
  if (r === 'trash' && (role() === 'trash' || !(folders[a.id] || []).some(f => f.role === 'trash'))) {
    if (!await ask(t('ask.deleteNow', {n: uids.length}), t('ask.delete'))) return;
  }
  removeFromList(uids);
  try {
    const where = await net(call('move_to_role', {accountId: a.id, folder, uids, role: r}));
    toast(where ? t('toast.moved', {n: uids.length, f: folderName((folders[a.id] || []).find(f => f.id === where) || {name: where, role: ''})}) : t('toast.deleted', {n: uids.length}));
    showFolders();
  } catch (e) { refreshList(true); }
}
async function moveTo(uids, target) {
  if (!uids.length || target === settings.folder) return;
  const a = acc(), folder = settings.folder;
  removeFromList(uids);
  try {
    await net(call('move_messages', {accountId: a.id, folder, uids, target}));
    const f = (folders[a.id] || []).find(x => x.id === target);
    toast(t('toast.moved', {n: uids.length, f: f ? folderName(f) : target}));
    showFolders();
  } catch (e) { refreshList(true); }
}
function pickFolder() {
  const uids = targets(); if (!uids.length) return;
  const fs = (folders[settings.account] || []).filter(f => f.selectable && f.id !== settings.folder);
  modal(t('a.move'), `<div class="pickf">${fs.map(f => `<button class="fold" data-pick="${esc(f.id)}" style="padding-left:${8 + depth(f) * 14}px">${ICONS[ROLE_ICON[f.role] || 'folder']}<span class="n">${esc(folderName(f))}</span></button>`).join('')}</div>
    <p><button class="btn-sm" id="newFolder">${esc(t('a.newFolder'))}</button></p>`);
  $$('[data-pick]').forEach(b => b.onclick = () => { closeModal(); moveTo(uids, b.dataset.pick); });
  $('#newFolder').onclick = async () => { closeModal(); const name = await newFolder(); if (name) { const f = (folders[settings.account] || []).find(x => x.name === name); if (f) moveTo(uids, f.id); } };
}
function prompt1(title, label, value = '') {
  return new Promise(res => {
    modal(title, `<p>${esc(label)}</p><input class="field-in" id="p1" style="width:100%" value="${esc(value)}">`);
    const foot = $('#modalFoot'); foot.hidden = false;
    foot.innerHTML = `<span class="flex"></span><button class="btn-sm" id="p1c">${esc(t('ui.cancel'))}</button><button class="btn-sm primary" id="p1o">${esc(t('ui.ok'))}</button>`;
    let done = false;
    const end = v => { if (done) return; done = true; closeModal(); res(v); };
    $('#p1o').onclick = () => end($('#p1').value.trim()); $('#p1c').onclick = () => end('');
    $('#p1').onkeydown = e => { if (e.key === 'Enter') end($('#p1').value.trim()); };
    modalOnClose = () => end('');
    setTimeout(() => $('#p1').focus(), 30);
  });
}
async function newFolder() {
  const name = await prompt1(t('a.newFolder'), t('ui.folderName'));
  if (!name) return '';
  try { await net(call('create_folder', {accountId: settings.account, name})); await showFolders(); toast(t('toast.folderMade', {x: name})); return name; }
  catch (e) { return ''; }
}

/* ================= Compose ================= */
let draft = null, draftT;
const DRAFT_KEY = 'mail-studio.draft';
const sig = a => a && a.signature && a.signature.trim() ? `\n\n-- \n${a.signature.trim()}` : '';
function quote(v) {
  const body = (v.text && v.text.trim()) ? v.text : htmlText(v.html || '');
  return body.replace(/\r\n/g, '\n').trimEnd().split('\n').map(l => '> ' + l).join('\n');
}
function htmlText(h) { const d = new DOMParser().parseFromString(h, 'text/html'); d.querySelectorAll('style,script,head').forEach(n => n.remove()); return (d.body && d.body.innerText || d.body.textContent || '').replace(/\n{3,}/g, '\n\n'); }
const rePrefix = (s, p) => new RegExp('^\\s*(' + p + ')\\s*:', 'i');
function openCompose(init = {}) {
  if (!accounts.length) { toast(t('ui.needAccount')); return accountDialog(); }
  const a = accounts.find(x => x.id === init.accountId) || acc() || accounts[0];
  draft = Object.assign({accountId: a.id, to: '', cc: '', bcc: '', subject: '', body: sig(a), attachments: [], inReplyTo: '', references: [], draftUid: null,
    answerFolder: '', answerUid: null, showCc: false}, init);
  if (draft.cc || draft.bcc) draft.showCc = true;
  renderCompose();
  $('#composeBg').classList.add('show');
  setTimeout(() => (draft.to ? $('#cBody') : $('#cTo')).focus(), 30);
  if (draft.to && init.body === undefined) { const b = $('#cBody'); b.setSelectionRange(0, 0); }
}
function renderCompose() {
  const d = draft, box = $('#compose');
  const opt = a => `<option value="${a.id}"${a.id === d.accountId ? ' selected' : ''}>${esc(a.name ? `${a.name} <${a.email}>` : a.email)}</option>`;
  box.innerHTML = `<div class="c-head"><b>${esc(d.subject || t('ui.newMessage'))}</b><button class="ibtn" id="cClose" title="${esc(t('ui.close'))}">&#10005;</button></div>
    <div class="c-fields">
      <label>${esc(t('c.from'))}</label><select id="cFrom">${accounts.map(opt).join('')}</select><span></span>
      <label>${esc(t('c.to'))}</label><input class="field-in" id="cTo" value="${esc(d.to)}" spellcheck="false" autocomplete="off"><button class="lnk" id="cCcT">${esc(t('c.ccBcc'))}</button>
      ${d.showCc ? `<label>${esc(t('c.cc'))}</label><input class="field-in" id="cCc" value="${esc(d.cc)}" spellcheck="false"><span></span>
      <label>${esc(t('c.bcc'))}</label><input class="field-in" id="cBcc" value="${esc(d.bcc)}" spellcheck="false"><span></span>` : ''}
      <label>${esc(t('c.subject'))}</label><input class="field-in" id="cSubj" value="${esc(d.subject)}"><span></span>
    </div>
    <textarea class="c-body" id="cBody" spellcheck="true"></textarea>
    <div class="c-atts" id="cAtts"></div>
    <div class="c-foot"><button class="btn-sm primary" id="cSend">${esc(t('c.send'))}</button><span class="hint">${esc(fmtKey('Ctrl+Enter'))}</span>
      <span class="flex"></span><button class="btn-sm" id="cAttach">${esc(t('c.attach'))}</button><button class="btn-sm" id="cDraft">${esc(t('c.saveDraft'))}</button>
      <button class="btn-sm" id="cDiscard">${esc(t('c.discard'))}</button></div>`;
  $('#cBody').value = d.body;
  renderChips();
  const bind = (id, k) => { const n = $(id); if (n) n.oninput = () => { d[k] = n.value; if (k === 'subject') box.querySelector('.c-head b').textContent = n.value || t('ui.newMessage'); keepDraft(); }; };
  bind('#cTo', 'to'); bind('#cCc', 'cc'); bind('#cBcc', 'bcc'); bind('#cSubj', 'subject'); bind('#cBody', 'body');
  $('#cFrom').onchange = e => {
    const old = accounts.find(x => x.id === d.accountId), nu = accounts.find(x => x.id === e.target.value);
    if (old && nu && sig(old) && d.body.includes(sig(old))) d.body = d.body.replace(sig(old), sig(nu));
    d.accountId = e.target.value; $('#cBody').value = d.body; keepDraft();
  };
  $('#cCcT').onclick = () => { d.showCc = !d.showCc; renderCompose(); };
  $('#cClose').onclick = () => closeCompose(true);
  $('#cDiscard').onclick = () => discardCompose();
  $('#cSend').onclick = sendCompose;
  $('#cDraft').onclick = () => saveDraft(false);
  $('#cAttach').onclick = async () => { const f = await call('pick_attachments').catch(() => []); addFiles(f); };
}
function renderChips() {
  $('#cAtts').innerHTML = draft.attachments.map((f, i) => `<span class="chip">${esc(f.name)} <small>${fmtSize(f.size)}</small><button data-rm="${i}">&#10005;</button></span>`).join('');
  $$('#cAtts [data-rm]').forEach(b => b.onclick = () => { draft.attachments.splice(+b.dataset.rm, 1); renderChips(); keepDraft(); });
}
function addFiles(files) {
  for (const f of files || []) if (!draft.attachments.some(x => x.path === f.path)) draft.attachments.push(f);
  const total = draft.attachments.reduce((s, f) => s + f.size, 0);
  if (total > 20 * 1048576) toast(t('toast.bigAttachments', {x: fmtSize(total)}), true);
  renderChips(); keepDraft();
}
function keepDraft() { clearTimeout(draftT); draftT = setTimeout(() => { try { localStorage.setItem(DRAFT_KEY, JSON.stringify(draft)); } catch (e) {} }, 400); }
const composing = () => $('#composeBg').classList.contains('show');
const draftEmpty = d => !d || (!d.to.trim() && !d.cc.trim() && !d.subject.trim() && !d.attachments.length &&
  (!d.body.trim() || d.body.trim() === sig(accounts.find(a => a.id === d.accountId)).trim()));
async function closeCompose(askFirst) {
  if (askFirst && !draftEmpty(draft)) {
    const r = await choice(t('ask.closeDraft'), [['save', t('c.saveDraft')], ['discard', t('c.discard')], ['cancel', t('ui.cancel')]]);
    if (r === 'cancel' || !r) return;
    if (r === 'save') return saveDraft(true);
  }
  $('#composeBg').classList.remove('show'); draft = null;
  try { localStorage.removeItem(DRAFT_KEY); } catch (e) {}
}
async function discardCompose() {
  if (!draftEmpty(draft) && !await ask(t('ask.discard'), t('c.discard'))) return;
  closeCompose(false);
}
const payload = d => ({accountId: d.accountId, to: d.to, cc: d.cc, bcc: d.bcc, subject: d.subject, body: d.body, attachments: d.attachments.map(f => f.path),
  inReplyTo: d.inReplyTo, references: d.references, draftUid: d.draftUid, answerFolder: d.answerFolder, answerUid: d.answerUid});
async function sendCompose() {
  const d = draft; if (!d) return;
  if (!d.to.trim() && !d.cc.trim() && !d.bcc.trim()) { toast(t('err.NORCPT'), true); $('#cTo').focus(); return; }
  if (!d.subject.trim() && !await ask(t('ask.noSubject'), t('c.send'))) return;
  if (/\b(anbei|anhang|angehängt|attached|attachment)\b/i.test(d.body.split('\n-- \n')[0].split('\n').filter(l => !l.startsWith('>')).join('\n')) && !d.attachments.length &&
    !await ask(t('ask.noAttachment'), t('c.send'))) return;
  const btn = $('#cSend'); btn.disabled = true; btn.textContent = t('c.sending');
  try {
    const r = await net(call('send', {draft: payload(d)}));
    closeCompose(false);
    toast(r.copy.startsWith('failed') ? t('toast.sentNoCopy', {x: errText(r.copy.slice(8))}) : t('toast.sent'), r.copy.startsWith('failed'));
    if (d.answerUid && d.answerFolder === settings.folder) patch([d.answerUid], m => { m.answered = true; });
    if (role() === 'sent' || role() === 'drafts') refreshList(true); else renderList();
    showFolders();
  } catch (e) { btn.disabled = false; btn.textContent = t('c.send'); }
}
async function saveDraft(close) {
  const d = draft; if (!d) return;
  try {
    const uid = await net(call('save_draft', {draft: payload(d)}));
    d.draftUid = uid ?? null;
    toast(t('toast.draftSaved'));
    if (close) closeCompose(false); else keepDraft();
    if (role() === 'drafts') refreshList(true);
    showFolders();
  } catch (e) {}
}
function myAddresses() { return new Set(accounts.map(a => a.email.toLowerCase())); }
async function reply(all) {
  const v = view; if (!v) return;
  const mine = myAddresses(), a = accounts.find(x => x.id === v.account) || acc();
  let to = (v.replyTo.length ? v.replyTo : v.from);
  if (role() === 'sent' || to.every(p => mine.has(p.email.toLowerCase()))) to = v.to;   // answering your own message: the same people again
  let cc = [];
  if (all) {
    const seen = new Set(to.map(p => p.email.toLowerCase()));
    cc = [...v.to, ...v.cc].filter(p => { const e = p.email.toLowerCase(); if (mine.has(e) || seen.has(e)) return false; seen.add(e); return true; });
  }
  const from = v.from[0] || {email: ''};
  const head = t('c.wrote', {date: fmtDate(v.date, true), name: who(from)});
  openCompose({accountId: a.id, to: to.map(fullAddr).join(', '), cc: cc.map(fullAddr).join(', '),
    subject: rePrefix('', 're|aw|antw').test(v.subject) ? v.subject : (lang === 'de' ? 'AW: ' : 'Re: ') + v.subject,
    body: `\n${sig(a)}\n\n${head}\n${quote(v)}\n`, inReplyTo: v.messageId, references: v.references,
    answerFolder: v.folder, answerUid: v.uid});
  setTimeout(() => { const b = $('#cBody'); if (b) { b.setSelectionRange(0, 0); b.scrollTop = 0; b.focus(); } }, 40);
}
async function forward() {
  const v = view; if (!v) return;
  const a = accounts.find(x => x.id === v.account) || acc();
  const head = [`---------- ${t('c.fwdHead')} ----------`, `${t('rd.from')} ${v.from.map(fullAddr).join(', ')}`, `${t('c.date')} ${fmtDate(v.date, true)}`,
    `${t('c.subject')}: ${v.subject}`, `${t('rd.to')} ${v.to.map(fullAddr).join(', ')}`].join('\n');
  const body = (v.text && v.text.trim()) ? v.text : htmlText(v.html || '');
  let files = [];
  if (v.attachments.some(x => !x.inline)) files = await net(call('forward_files', {accountId: v.account, folder: v.folder, uid: v.uid, validity: page ? page.uidValidity : 0})).catch(() => []);
  openCompose({accountId: a.id, subject: rePrefix('', 'fwd|fw|wg').test(v.subject) ? v.subject : (lang === 'de' ? 'WG: ' : 'Fwd: ') + v.subject,
    body: `\n${sig(a)}\n\n${head}\n\n${body.trim()}\n`, attachments: files});
  setTimeout(() => { const b = $('#cBody'); if (b) b.setSelectionRange(0, 0); }, 40);
}
async function editDraft(uid = cur) {
  const m = byUid(uid); if (!m) return;
  const a = acc();
  try {
    const v = view && view.uid === uid ? view : await net(call('message_read', {accountId: a.id, folder: settings.folder, uid, validity: page ? page.uidValidity : 0}));
    let files = [];
    if (v.attachments.some(x => !x.inline)) files = await call('forward_files', {accountId: a.id, folder: settings.folder, uid, validity: page ? page.uidValidity : 0}).catch(() => []);
    openCompose({accountId: a.id, to: v.to.map(fullAddr).join(', '), cc: v.cc.map(fullAddr).join(', '), subject: v.subject,
      body: (v.text && v.text.trim()) ? v.text.replace(/\r\n/g, '\n') : htmlText(v.html || ''), attachments: files, draftUid: uid, references: v.references});
  } catch (e) {}
}
function composeMailto(href) {
  try {
    const u = new URL(href), q = u.searchParams;
    openCompose({to: decodeURIComponent(u.pathname || '') + (q.get('to') ? ', ' + q.get('to') : ''), cc: q.get('cc') || '', bcc: q.get('bcc') || '',
      subject: q.get('subject') || '', ...(q.get('body') ? {body: q.get('body') + sig(acc())} : {})});
  } catch (e) { openCompose({to: href.replace(/^mailto:/i, '')}); }
}
$('#compose').addEventListener('keydown', e => {
  if ((e.ctrlKey || e.metaKey) && e.key === 'Enter') { e.preventDefault(); sendCompose(); }
  else if (e.key === 'Escape') { e.preventDefault(); closeCompose(true); }
});
$('#composeBg').addEventListener('mousedown', e => { if (e.target.id === 'composeBg') closeCompose(true); });

/* ================= Accounts dialog ================= */
let modalOnClose = null;
function accountDialog(edit = null) {
  const a = Object.assign({id: '', name: '', email: '', user: '', imapHost: '', imapPort: 993, imapSecurity: 'tls', smtpHost: '', smtpPort: 587, smtpSecurity: 'starttls',
    signature: '', serverSavesSent: false, label: ''}, edit || {});
  let advanced = !!edit, note = '', preset = null, userTouched = !!edit, status = '';
  const sec = (k, v) => `<select data-f="${k}">${['tls', 'starttls', 'plain'].map(s => `<option value="${s}"${s === v ? ' selected' : ''}>${esc(t('sec.' + s))}</option>`).join('')}</select>`;
  const draw = () => {
    $('#modalTitle').textContent = edit ? t('acc.edit') : t('a.addAccount');
    $('#modalBody').innerHTML = `<div class="form">
      <label>${esc(t('acc.email'))}</label><input class="field-in" data-f="email" value="${esc(a.email)}" placeholder="name@icloud.com" spellcheck="false" autocomplete="off">
      <label>${esc(t('acc.name'))}</label><input class="field-in" data-f="name" value="${esc(a.name)}" placeholder="${esc(t('acc.namePh'))}">
      <label>${esc(t('acc.password'))}</label><input class="field-in" type="password" id="accPass" placeholder="${esc(edit && edit.hasPassword ? t('acc.passKeep') : '')}" autocomplete="off">
      ${note ? `<div class="full note${note === 'oauth' ? ' warn' : ''}">${t('note.' + note, {p: esc(preset && preset.provider || '')})}</div>` : ''}
      <label>${esc(t('acc.label'))}</label><input class="field-in" data-f="label" value="${esc(a.label)}" placeholder="${esc(t('acc.labelPh'))}">
      <label>${esc(t('acc.signature'))}</label><textarea data-f="signature">${esc(a.signature)}</textarea>
      <div class="full"><button class="lnk btn-sm" id="accAdv">${esc(advanced ? t('acc.hideServers') : t('acc.servers'))}</button></div>
      ${advanced ? `
      <label>${esc(t('acc.user'))}</label><input class="field-in" data-f="user" value="${esc(a.user)}" spellcheck="false">
      <label>IMAP</label><div class="row2"><input class="field-in" data-f="imapHost" value="${esc(a.imapHost)}" spellcheck="false"><input class="field-in" data-f="imapPort" value="${a.imapPort}">${sec('imapSecurity', a.imapSecurity)}</div>
      <label>SMTP</label><div class="row2"><input class="field-in" data-f="smtpHost" value="${esc(a.smtpHost)}" spellcheck="false"><input class="field-in" data-f="smtpPort" value="${a.smtpPort}">${sec('smtpSecurity', a.smtpSecurity)}</div>
      <span></span><label class="switch"><input type="checkbox" data-f="serverSavesSent"${a.serverSavesSent ? ' checked' : ''}><span></span>${esc(t('acc.serverSaves'))}</label>` : ''}
      ${status ? `<div class="full note ${status.startsWith('ok:') ? '' : 'err'}">${esc(status.replace(/^(ok|err):/, ''))}</div>` : ''}
    </div>`;
    const foot = $('#modalFoot'); foot.hidden = false;
    foot.innerHTML = `${edit ? `<button class="btn-sm" id="accRemove">${esc(t('acc.remove'))}</button>` : ''}<span class="flex"></span>
      <button class="btn-sm" id="accTest">${esc(t('acc.test'))}</button>${status.startsWith('err:') ? `<button class="btn-sm" id="accForce">${esc(t('acc.saveAnyway'))}</button>` : ''}
      <button class="btn-sm primary" id="accSave">${esc(t('acc.save'))}</button>`;
    $$('#modalBody [data-f]').forEach(n => {
      const k = n.dataset.f;
      n.oninput = n.onchange = () => {
        a[k] = n.type === 'checkbox' ? n.checked : /Port$/.test(k) ? (parseInt(n.value, 10) || 0) : n.value;
        if (k === 'user') userTouched = true;
        if (k === 'email' && !userTouched) a.user = a.email.trim();
      };
    });
    const em = $('[data-f="email"]');
    em.onchange = async () => {
      a.email = em.value.trim(); if (!userTouched) a.user = a.email;
      preset = await call('account_preset', {email: a.email}).catch(() => null);
      if (preset && !edit) {
        Object.assign(a, {imapHost: preset.imapHost, imapPort: preset.imapPort, imapSecurity: preset.imapSecurity, smtpHost: preset.smtpHost,
          smtpPort: preset.smtpPort, smtpSecurity: preset.smtpSecurity, serverSavesSent: preset.serverSavesSent});
        if (!a.label && preset.provider) a.label = preset.provider;
        if (preset.note === 'guess') advanced = true;
      }
      note = preset ? preset.note : '';
      const pass = $('#accPass').value; draw(); $('#accPass').value = pass;
    };
    $('#accAdv').onclick = () => { const pass = $('#accPass').value; advanced = !advanced; draw(); $('#accPass').value = pass; };
    const go = async (saveIt, test) => {
      const pass = $('#accPass').value;
      if (!a.name.trim()) a.name = a.email.split('@')[0];
      if (!edit && !pass) { status = 'err:' + t('acc.needPass'); draw(); return; }
      if (test) {
        $('#accTest').disabled = $('#accSave').disabled = true; $('#accTest').textContent = t('acc.testing');
        try { await net(call('account_test', {account: a, password: pass || null}), true); status = 'ok:' + t('acc.testOk'); }
        catch (e) { status = 'err:' + errText(e); draw(); $('#accPass').value = pass; return; }
      }
      if (!saveIt) { draw(); $('#accPass').value = pass; return; }
      try {
        const saved = await call('account_save', {account: a, password: pass || null});
        closeModal();
        delete folders[saved.id];
        await loadAccounts();
        if (!edit || settings.account === saved.id) { settings.account = saved.id; settings.folder = 'INBOX'; save(); renderRail(); await showFolders(); openFolder('INBOX'); }
        toast(edit ? t('toast.accSaved') : t('toast.accAdded', {x: saved.email}));
      } catch (e) { status = 'err:' + errText(e); draw(); $('#accPass').value = pass; }
    };
    $('#accTest').onclick = () => go(false, true);
    $('#accSave').onclick = () => go(true, true);
    if ($('#accForce')) $('#accForce').onclick = () => go(true, false);
    if ($('#accRemove')) $('#accRemove').onclick = async () => {
      if (!await ask(t('ask.removeAccount', {x: a.email}), t('acc.remove'))) return;
      await call('account_remove', {id: a.id}).catch(e => toast(errText(e), true));
      closeModal(); delete folders[a.id];
      if (settings.account === a.id) { settings.account = ''; page = null; list = []; cur = null; view = null; }
      await loadAccounts(); await showFolders(false); if (acc()) openFolder('INBOX'); else { renderList(); renderReader(); }
    };
  };
  $('.modal').classList.add('wide'); settingsOpen = false; modalOnClose = null;
  draw(); $('#modal').classList.add('show');
  setTimeout(() => $('[data-f="email"]').focus(), 30);
}

/* ================= Modal, questions, settings ================= */
let settingsOpen = false, prefsSection = 'general';
function modal(title, html) {
  $('.modal').classList.remove('wide'); $('#modalFoot').hidden = true; settingsOpen = false; modalOnClose = null;
  $('#modalTitle').textContent = title; $('#modalBody').innerHTML = html; $('#modal').classList.add('show');
}
function closeModal() {
  $('#modal').classList.remove('show'); settingsOpen = false;
  const f = modalOnClose; modalOnClose = null; if (f) f();
}
$('#modalClose').onclick = closeModal;
$('#modal').onmousedown = e => { if (e.target.id === 'modal') closeModal(); };
function choice(message, options) {
  return new Promise(res => {
    modal(t('ui.question'), `<p>${esc(message)}</p>`);
    const foot = $('#modalFoot'); foot.hidden = false;
    foot.innerHTML = '<span class="flex"></span>' + options.map(([id, label], i) => `<button class="btn-sm${i === 0 ? ' primary' : ''}" data-ch="${id}">${esc(label)}</button>`).join('');
    let done = false;
    const end = v => { if (done) return; done = true; closeModal(); res(v); };
    $$('[data-ch]').forEach(b => b.onclick = () => end(b.dataset.ch));
    modalOnClose = () => end(null);
  });
}
async function ask(message, ok) { return (await choice(message, [['ok', ok], ['cancel', t('ui.cancel')]])) === 'ok'; }
function openSettings(section) {
  if (section) prefsSection = section;
  $('.modal').classList.add('wide'); $('#modalFoot').hidden = false; modalOnClose = null;
  settingsOpen = true; renderSettings(); $('#modal').classList.add('show');
}
function renderSettings() {
  const opt = (v, label, c) => `<option value="${v}"${String(v) === String(c) ? ' selected' : ''}>${esc(label)}</option>`;
  const sw = (name, label) => `<label class="switch"><input type="checkbox" data-sw="${name}"${settings[name] ? ' checked' : ''}><span></span>${esc(label)}</label>`;
  const mock = th => `<div class="tc-mock" data-theme="${th}"><div class="tc-bar"><i></i><i></i><i></i><b></b></div>
    <div class="tc-body" style="grid-template-columns:14px 30px 1fr 1fr"><div class="tc-rail"></div><div class="tc-dock" style="border-left:0;border-right:1px solid var(--line)"></div>
    <div class="tc-ed"><i></i><i style="width:70%"></i><i></i><i style="width:60%"></i></div><div class="tc-pv"><div class="tc-paper"><i></i><i style="width:70%"></i><i style="width:50%"></i></div></div></div></div>`;
  const body = {
    general: `<h3>${esc(t('set.language'))}</h3><p class="hint">${esc(t('set.languageHint'))}</p>
      <select class="wide-sel" data-sel="lang">${opt('auto', t('a.langAuto'), settings.lang)}${opt('en', 'English', settings.lang)}${opt('de', 'Deutsch', settings.lang)}</select>
      <h3>${esc(t('set.interval'))}</h3><select class="wide-sel" data-sel="interval">${[1, 2, 5, 10, 15, 30, 0].map(n => opt(n, n ? t('set.minutes', {n}) : t('set.manual'), settings.interval)).join('')}</select>
      <h3>${esc(t('set.markRead'))}</h3><select class="wide-sel" data-sel="markRead">${['open', 'delay', 'never'].map(v => opt(v, t('set.markRead.' + v), settings.markRead)).join('')}</select>
      <h3>${esc(t('set.images'))}</h3><p class="hint">${esc(t('set.imagesHint'))}</p><select class="wide-sel" data-sel="images">${['ask', 'always'].map(v => opt(v, t('set.images.' + v), settings.images)).join('')}</select>
      ${settings.trusted.length ? `<p class="hint">${esc(t('set.trusted', {n: settings.trusted.length}))} <button class="btn-sm" data-pref="untrust">${esc(t('set.untrust'))}</button></p>` : ''}
      <h3>${esc(t('set.layout'))}</h3>${sw('autoHide', t('set.autoHide'))}`,
    accounts: `<h3>${esc(t('set.accounts'))}</h3><p class="hint">${esc(t('set.accountsHint'))}</p><div class="alist">${accounts.map(a =>
      `<div class="aitem"><span class="acct-dot" style="background:${colorOf(a.email)}">${esc(initials(a))}</span><div><b>${esc(a.label || a.name || a.email)}</b><small>${esc(a.email)} · ${esc(a.imapHost)}${a.hasPassword ? '' : ' · ' + esc(t('acc.noPass'))}</small></div>
       <button class="btn-sm" data-edit-acc="${a.id}">${esc(t('acc.editShort'))}</button></div>`).join('') || `<p class="hint">${esc(t('ui.noAccounts'))}</p>`}</div>
      <p><button class="btn-sm primary" data-pref="addAccount">${esc(t('a.addAccount'))}</button></p>`,
    appearance: (window.Shared ? Shared.scopePicker() : '') + `<h3>${esc(t('set.theme'))}</h3><div class="theme-grid">` + THEMES.map(th =>
      `<button class="theme-card${settings.theme === th ? ' active' : ''}" data-theme-pick="${th}">${mock(th)}
       <div class="theme-name">${esc(t('theme.' + th))}<span>${settings.theme === th ? '&#10003;' : ''}</span></div></button>`).join('') + '</div>' + (window.Shared ? Shared.layoutHeading() : ''),
    keys: '<div id="keysBox"></div>'
  };
  const sections = ['general', 'accounts', 'appearance', 'keys'];
  $('#modalTitle').textContent = t('set.title');
  $('#modalBody').innerHTML = `<div class="prefs"><nav>${sections.map(k =>
    `<button data-sec="${k}" class="${k === prefsSection ? 'active' : ''}">${esc(t('set.' + k))}</button>`).join('')}</nav><section>${body[prefsSection]}</section></div>`;
  if (prefsSection === 'keys') Shortcuts.editor($('#keysBox'), {MENUS, keys: settings.keys || {}, t, fmtKey, comboOf, label: a => t('a.' + a),
    onChange: keys => { settings.keys = keys; applySettings(); }});
  $('#modalFoot').innerHTML = `<button class="btn-sm" data-pref="reset">${esc(t('set.reset'))}</button><span class="flex"></span>
    <button class="btn-sm primary" data-pref="done">${esc(t('set.done'))}</button>`;
}
$('#modal').addEventListener('click', e => {
  if (!settingsOpen) return;
  const x = e.target.closest('[data-sec],[data-theme-pick],[data-pref],[data-edit-acc]'); if (!x) return;
  if (x.dataset.editAcc) { const a = accounts.find(y => y.id === x.dataset.editAcc); settingsOpen = false; return accountDialog(a); }
  if (x.dataset.sec) prefsSection = x.dataset.sec;
  else if (x.dataset.themePick) settings.theme = x.dataset.themePick;
  else if (x.dataset.pref === 'done') return closeModal();
  else if (x.dataset.pref === 'addAccount') { settingsOpen = false; return accountDialog(); }
  else if (x.dataset.pref === 'untrust') settings.trusted = [];
  else if (x.dataset.pref === 'reset') { const keep = {account: settings.account, folder: settings.folder}; settings = {...DEF, ...keep, trusted: []}; toast(t('set.resetDone')); }
  applySettings(); renderSettings();
});
$('#modal').addEventListener('change', e => {
  if (!settingsOpen) return;
  const d = e.target.dataset;
  if (d.sel) { settings[d.sel] = d.sel === 'interval' ? +e.target.value : e.target.value; applySettings(); renderSettings(); }
  if (d.sw) { settings[d.sw] = e.target.checked; applySettings(); }
});
function setLang(l) { settings.lang = l; applySettings(); }
function setTheme(th) { settings.theme = th; applySettings(); }

/* ================= Actions, menus, shortcuts ================= */
const A = {
  compose: () => openCompose(), refresh: () => syncNow(), addAccount: () => accountDialog(), editAccount: () => accountDialog(acc()),
  reply: () => reply(false), replyAll: () => reply(true), forward, editDraft: () => editDraft(),
  archive: () => toRole('archive'), junk: () => toRole('junk'), delete: () => toRole('trash'), move: pickFolder,
  toggleRead: () => { const u = targets(), all = u.every(x => (byUid(x) || {}).seen); setFlag(u, 'seen', !all); },
  toggleFlag: () => { const u = targets(), all = u.every(x => (byUid(x) || {}).flagged); setFlag(u, 'flagged', !all); },
  selectAll: () => { sel = new Set(visible().map(m => m.uid)); renderList(); renderReader(); },
  find: () => { $('#search').focus(); $('#search').select(); },
  next: () => step(1), prev: () => step(-1), newFolder,
  source: async () => { if (!view) return; const s = await net(call('message_source', {accountId: view.account, folder: view.folder, uid: view.uid, validity: page ? page.uidValidity : 0})).catch(() => null); if (s != null) modal(t('a.source'), `<pre class="src">${esc(s)}</pre>`); },
  saveEml: async () => { if (!view) return; const p = await call('message_save', {accountId: view.account, folder: view.folder, uid: view.uid, validity: page ? page.uidValidity : 0, name: view.subject || 'message'}).catch(e => { toast(errText(e), true); return null; }); if (p) toast(t('toast.saved', {x: p})); },
  toggleAutoHide: () => { settings.autoHide = !settings.autoHide; applySettings(); },
  settings: () => openSettings(), accounts: () => openSettings('accounts'),
  langAuto: () => setLang('auto'), langEn: () => setLang('en'), langDe: () => setLang('de'),
  shortcuts: () => {
    let h = '';
    MENUS.forEach(([m, items]) => {
      const rows = items.filter(it => Array.isArray(it) && it[1]);
      if (rows.length) h += `<h4>${esc(t('menu.' + m))}</h4><table>` + rows.map(it => `<tr><td>${esc(t('a.' + it[0]))}</td><td><kbd>${esc(fmtKey(it[1]))}</kbd></td></tr>`).join('') + '</table>';
    });
    h += `<h4>${esc(t('menu.compose'))}</h4><table><tr><td>${esc(t('c.send'))}</td><td><kbd>${esc(fmtKey('Ctrl+Enter'))}</kbd></td></tr><tr><td>${esc(t('ui.close'))}</td><td><kbd>Esc</kbd></td></tr></table>
      <h4>${esc(t('menu.list'))}</h4><table><tr><td>${esc(t('ed.nav'))}</td><td><kbd>↑</kbd> <kbd>↓</kbd> / <kbd>J</kbd> <kbd>K</kbd></td></tr><tr><td>${esc(t('a.delete'))}</td><td><kbd>Entf</kbd> / <kbd>⌫</kbd></td></tr></table>`;
    modal(t('a.shortcuts'), h);
  },
  about: async () => {
    const v = invoke ? await window.__TAURI__.app.getVersion().catch(() => '') : '';
    modal(t('a.about'), `${v ? `<p><b>${esc(t('about.version', {v}))}</b></p>` : ''}<p>${esc(t('about.p1'))}</p><p>${esc(t('about.p2'))}</p><p>${esc(t('about.p3'))}</p>`);
  }
};
const THEME_ACTS = {themeDark: 'dark', themeLight: 'light', themeGithub: 'github', themeGithubDark: 'github-dark', themeCli: 'cli', themeCliAmber: 'cli-amber', themeCliCyan: 'cli-cyan',
  themeCliMagenta: 'cli-magenta', themeCliWhite: 'cli-white', themeMidnight: 'midnight', themeDusk: 'dusk', themeGraphite: 'graphite', themeContrast: 'contrast', themeSand: 'sand', themeRose: 'rose'};
for (const [k, v] of Object.entries(THEME_ACTS)) A[k] = () => setTheme(v);
// Menu items: [action, shortcut, checked(), fixed, extraKeys]; '-' is a separator, '#key' a section label (as in every Studio app).
const MENUS = [
  ['file', [['compose', 'Ctrl+N'], ['refresh', 'F5', null, false, ['Ctrl+Shift+N']], '-', ['saveEml', 'Ctrl+S'], ['source', 'Ctrl+U'], '-', ['addAccount'], ['editAccount'], ['newFolder']]],
  ['edit', [['selectAll', 'Ctrl+A'], ['find', 'Ctrl+F']]],
  ['message', [['reply', 'Ctrl+R'], ['replyAll', 'Ctrl+Shift+R'], ['forward', 'Ctrl+Shift+F'], '-', ['toggleRead', 'Ctrl+Shift+U'], ['toggleFlag', 'Ctrl+Shift+L'], '-',
    ['archive', 'Ctrl+E'], ['junk', 'Ctrl+Shift+J'], ['delete', 'Delete', null, true], ['move', 'Ctrl+Shift+M'], '-', ['next', 'Alt+ArrowDown'], ['prev', 'Alt+ArrowUp']]],
  ['view', [['toggleAutoHide', null, () => settings.autoHide]]],
  ['settings', [['settings', 'Ctrl+,'], ['accounts'], '-', '#menu.language',
    ['langAuto', null, () => settings.lang === 'auto'], ['langEn', null, () => settings.lang === 'en'], ['langDe', null, () => settings.lang === 'de'],
    '-', '#menu.theme', ...Object.entries(THEME_ACTS).map(([k, v]) => [k, null, () => settings.theme === v])]],
  ['help', [['shortcuts', 'F1'], '-', ['about']]]
];
const NEEDS_MSG = new Set(['reply', 'replyAll', 'forward', 'source', 'saveEml', 'editDraft']);
const NEEDS_SEL = new Set(['archive', 'junk', 'delete', 'move', 'toggleRead', 'toggleFlag']);
const NEEDS_ACC = new Set(['refresh', 'editAccount', 'newFolder', 'selectAll', 'find', 'next', 'prev', 'compose']);
const allowed = act => (!NEEDS_ACC.has(act) || !!acc()) && (!NEEDS_MSG.has(act) || (!!view && view.uid === cur)) && (!NEEDS_SEL.has(act) || targets().length > 0) &&
  (act !== 'editDraft' || role() === 'drafts');
function run(act) { if (A[act] && allowed(act)) A[act](); }
function syncDisabled() { $$('[data-act]').forEach(b => { if (b.closest('#modal')) return; b.disabled = !allowed(b.dataset.act); }); }
const KEYMAP = {};
function buildKeymap() {
  for (const k in KEYMAP) delete KEYMAP[k];
  MENUS.forEach(([, items]) => items.forEach(it => { if (Array.isArray(it) && !it[3]) [it[1], ...(it[4] || [])].forEach(k => { if (k) KEYMAP[k] = it[0]; }); }));
}
let menuOpen = null;
function openMenu(m, items) {
  closeMenus();
  const dd = m.querySelector('.dropdown'); dd.innerHTML = '';
  items.forEach(it => {
    if (it === '-') return dd.append(document.createElement('hr'));
    if (typeof it === 'string') return dd.append(el('div', {className: 'dd-label', textContent: t(it.slice(1))}));
    const [act, key, check] = it, b = document.createElement('button');
    b.innerHTML = `<span class="chk">${check && check() ? '&#10003;' : ''}</span><span>${esc(t('a.' + act))}</span><kbd>${esc(key ? fmtKey(key.replace('ArrowDown', '↓').replace('ArrowUp', '↑')) : '')}</kbd>`;
    b.disabled = !allowed(act);
    b.onmousedown = e => e.preventDefault();
    b.onclick = () => { closeMenus(); run(act); };
    dd.append(b);
  });
  m.classList.add('open'); menuOpen = m;
}
function closeMenus() { $$('.menu.open').forEach(x => x.classList.remove('open')); menuOpen = null; }
function buildMenubar() {
  const nav = $('#menus'); nav.innerHTML = ''; menuOpen = null;
  MENUS.forEach(([name, items]) => {
    const m = el('div', {className: 'menu'}), b = el('button', {textContent: t('menu.' + name)}), dd = el('div', {className: 'dropdown'});
    b.onmousedown = e => e.preventDefault();
    b.onclick = () => menuOpen === m ? closeMenus() : openMenu(m, items);
    b.onmouseenter = () => { if (menuOpen && menuOpen !== m) openMenu(m, items); };
    m.append(b, dd); nav.append(m);
  });
}
document.addEventListener('mousedown', e => { if (menuOpen && !e.target.closest('.menu')) closeMenus(); });
const keyOf = act => { for (const [, items] of MENUS) for (const it of items) if (Array.isArray(it) && it[0] === act && it[1]) return it[1]; return ''; };
const tip = act => t('a.' + act) + (keyOf(act) ? ` (${fmtKey(keyOf(act))})` : '');
function buildToolbars() {
  const btn = act => `<button class="ibtn" data-act="${act}" title="${esc(tip(act))}">${ICONS[{toggleRead: 'unread', toggleFlag: 'flag', delete: 'delete'}[act] || act]}</button>`;
  $('#mainBtns').innerHTML = btn('refresh') + btn('compose');
  $('#msgBtns').innerHTML = ['reply', 'replyAll', 'forward'].map(btn).join('') + '<span class="sep" style="width:1px;height:18px;background:var(--border);margin:0 4px"></span>' +
    ['archive', 'junk', 'delete'].map(btn).join('') + '<span class="sep" style="width:1px;height:18px;background:var(--border);margin:0 4px"></span>' + ['toggleRead', 'toggleFlag', 'move'].map(btn).join('');
  syncDisabled();
}
document.addEventListener('click', e => {
  const b = e.target.closest('[data-act]');
  if (b && !b.closest('.dropdown')) { run(b.dataset.act); }
});

function applyLanguage() {
  document.documentElement.lang = lang;
  $$('[data-i18n]').forEach(n => n.textContent = t(n.dataset.i18n));
  $$('[data-i18n-ph]').forEach(n => n.placeholder = t(n.dataset.i18nPh));
  $$('[data-i18n-title]').forEach(n => n.title = t(n.dataset.i18nTitle));
  buildMenubar(); buildToolbars();
  if (booted) { renderRail(); renderFolders(); renderList(); renderReader(); busy(0); }
}
let appliedKeys = null;
function applySettings() {
  if (window.AutoHide) AutoHide.set(settings.autoHide);
  if (!THEMES.includes(settings.theme)) settings.theme = 'dark';
  if (window.Shared) Shared.push({theme: settings.theme, lang: settings.lang});
  document.documentElement.dataset.theme = settings.theme;
  if (JSON.stringify(settings.keys || {}) !== appliedKeys) {
    appliedKeys = JSON.stringify(settings.keys || {});
    Shortcuts.apply(MENUS, settings.keys || {}); buildKeymap(); buildToolbars();
  }
  const nl = effectiveLang();
  if (nl !== lang) { lang = nl; applyLanguage(); }
  $('#split').style.setProperty('--list-w', settings.listW + 'px');
  schedule(); save();
}

/* ================= Keyboard ================= */
function comboOf(e) {
  let k = e.key;
  if (/^Digit\d$/.test(e.code)) k = e.code.slice(5);
  else if (e.code === 'Comma') k = ',';
  else if (e.code === 'Period') k = '.';
  else if (k.length === 1) k = k.toUpperCase();
  return (e.ctrlKey || e.metaKey ? 'Ctrl+' : '') + (e.shiftKey ? 'Shift+' : '') + (e.altKey ? 'Alt+' : '') + k;
}
const FIELD_KEYS = new Set(['Ctrl+Z', 'Ctrl+Y', 'Ctrl+Shift+Z', 'Ctrl+X', 'Ctrl+C', 'Ctrl+V', 'Ctrl+A', 'Delete', 'Backspace', 'Ctrl+Shift+F', 'Ctrl+U']);
window.addEventListener('keydown', e => {
  if (window.Shortcuts && Shortcuts.recording) return;
  const k = e.key;
  if (k === 'Escape') {
    if ($('#modal').classList.contains('show')) closeModal();
    else if (menuOpen) closeMenus();
    else if (document.activeElement === $('#search') && $('#search').value) { $('#search').value = ''; runSearch(''); }
    return;
  }
  if (composing() || $('#modal').classList.contains('show')) return;
  if (e.ctrlKey && e.altKey) return;
  const combo = comboOf(e), inField = Native.isField(e.target) || e.target.isContentEditable || e.target.tagName === 'SELECT';
  if (inField && FIELD_KEYS.has(combo)) return;
  const act = KEYMAP[combo];
  if (act) { e.preventDefault(); e.stopPropagation(); run(act); return; }
  if (inField) return;
  if (k === 'Delete' || k === 'Backspace') { e.preventDefault(); run('delete'); }
  else if (k === 'ArrowDown' || k === 'j') { e.preventDefault(); step(1); }
  else if (k === 'ArrowUp' || k === 'k') { e.preventDefault(); step(-1); }
  else if (k === 'Enter' && role() === 'drafts' && cur) { e.preventDefault(); editDraft(); }
}, true);
$('#search').addEventListener('keydown', e => { if (e.key === 'Enter') { e.preventDefault(); runSearch($('#search').value); } });
$('#search').addEventListener('input', () => { if (!$('#search').value && search) runSearch(''); else if (!search) renderList(); });

/* ================= Native right-click menus (see native.js) ================= */
{
  const item = act => ({id: act, label: t('a.' + act), accel: keyOf(act) || null, enabled: allowed(act)});
  let ctxAcct = null, ctxFolder = null;
  Native.setProvider(e => {
    const row = e.target.closest('.row');
    if (row) {
      const uid = +row.dataset.uid;
      if (!sel.has(uid)) openMessage(uid);
      return [item('reply'), item('replyAll'), item('forward'), null, item('toggleRead'), item('toggleFlag'), null, item('archive'), item('junk'), item('move'), item('delete')];
    }
    const ac = e.target.closest('.acct');
    if (ac) { ctxAcct = ac.dataset.acct; return [{id: 'acct:edit', label: t('acc.edit')}]; }
    const f = e.target.closest('.fold[data-folder]');
    if (f) { ctxFolder = f.dataset.folder; return [{id: 'fold:open', label: t('ui.openFolder')}, {id: 'fold:read', label: t('a.markFolderRead')}, null, item('newFolder')]; }
    return null;
  });
  Native.setHandler(id => {
    if (id === 'acct:edit') return accountDialog(accounts.find(a => a.id === ctxAcct));
    if (id === 'fold:open') return openFolder(ctxFolder);
    if (id === 'fold:read') return markFolderRead(ctxFolder);
    run(id);
  });
}
async function markFolderRead(id) {
  if (id !== settings.folder) await openFolder(id);
  const u = list.filter(m => !m.seen).map(m => m.uid);
  if (u.length) setFlag(u, 'seen', true);
}

/* ================= List width, sync timer ================= */
{
  const g = $('#gutter');
  g.addEventListener('mousedown', e => {
    e.preventDefault();
    const x0 = e.clientX, w0 = settings.listW;
    const mv = ev => { settings.listW = Math.max(280, Math.min(900, w0 + ev.clientX - x0)); $('#split').style.setProperty('--list-w', settings.listW + 'px'); };
    const up = () => { removeEventListener('mousemove', mv); removeEventListener('mouseup', up); save(); };
    addEventListener('mousemove', mv); addEventListener('mouseup', up);
  });
}
let syncT = null, syncing = false;
function schedule() {
  clearInterval(syncT);
  if (settings.interval > 0) syncT = setInterval(() => syncNow(true), settings.interval * 60000);
}
async function syncNow(quiet = false) {
  if (syncing || !acc()) return;
  syncing = true;
  try {
    const before = ((folders[settings.account] || []).find(f => f.role === 'inbox') || {}).unseen || 0;
    await Promise.all([showFolders(), search ? null : refreshList(quiet)]);
    const after = ((folders[settings.account] || []).find(f => f.role === 'inbox') || {}).unseen || 0;
    if (quiet && after > before && !document.hasFocus()) toast(t('toast.newMail', {n: after - before}));
    // The other accounts: only their counts (the badges in the rail).
    for (const a of accounts) if (a.id !== settings.account) call('folders', {accountId: a.id}).then(f => { folders[a.id] = f; renderRail(); }).catch(() => {});
  } finally { syncing = false; }
}
window.addEventListener('focus', () => { if (lastSync && Date.now() - lastSync > 60000) syncNow(true); });

/* ================= Drag and drop of files onto the compose window ================= */
if (invoke && window.__TAURI__.webview) {
  window.__TAURI__.webview.getCurrentWebview().onDragDropEvent(async ev => {
    const p = ev.payload;
    if (!composing() || !draft) return;
    $('#compose').classList.toggle('dragover', p.type === 'over' || p.type === 'enter');
    if (p.type === 'drop') { $('#compose').classList.remove('dragover'); addFiles(await call('file_infos', {paths: p.paths}).catch(() => [])); }
  }).catch(() => {});
}

/* Test hook (like the other Studio apps). */
window.__ml = {A, run, get list() { return list; }, get view() { return view; }, get draft() { return draft; }, get settings() { return settings; },
  openCompose, openMessage, reply, forward, errText, fmtDate, htmlText, quote, frameHtml, REMOTE};

/* ================= Boot ================= */
(async function boot() {
  await loadTexts();
  let saved = null;
  try { saved = JSON.parse(localStorage.getItem(STORE)); } catch (e) {}
  if (saved && saved.settings) settings = {...DEF, ...saved.settings};
  if (window.Shared) await Shared.init({app: 'ml', apply: sh => {
    if (sh.theme) settings.theme = sh.theme;
    if (sh.lang) settings.lang = sh.lang;
    if (booted) applySettings();
  }});
  lang = effectiveLang();
  document.documentElement.dataset.theme = THEMES.includes(settings.theme) ? settings.theme : 'dark';
  applyLanguage();
  booted = true;
  applySettings();
  await loadAccounts();
  renderReader();
  if (acc()) { await showFolders(false); await openFolder(settings.folder || 'INBOX'); showFolders(); }
  else renderList();
  // A draft that was open when the app was closed.
  try { const d = JSON.parse(localStorage.getItem(DRAFT_KEY)); if (d && !draftEmpty(d) && accounts.some(a => a.id === d.accountId)) { openCompose(d); toast(t('toast.draftRestored')); } } catch (e) {}
  if (invoke) {
    call('initial_mailto').then(m => { if (m) composeMailto(m); }).catch(() => {});
    window.__TAURI__.event.listen('mailto', e => composeMailto(String(e.payload)));
  }
})();
