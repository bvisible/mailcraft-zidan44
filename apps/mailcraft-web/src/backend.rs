//! The browser's network side: the engine's jobs run over JMAP and `fetch`, one account's at a
//! time in order (as the desktop's worker threads do), and their events wait for the next frame.
//! New mail comes by push: the session's event stream, read through `fetch` (an `EventSource`
//! can't send the `Authorization` header), with polling as the fallback.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;

use mailcraft_engine::model::{AccountConfig, Event, Folder, FolderRole, Job, Op, SyncStatus, sort_folders};
use mailcraft_engine::store::Store;
use mailcraft_jmap::{Client, Error, State, SyncOptions};
use mailcraft_ui_egui::Backend;

use crate::fetch::{Fetch, events, sleep};

/// How often the Inbox is checked for new mail, and every how many checks everything is.
const POLL_MS: i32 = 120_000;
const FULL_EVERY: u32 = 8;
/// How long before a push stream that ended is opened again (and how often a stream waits for
/// its account to connect).
const PUSH_RETRY_MS: i32 = 30_000;
const PUSH_WAIT_MS: i32 = 3_000;
/// Largest picture downloaded for a message.
const MAX_PICTURE: usize = 8 << 20;

/// What an account's runner does next.
enum Task {
    Connect,
    SyncAll,
    SyncFolder(String),
    Job(Job),
}

struct Account {
    config: AccountConfig,
    /// Kept for this page only.
    password: Option<String>,
    client: Option<Rc<Client<Fetch>>>,
    state: State,
    folders: Vec<Folder>,
    /// Changes made while signed out, replayed on connecting.
    ops: Vec<Op>,
    queue: VecDeque<Task>,
    running: bool,
    /// Bumped on every start, so an old poller stops.
    generation: u32,
    /// The push stream is open: the Inbox needn't be polled.
    pushing: bool,
    // //// Neoffice — a new token was asked of the hosting page after a refusal: once, until a sign-in works (renew).
    renewing: bool,
    // //// Neoffice — when the address books and calendars were last read (pim), so a sync of the mail reads them at most
    // every CONTACTS_EVERY seconds.
    contacts_at: Option<i64>,
}

// //// Neoffice — the address books and calendars are read again with the mail, at most this often (seconds).
const CONTACTS_EVERY: i64 = 600;

struct Shared {
    store: Store,
    events: Vec<Event>,
    accounts: HashMap<String, Account>,
    ctx: egui::Context,
    opts: SyncOptions,
    // //// Neoffice — what the mailboxes a hosting page handed over sign in with: address (lower case) →
    // (secret, Bearer); in memory only (host.rs).
    host: HashMap<String, (String, bool)>,
    // //// Neoffice — the accounts waiting for new tokens while one request to the hosting page is on its way (renew):
    // several mailboxes refused at once ask the page once.
    renewal: Option<Vec<String>>,
}

type Rcs = Rc<RefCell<Shared>>;

pub struct WebBackend {
    shared: Rcs,
}

impl WebBackend {
    // //// Neoffice — `host`: the secrets of the mailboxes a hosting page handed over (host.rs).
    pub fn new(store: Store, ctx: egui::Context, host: HashMap<String, (String, bool)>) -> WebBackend {
        let opts = SyncOptions { window: 300, prefetch: 20, ..SyncOptions::default() };
        WebBackend { shared: Rc::new(RefCell::new(Shared { store, events: Vec::new(), accounts: HashMap::new(), ctx, opts, host, renewal: None })) }
    }
}

fn now() -> i64 {
    (js_sys::Date::now() / 1000.0) as i64
}

fn emit(sh: &Rcs, e: Event) {
    let ctx = {
        let mut s = sh.borrow_mut();
        s.events.push(e);
        s.ctx.clone()
    };
    ctx.request_repaint();
}

fn status(sh: &Rcs, id: &str, status: SyncStatus) {
    emit(sh, Event::Status { account: id.to_string(), status });
}

fn notice(sh: &Rcs, id: &str, message: String) {
    emit(sh, Event::Notice { account: id.to_string(), message });
}

fn store(sh: &Rcs) -> Store {
    sh.borrow().store.clone()
}

fn client(sh: &Rcs, id: &str) -> Option<Rc<Client<Fetch>>> {
    sh.borrow().accounts.get(id).and_then(|a| a.client.clone())
}

fn state(sh: &Rcs, id: &str) -> State {
    sh.borrow().accounts.get(id).map(|a| a.state.clone()).unwrap_or_default()
}

fn put_state(sh: &Rcs, id: &str, st: State) {
    let store = store(sh);
    if let Err(e) = st.save(&store, id) {
        log::warn!("{id}: {e}");
    }
    if let Some(a) = sh.borrow_mut().accounts.get_mut(id) {
        a.state = st;
    }
}

/// Queue work for an account and make sure its runner is going.
fn enqueue(sh: &Rcs, id: &str, task: Task) {
    let start = {
        let mut s = sh.borrow_mut();
        let Some(a) = s.accounts.get_mut(id) else { return };
        a.queue.push_back(task);
        !std::mem::replace(&mut a.running, true)
    };
    if start {
        wasm_bindgen_futures::spawn_local(run(sh.clone(), id.to_string()));
    }
}

async fn run(sh: Rcs, id: String) {
    loop {
        let task = {
            let mut s = sh.borrow_mut();
            let Some(a) = s.accounts.get_mut(&id) else { return };
            match a.queue.pop_front() {
                Some(t) => t,
                None => {
                    a.running = false;
                    return;
                }
            }
        };
        match task {
            Task::Connect => connect(&sh, &id).await,
            Task::SyncAll => sync_all(&sh, &id).await,
            Task::SyncFolder(f) => sync_one(&sh, &id, &f).await,
            Task::Job(j) => job(&sh, &id, j).await,
        }
    }
}

/// A failure: signing in again, reconnecting later, or just a notice.
fn failed(sh: &Rcs, id: &str, e: Error) {
    log::warn!("{id}: {e}");
    match e {
        Error::Auth(m) => {
            // //// Neoffice — a mailbox the hosting page handed over: its token was refused (it expired, or died with the
            // desk session or the server's cache). The page gives a new one, and the account signs in again, once.
            let hosted = {
                let s = sh.borrow();
                s.accounts.get(id).is_some_and(|a| !a.renewing && s.host.contains_key(&a.config.email.to_lowercase()))
            };
            if let Some(a) = sh.borrow_mut().accounts.get_mut(id) {
                a.client = None;
                a.password = None;
                // //// Neoffice — asked once per sign-in (renew), so a token refused again stops here.
                a.renewing |= hosted;
            }
            // //// Neoffice — one request to the page at a time: the others refused meanwhile wait for its answer.
            if hosted {
                status(sh, id, SyncStatus::Connecting);
                let first = {
                    let mut s = sh.borrow_mut();
                    let waiting = s.renewal.get_or_insert_with(Vec::new);
                    waiting.push(id.to_string());
                    waiting.len() == 1
                };
                if first {
                    let sh = sh.clone();
                    wasm_bindgen_futures::spawn_local(async move { renew(&sh).await });
                }
                return;
            }
            status(sh, id, SyncStatus::NeedsPassword { error: m });
        }
        e @ (Error::Io(_) | Error::Closed(_) | Error::Protocol(_) | Error::Tls(_)) => {
            if let Some(a) = sh.borrow_mut().accounts.get_mut(id) {
                a.client = None;
            }
            status(sh, id, SyncStatus::Error { error: e.to_string() });
        }
        e => notice(sh, id, e.to_string()),
    }
}

// //// Neoffice — new tokens from the hosting page (host.rs) for the accounts waiting for them, which then sign in
// again. They replace what the page handed over before: a mailbox it no longer gives, or no answer, means reloading.
async fn renew(sh: &Rcs) {
    let fresh = crate::host::renew().await;
    let waiting = {
        let mut s = sh.borrow_mut();
        if let Some(accounts) = &fresh {
            s.host = crate::host::secrets(accounts);
        }
        s.renewal.take().unwrap_or_default()
    };
    for id in waiting {
        let secret = {
            let s = sh.borrow();
            let given = s.accounts.get(&id).and_then(|a| s.host.get(&a.config.email.to_lowercase()));
            given.filter(|_| fresh.is_some()).map(|(secret, _)| secret.clone())
        };
        match secret {
            Some(secret) => {
                if let Some(a) = sh.borrow_mut().accounts.get_mut(&id) {
                    a.password = Some(secret);
                }
                enqueue(sh, &id, Task::Connect);
            }
            None => status(sh, &id, SyncStatus::NeedsPassword { error: "the page gave no new access to this mailbox: reload it".into() }),
        }
    }
}

async fn connect(sh: &Rcs, id: &str) {
    let Some((config, password)) = sh.borrow().accounts.get(id).map(|a| (a.config.clone(), a.password.clone())) else { return };
    let Some(url) = config.jmap.clone() else {
        return status(sh, id, SyncStatus::Error { error: "only JMAP accounts work in the browser; use the desktop app for this one".into() });
    };
    let Some(password) = password else {
        return status(sh, id, SyncStatus::NeedsPassword { error: "enter the password: the browser doesn't keep it".into() });
    };
    status(sh, id, SyncStatus::Connecting);
    let user = if config.username.is_empty() { config.email.clone() } else { config.username.clone() };
    // //// Neoffice — a mailbox the hosting page handed over signs in the way the page said (Bearer, host.rs).
    let bearer = sh.borrow().host.get(&config.email.to_lowercase()).is_some_and(|(_, bearer)| *bearer);
    match Client::connect(Fetch, &url, &user, &password, bearer).await {
        Ok(c) => {
            let ops = {
                let mut s = sh.borrow_mut();
                let Some(a) = s.accounts.get_mut(id) else { return };
                a.client = Some(Rc::new(c));
                // //// Neoffice — signed in: a later refusal may ask the hosting page again (renew).
                a.renewing = false;
                std::mem::take(&mut a.ops)
            };
            for op in ops {
                remote(sh, id, op).await;
            }
            sync_all(sh, id).await;
        }
        Err(e) => failed(sh, id, e),
    }
}

/// Refresh the folder list; `false` when that failed.
async fn relist(sh: &Rcs, id: &str) -> bool {
    let Some(c) = client(sh, id) else { return false };
    let mut st = state(sh, id);
    let r = mailcraft_jmap::list_folders(&c, &mut st, id).await;
    put_state(sh, id, st);
    match r {
        Ok(mut list) => {
            sort_folders(&mut list);
            let store = store(sh);
            let old: Vec<Folder> = sh.borrow().accounts.get(id).map(|a| a.folders.clone()).unwrap_or_default();
            for f in old.iter().filter(|o| !list.iter().any(|f| f.path == o.path)) {
                let _ = store.clear_folder(id, &f.path);
            }
            let _ = store.save_folders(id, &list);
            if let Some(a) = sh.borrow_mut().accounts.get_mut(id) {
                a.folders = list.clone();
            }
            emit(sh, Event::Folders { account: id.to_string(), folders: list });
            true
        }
        Err(e) => {
            failed(sh, id, e);
            false
        }
    }
}

async fn sync_all(sh: &Rcs, id: &str) {
    if !relist(sh, id).await {
        return;
    }
    let paths: Vec<String> =
        sh.borrow().accounts.get(id).map(|a| a.folders.iter().filter(|f| f.selectable).map(|f| f.path.clone()).collect()).unwrap_or_default();
    // The Inbox first.
    let inbox = sh.borrow().accounts.get(id).and_then(|a| a.folders.iter().find(|f| f.role == FolderRole::Inbox).map(|f| f.path.clone()));
    for p in inbox.iter().chain(paths.iter().filter(|p| Some(*p) != inbox.as_ref())) {
        if client(sh, id).is_none() {
            return;
        }
        sync_one(sh, id, p).await;
    }
    if let Some(c) = client(sh, id) {
        if let Ok(Some((used, limit))) = c.quota().await {
            emit(sh, Event::Quota { account: id.to_string(), used, limit });
        }
        // //// Neoffice — the address books and the calendars with the mail, when the server offers them (pim).
        let due = sh.borrow().accounts.get(id).is_some_and(|a| a.contacts_at.is_none_or(|at| now() - at >= CONTACTS_EVERY));
        if due {
            pim(sh, id).await;
        }
        status(sh, id, SyncStatus::Online { at: now() });
    }
}

// //// Neoffice — read the account's address books and calendars (JMAP for Contacts and for Calendars) and hand them
// to the engine to merge in one sync, as a CardDAV and CalDAV sync's result. What the server does not offer is not
// asked; a failure is said and tried at the next sync, and the other part is merged all the same.
async fn pim(sh: &Rcs, id: &str) {
    let Some(c) = client(sh, id) else { return };
    if !c.offers_contacts() && !c.offers_calendars() {
        return;
    }
    let at = now();
    let contacts = c.sync_contacts(at).await.unwrap_or_else(|e| {
        log::warn!("{id}: the address books were not read: {e}");
        None
    });
    let calendars = c.sync_calendars(at).await.unwrap_or_else(|e| {
        log::warn!("{id}: the calendars were not read: {e}");
        None
    });
    if let Some(a) = sh.borrow_mut().accounts.get_mut(id) {
        a.contacts_at = Some(at);
    }
    if let Some(sync) = mailcraft_jmap::calendars::pim_sync(contacts, calendars, at) {
        emit(sh, Event::DavSynced { account: id.to_string(), sync: Box::new(sync) });
    }
}

async fn sync_one(sh: &Rcs, id: &str, path: &str) {
    let Some(c) = client(sh, id) else { return };
    status(sh, id, SyncStatus::Syncing { folder: path.to_string() });
    let (store, opts) = {
        let s = sh.borrow();
        (s.store.clone(), s.opts)
    };
    let mut st = state(sh, id);
    let r = mailcraft_jmap::sync_folder(&c, &mut st, &store, id, path, &opts).await;
    put_state(sh, id, st);
    match r {
        Ok(r) => {
            let (first, counts) = {
                let mut s = sh.borrow_mut();
                let Some(a) = s.accounts.get_mut(id) else { return };
                let Some(f) = a.folders.iter_mut().find(|f| f.path == path) else { return };
                let first = f.total == 0 && r.total > 0;
                let changed = f.unread != r.unread || f.total != r.total;
                (f.unread, f.total) = (r.unread, r.total);
                (first, changed.then(|| a.folders.clone()))
            };
            if let Some(folders) = counts {
                let _ = store.save_folders(id, &folders);
                emit(sh, Event::Folders { account: id.to_string(), folders });
            }
            if r.touched() || first {
                emit(sh, Event::FolderChanged { account: id.to_string(), folder: path.to_string() });
            }
            status(sh, id, SyncStatus::Online { at: now() });
        }
        Err(e) => failed(sh, id, e),
    }
}

/// A change on the server (after the cache has it); then what it touched is shown afresh.
async fn remote(sh: &Rcs, id: &str, op: Op) {
    let Some(c) = client(sh, id) else {
        if let Some(a) = sh.borrow_mut().accounts.get_mut(id) {
            a.ops.push(op);
        }
        return;
    };
    let st = state(sh, id);
    match mailcraft_jmap::apply_remote(&c, &st, &op).await {
        Ok(()) => {
            // The folders it touched, with the server's counts.
            let mut touched: Vec<String> = op.keys().iter().map(|k| k.folder.clone()).collect();
            match &op {
                Op::Move { to, .. } => touched.push(to.clone()),
                Op::EmptyFolder { path } => touched.push(path.clone()),
                Op::CreateFolder { .. } | Op::RenameFolder { .. } | Op::DeleteFolder { .. } => {
                    relist(sh, id).await;
                }
                Op::SetFlag { .. } | Op::SetKeyword { .. } | Op::Expunge { .. } => {}
            }
            touched.sort();
            touched.dedup();
            for f in touched {
                sync_one(sh, id, &f).await;
            }
        }
        Err(e) => emit(sh, Event::OpFailed { account: id.to_string(), op, error: e.to_string() }),
    }
}

fn sent_folder(sh: &Rcs, id: &str, given: Option<String>) -> Option<String> {
    given.or_else(|| sh.borrow().accounts.get(id).and_then(|a| a.folders.iter().find(|f| f.role == FolderRole::Sent).map(|f| f.path.clone())))
}

async fn job(sh: &Rcs, id: &str, j: Job) {
    let store = store(sh);
    match j {
        Job::SyncAll { .. } => {
            if client(sh, id).is_some() {
                sync_all(sh, id).await;
            } else {
                connect(sh, id).await;
            }
        }
        Job::SyncFolder { folder, .. } => sync_one(sh, id, &folder).await,
        Job::FetchBody { key } => {
            let r = match client(sh, id) {
                Some(c) => mailcraft_jmap::fetch_body(&c, &state(sh, id), &store, &key).await,
                None => store.read_body(&key).ok_or_else(|| Error::No("not downloaded yet, and the account is offline".into())),
            };
            match r {
                Ok(raw) => {
                    emit(sh, Event::Body { key: key.clone(), raw });
                    emit(sh, Event::FolderChanged { account: key.account.clone(), folder: key.folder.clone() });
                }
                Err(e) => emit(sh, Event::BodyFailed { key, error: e.to_string() }),
            }
        }
        Job::Apply { op, .. } => {
            // The cache first, so the change shows at once.
            match store.apply_op(id, &op) {
                Ok(changed) => {
                    if op.is_folder_op() {
                        let folders = store.load_folders(id).unwrap_or_default();
                        if let Some(a) = sh.borrow_mut().accounts.get_mut(id) {
                            a.folders = folders.clone();
                        }
                        emit(sh, Event::Folders { account: id.to_string(), folders });
                    }
                    for folder in changed {
                        emit(sh, Event::FolderChanged { account: id.to_string(), folder });
                    }
                }
                Err(e) => notice(sh, id, e.to_string()),
            }
            remote(sh, id, op).await;
        }
        Job::Send { id: msg, raw, from, recipients, sent_folder: sent, .. } => {
            let Some(c) = client(sh, id) else {
                return emit(sh, Event::SendFailed { account: id.to_string(), id: msg, error: "the account is offline".into() });
            };
            let sent = sent_folder(sh, id, sent);
            match mailcraft_jmap::send(&c, &state(sh, id), sent.as_deref(), &raw, &from, &recipients).await {
                Ok(()) => {
                    emit(sh, Event::Sent { account: id.to_string(), id: msg });
                    if let Some(f) = sent {
                        sync_one(sh, id, &f).await;
                    }
                }
                Err(e) => emit(sh, Event::SendFailed { account: id.to_string(), id: msg, error: e.to_string() }),
            }
        }
        Job::SaveDraft { raw, folder, replaces, .. } => {
            let Some(c) = client(sh, id) else { return notice(sh, id, "the draft wasn't saved on the server: the account is offline".into()) };
            let st = state(sh, id);
            let r = async {
                mailcraft_jmap::append(&c, &st, &folder, &raw, &["$draft", "$seen"]).await?;
                if let Some(old) = replaces {
                    mailcraft_jmap::apply_remote(&c, &st, &Op::Expunge { keys: vec![old] }).await?;
                }
                Ok::<(), Error>(())
            }
            .await;
            match r {
                Ok(()) => sync_one(sh, id, &folder).await,
                Err(e) => failed(sh, id, e),
            }
        }
        Job::GetVacation { .. } | Job::SetVacation { .. } => {
            let Some(c) = client(sh, id) else {
                return emit(sh, Event::Vacation { account: id.to_string(), vacation: None, error: Some("the account is offline".into()) });
            };
            let mut failed_change = None;
            if let Job::SetVacation { vacation, .. } = &j
                && let Err(e) = c.set_vacation(vacation).await
            {
                failed_change = Some(e.to_string());
            }
            let (vacation, error) = match c.vacation().await {
                Ok(v) => (Some(v), failed_change),
                Err(e) => (None, Some(failed_change.unwrap_or_else(|| e.to_string()))),
            };
            emit(sh, Event::Vacation { account: id.to_string(), vacation, error });
        }
        // Nothing to do in the browser: no Outbox (sending needs the server), no CalDAV.
        // //// Neoffice — but address books and calendars over JMAP, read again when the engine asks (pim).
        Job::SyncDav { .. } => pim(sh, id).await,
        Job::FlushOutbox { .. } => {}
        Job::Start { .. }
        | Job::Remove { .. }
        | Job::FetchUrl { .. }
        | Job::Discover { .. }
        | Job::SignIn { .. }
        | Job::ImportPst { .. }
        | Job::ExportPst { .. }
        | Job::WorkOffline { .. } => {}
    }
}

/// Check the Inbox every few minutes (and everything now and then), reconnecting when needed.
async fn poll(sh: Rcs, id: String, generation: u32) {
    let mut tick = 0u32;
    loop {
        sleep(POLL_MS).await;
        let state = sh.borrow().accounts.get(&id).map(|a| (a.generation, a.client.is_some(), a.password.is_some()));
        let Some((g, connected, password)) = state else { return };
        if g != generation {
            return;
        }
        tick = tick.wrapping_add(1);
        let inbox = sh.borrow().accounts.get(&id).and_then(|a| a.folders.iter().find(|f| f.role == FolderRole::Inbox).map(|f| f.path.clone()));
        let pushing = sh.borrow().accounts.get(&id).is_some_and(|a| a.pushing);
        match (connected, inbox) {
            (false, _) if password => enqueue(&sh, &id, Task::Connect),
            (false, _) => {}
            (true, _) if tick.is_multiple_of(FULL_EVERY) => enqueue(&sh, &id, Task::SyncAll),
            (true, Some(f)) if !pushing => enqueue(&sh, &id, Task::SyncFolder(f)),
            (true, _) => {}
        }
    }
}

fn inbox(sh: &Rcs, id: &str) -> Option<String> {
    sh.borrow().accounts.get(id).and_then(|a| a.folders.iter().find(|f| f.role == FolderRole::Inbox).map(|f| f.path.clone()))
}

fn current(sh: &Rcs, id: &str, generation: u32) -> bool {
    sh.borrow().accounts.get(id).is_some_and(|a| a.generation == generation)
}

fn set_pushing(sh: &Rcs, id: &str, on: bool) {
    if let Some(a) = sh.borrow_mut().accounts.get_mut(id) {
        a.pushing = on;
    }
}

/// Hold the account's push stream open while the account is (RFC 8620 §7.3): a `state` event
/// means something changed, and the Inbox syncs.
async fn push(sh: Rcs, id: String, generation: u32) {
    let mut wait = 0;
    loop {
        if wait > 0 {
            sleep(wait).await;
        }
        if !current(&sh, &id, generation) {
            return;
        }
        let Some(c) = client(&sh, &id) else {
            wait = PUSH_WAIT_MS;
            continue;
        };
        // No push from this server: polling it is.
        let Some(url) = c.events_url() else { return };
        let mut opened = false;
        let mut on_event = |e: mailcraft_jmap::sse::Sse| {
            if !current(&sh, &id, generation) {
                return false;
            }
            if !std::mem::replace(&mut opened, true) {
                set_pushing(&sh, &id, true);
            }
            // Mail that came while the stream was closed sends no event: catch up once it's open.
            if (e.event == "state" || e.event == "open")
                && let Some(f) = inbox(&sh, &id)
            {
                enqueue(&sh, &id, Task::SyncFolder(f));
            }
            true
        };
        let r = events(&url, c.authorization(), &mut on_event).await;
        set_pushing(&sh, &id, false);
        if let Err(e) = r {
            log::info!("{id}: push: {e}");
        }
        wait = PUSH_RETRY_MS;
    }
}

/// Download a picture a message links to (once the user asked to see pictures).
async fn fetch_url(sh: Rcs, url: String) {
    use mailcraft_jmap::Http as _;
    let r = Fetch.send(mailcraft_jmap::Request::get(&url, MAX_PICTURE)).await;
    let e = match r {
        Ok(resp) if (200..300).contains(&resp.status) => Event::Fetched { url, data: Some(resp.body), error: None },
        Ok(resp) => Event::Fetched { url, data: None, error: Some(format!("HTTP {}", resp.status)) },
        Err(e) => Event::Fetched { url, data: None, error: Some(e.to_string()) },
    };
    emit(&sh, e);
}

impl Backend for WebBackend {
    fn submit(&mut self, job: Job) {
        let sh = &self.shared;
        match job {
            Job::FetchUrl { url } => wasm_bindgen_futures::spawn_local(fetch_url(sh.clone(), url)),
            // No autoconfiguration in the browser: the user gives the JMAP address.
            Job::Discover { email } => emit(sh, Event::Discovered { email, config: None, source: String::new() }),
            Job::SignIn { account } => {
                status(sh, &account.id, SyncStatus::NeedsPassword { error: "signing in through Google or Microsoft needs the desktop app".into() })
            }
            Job::ImportPst { account, .. } => emit(
                sh,
                Event::Imported {
                    account,
                    messages: 0,
                    skipped: 0,
                    done: true,
                    error: Some("opening Outlook data files needs the desktop app".into()),
                },
            ),
            Job::ExportPst { account, .. } => emit(
                sh,
                Event::Exported {
                    account,
                    messages: 0,
                    skipped: 0,
                    done: true,
                    error: Some("writing Outlook data files needs the desktop app".into()),
                },
            ),
            Job::Start { account, password } => {
                let id = account.id.clone();
                let generation = {
                    let mut s = sh.borrow_mut();
                    let store = s.store.clone();
                    let old = s.accounts.remove(&id);
                    let generation = old.as_ref().map_or(1, |a| a.generation.wrapping_add(1));
                    let password = password.map(|p| p.0).or_else(|| old.as_ref().and_then(|a| a.password.clone()));
                    // //// Neoffice — else what the hosting page handed over for this mailbox (host.rs).
                    let password = password.or_else(|| s.host.get(&account.email.to_lowercase()).map(|(secret, _)| secret.clone()));
                    s.accounts.insert(
                        id.clone(),
                        Account {
                            state: State::load(&store, &id),
                            folders: store.load_folders(&id).unwrap_or_default(),
                            ops: old.map(|a| a.ops).unwrap_or_default(),
                            config: account,
                            password,
                            client: None,
                            queue: VecDeque::new(),
                            running: false,
                            generation,
                            // //// Neoffice — see `renew`.
                            renewing: false,
                            contacts_at: None,
                            pushing: false,
                        },
                    );
                    generation
                };
                enqueue(sh, &id, Task::Connect);
                wasm_bindgen_futures::spawn_local(poll(sh.clone(), id.clone(), generation));
                wasm_bindgen_futures::spawn_local(push(sh.clone(), id, generation));
            }
            Job::Remove { account } => {
                sh.borrow_mut().accounts.remove(&account);
            }
            other => {
                let id = other.account().to_string();
                if sh.borrow().accounts.contains_key(&id) {
                    enqueue(sh, &id, Task::Job(other));
                } else if let Job::FetchBody { key } = other {
                    // The sample mailbox and other accounts with no server: the cache has it or nobody does.
                    let raw = store(sh).read_body(&key);
                    match raw {
                        Some(raw) => emit(sh, Event::Body { key, raw }),
                        None => emit(sh, Event::BodyFailed { key, error: "not downloaded".into() }),
                    }
                } else if let Job::Apply { account, op } = other {
                    match store(sh).apply_op(&account, &op) {
                        Ok(changed) => {
                            if op.is_folder_op() {
                                let folders = store(sh).load_folders(&account).unwrap_or_default();
                                emit(sh, Event::Folders { account: account.clone(), folders });
                            }
                            for folder in changed {
                                emit(sh, Event::FolderChanged { account: account.clone(), folder });
                            }
                        }
                        Err(e) => notice(sh, &account, e.to_string()),
                    }
                } else if let Job::Send { account, id, .. } = other {
                    emit(sh, Event::SendFailed { account, id, error: "this mailbox has no server to send through".into() });
                }
            }
        }
    }

    fn poll(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.shared.borrow_mut().events)
    }
}
