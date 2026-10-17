//! The browser shell: the eframe web runner, the browser's `Services`, and what the page keeps
//! between visits: the cache in IndexedDB (local storage when that's unavailable), settings in
//! local storage.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use mailcraft_engine::Session;
use mailcraft_engine::model::dates::Clock;
use mailcraft_engine::store::Store;
use mailcraft_ui_egui::{MailApp, Services};
use wasm_bindgen::JsCast as _;

use crate::backend::WebBackend;

const CANVAS_ID: &str = "mailcraft_canvas";
const LOADING_ID: &str = "mailcraft_loading";
/// Where the account list and settings are kept without IndexedDB (no passwords, no mail).
const FILES_KEY: &str = "mailcraft.files";
const UI_KEY: &str = "mailcraft.ui";
// //// Neoffice — the signatures and the names shown on mail made here for a hosting page's mailboxes, by address, with the
// page's each replaced (kept.rs).
const SIGNATURES_KEY: &str = "mailcraft.signatures";
const NAMES_KEY: &str = "mailcraft.names";
/// Most bytes kept in local storage (browsers allow about 5 MB a site).
const MAX_KEPT: usize = 3 << 20;

pub fn start() {
    eframe::WebLogger::init(log::LevelFilter::Info).ok();
    wasm_bindgen_futures::spawn_local(async {
        let Some(document) = web_sys::window().and_then(|w| w.document()) else {
            log::error!("no document");
            return;
        };
        let Some(canvas) = document.get_element_by_id(CANVAS_ID).and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok()) else {
            log::error!("missing <canvas id=\"{CANVAS_ID}\">");
            return;
        };
        let sample = query().contains("sample");
        // //// Neoffice — the desk's language for « Automatic », and the product's name (NeoMail), when a Neoffice page
        // hosts MailCraft (host.rs).
        if let Some(tag) = crate::host::language() {
            mailcraft_ui_egui::i18n::prefer_host_language(&tag);
        }
        let product = crate::host::product();
        if let Some(name) = &product {
            mailcraft_ui_egui::i18n::rename_product(name);
        }
        // //// Neoffice — the mailboxes a hosting page hands over (host.rs): opened at once, nothing kept.
        let host = if sample || param("open").is_some() { Vec::new() } else { crate::host::accounts() };
        let hosted = !host.is_empty();
        // The cache from the last visit.
        // //// Neoffice — none with the host's mailboxes, as with the sample one.
        let (db, kept) = if sample || hosted {
            (None, Vec::new())
        } else {
            match crate::idb::open().await {
                Ok(db) => {
                    let kept = crate::idb::load(&db).await.unwrap_or_else(|e| {
                        log::warn!("the kept mail could not be read: {e}");
                        Vec::new()
                    });
                    (Some(db), kept)
                }
                Err(e) => {
                    log::warn!("no IndexedDB ({e}): only settings are kept");
                    (None, Vec::new())
                }
            }
        };
        let mut options = eframe::WebOptions::default();
        if query().contains("webgl")
            && let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut options.wgpu_options.wgpu_setup
        {
            create.instance_descriptor.backends = eframe::wgpu::Backends::GL;
        }
        let result = eframe::WebRunner::new()
            .start(
                canvas,
                options,
                Box::new(move |cc| {
                    let (now, offset) = clock();
                    let store = if sample {
                        let s = Store::memory();
                        if let Err(e) = mailcraft_engine::sample::seed_if_empty_at(&s, now, i64::from(offset)) {
                            log::error!("the sample mailbox couldn't be made: {e}");
                        }
                        s
                    // //// Neoffice — the host's mailboxes, in memory (host.rs), signing with what was made here (kept.rs).
                    } else if hosted {
                        crate::host::store(&host, &kept_texts(cc.storage, SIGNATURES_KEY), &kept_texts(cc.storage, NAMES_KEY))
                    } else if !kept.is_empty() {
                        Store::memory_from(kept)
                    } else {
                        // A first visit with IndexedDB, or none: what local storage has.
                        let kept: Vec<(String, String)> =
                            cc.storage.and_then(|s| s.get_string(FILES_KEY)).and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default();
                        let s = Store::memory_from(kept.into_iter().map(|(p, v)| (p, v.into_bytes())).collect());
                        // Into IndexedDB from now on.
                        if let Some(db) = &db {
                            let all: Vec<_> = s.snapshot(|_| true).into_iter().map(|(p, v)| (p, Some(v))).collect();
                            if let Err(e) = crate::idb::save(db, &all) {
                                log::warn!("the mail cache wasn't kept: {e}");
                            }
                        }
                        s
                    };
                    if db.is_some() {
                        store.track_changes();
                    }
                    let mut session = Session::open(Some(store.clone()), Clock::new(now, offset));
                    session.jmap_only = true;
                    // //// Neoffice — with the host's mailboxes nothing is kept between visits: new events and contacts go to
                    // their servers' collections only, never to the ones here, lost at the next reload (host.rs).
                    session.keeps_local_pim = !hosted;
                    // //// Neoffice — the host's mailboxes sign in with what the page gave (host.rs).
                    let backend = WebBackend::new(store.clone(), cc.egui_ctx.clone(), crate::host::secrets(&host));
                    // //// Neoffice — Nora, when the page asked for her (`?nora`) or makes her calls itself (`mailcraftNoraCall`, Neodesk).
                    let nora = crate::nora::requested(&query())
                        .or_else(|| crate::nora::offered().then(String::new))
                        .map(|origin| crate::nora::Nora::new(origin, cc.egui_ctx.clone()));
                    let hosted_mail = host.iter().map(|a| a.email.to_lowercase()).collect();
                    let services =
                        WebServices { picked: Rc::default(), ctx: cc.egui_ctx.clone(), nora, hosted: hosted_mail, store: store.clone(), zone: None };
                    let mut app = MailApp::new(session, Box::new(backend), Box::new(services));
                    if sample {
                        app.ui.title_base = "MailCraft – Sample mailbox".into();
                    }
                    // //// Neoffice — the window's title under the product's name the page gives.
                    if let Some(name) = &product {
                        app.ui.title_base = app.ui.title_base.replace("MailCraft", name);
                    }
                    // //// Neoffice — the person signed in to the hosting page (`?author=`), shown as the document editors
                    // show them: their initials at the right end of the top bar (host.rs).
                    app.ui.signed_in = crate::host::author();
                    if let Some(v) = cc.storage.and_then(|s| s.get_string(UI_KEY)).and_then(|j| serde_json::from_str(&j).ok()) {
                        // //// Neoffice — with the host's mailboxes, each mailbox's settings by its address (hosted.rs).
                        let v = if hosted { mailcraft_ui_egui::hosted::restored_settings(v, &ids(&app.session)) } else { v };
                        app.restore_state(&v);
                    }
                    // A message's own window (opened from a message's pop-out).
                    if let Some(key) = param("open") {
                        app.show_message_page(&key);
                    }
                    // //// Neoffice — with the host's mailboxes, the settings only (kept.rs), and what the page gave each
                    // mailbox for a signature and a name, to tell one made here.
                    let page_signatures = host.iter().map(|a| (a.email.to_lowercase(), a.signature.clone())).collect();
                    let page_names = host.iter().map(|a| (a.email.to_lowercase(), a.name.clone())).collect();
                    Ok(Box::new(WebShell { app, store, sample, hosted, page_signatures, page_names, db }))
                }),
            )
            .await;
        if let Some(el) = document.get_element_by_id(LOADING_ID) {
            match result {
                Ok(()) => el.remove(),
                Err(e) => el.set_inner_html(&format!("<p>MailCraft failed to start: {e:?}</p><p>A browser with WebGPU or WebGL2 is required.</p>")),
            }
        }
    });
}

fn query() -> String {
    web_sys::window().and_then(|w| w.location().search().ok()).unwrap_or_default()
}

/// A `?name=value` parameter of the page's address, decoded.
fn param(name: &str) -> Option<String> {
    let q = query();
    let raw = q.trim_start_matches('?').split('&').find_map(|kv| kv.strip_prefix(&format!("{name}=")))?;
    js_sys::decode_uri_component(raw).ok().map(String::from)
}

// //// Neoffice — the browser's offset of an instant (its time-zone rules), and its rules from three years back to ten ahead
// (Services::zone: every date by its own offset, daylight saving), worked out once a day.
fn browser_offset(t: i64) -> i32 {
    -(js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(t as f64 * 1000.0)).get_timezone_offset() as i32) * 60
}

fn browser_zone(now: i64) -> mailcraft_engine::model::dates::Zone {
    mailcraft_engine::model::dates::Zone::from_offsets(now - 3 * 366 * 86_400, now + 10 * 366 * 86_400, browser_offset)
}

/// Unix seconds and the local UTC offset in seconds.
fn clock() -> (i64, i32) {
    let d = js_sys::Date::new_0();
    let now = (d.get_time() / 1000.0) as i64;
    // getTimezoneOffset is minutes behind UTC.
    let offset = -(d.get_timezone_offset() as i32) * 60;
    (now, offset)
}

struct WebShell {
    app: MailApp,
    store: Store,
    sample: bool,
    // //// Neoffice — a hosting page's mailboxes: the settings are kept, and the signatures made here; never mail,
    // accounts or secrets (kept.rs). What the page gave each for a signature (by address, lower case).
    hosted: bool,
    page_signatures: std::collections::HashMap<String, String>,
    page_names: std::collections::HashMap<String, String>,
    /// Where the cache is kept between visits.
    db: Option<web_sys::IdbDatabase>,
}

impl eframe::App for WebShell {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.app.ui(ui);
        // What changed in the cache this frame goes to IndexedDB.
        if let Some(db) = &self.db {
            let changes: Vec<_> = self.store.take_changes().into_iter().filter(|(p, _)| !p.ends_with(".bad")).collect();
            if !changes.is_empty()
                && let Err(e) = crate::idb::save(db, &changes)
            {
                log::warn!("the mail cache wasn't kept: {e}");
            }
        }
    }

    /// Keep the settings, and without IndexedDB the account and folder lists (not mail, never
    /// passwords).
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        // //// Neoffice — what is kept depends on the mailboxes (kept.rs): with a hosting page's, the settings only.
        let keeps = crate::kept::keeps(self.sample, self.hosted);
        if !keeps.settings {
            return;
        }
        // //// Neoffice — with the host's mailboxes, the interface's only, each mailbox's by its address (hosted.rs).
        let state = if self.hosted {
            mailcraft_ui_egui::hosted::saved_settings(self.app.saved_state(), &ids(&self.app.session))
        } else {
            self.app.saved_state()
        };
        if let Ok(j) = serde_json::to_string(&state) {
            storage.set_string(UI_KEY, j);
        }
        // //// Neoffice — and the signatures and names made here (kept.rs), not the mail, the accounts or their secrets.
        if self.hosted {
            save_kept(storage, SIGNATURES_KEY, &self.app.session, &self.page_signatures, |c| &c.signature);
            save_kept(storage, NAMES_KEY, &self.app.session, &self.page_names, |c| &c.name);
        }
        if !keeps.files {
            return;
        }
        if self.db.is_some() {
            storage.remove_string(FILES_KEY);
            return;
        }
        let keep = |p: &str| p.ends_with(".json") && !p.ends_with(".bad") && !p.contains("/outbox/");
        let mut files: Vec<(String, String)> =
            self.store.snapshot(keep).into_iter().filter_map(|(p, v)| Some((p, String::from_utf8(v).ok()?))).collect();
        // Folder contents only while there's room; they come back with the next sync.
        let size = |f: &[(String, String)]| f.iter().map(|(p, v)| p.len() + v.len()).sum::<usize>();
        if size(&files) > MAX_KEPT {
            files.retain(|(p, _)| !p.ends_with("/state.json"));
        }
        if size(&files) <= MAX_KEPT
            && let Ok(j) = serde_json::to_string(&files)
        {
            storage.set_string(FILES_KEY, j);
        }
    }

    fn auto_save_interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs(10)
    }
}

// //// Neoffice — each account's id and address, which what is a mailbox's is kept by (hosted.rs).
fn ids(session: &Session) -> Vec<(String, String)> {
    session.accounts.iter().map(|a| (a.config.id.clone(), a.config.email.to_lowercase())).collect()
}

// //// Neoffice — the signatures (or names) made here for a hosting page's mailboxes (kept.rs), as kept under `key`: by
// address, `{ text, base }`.
fn kept_texts(storage: Option<&dyn eframe::Storage>, key: &str) -> std::collections::HashMap<String, crate::kept::Kept> {
    let Some(v) = storage.and_then(|s| s.get_string(key)).and_then(|j| serde_json::from_str::<serde_json::Value>(&j).ok()) else {
        return Default::default();
    };
    let text = |e: &serde_json::Value, key: &str| e.get(key).and_then(serde_json::Value::as_str).unwrap_or_default().to_string();
    v.as_object()
        .map(|m| m.iter().map(|(email, e)| (email.to_lowercase(), crate::kept::Kept { text: text(e, "text"), base: text(e, "base") })).collect())
        .unwrap_or_default()
}

// //// Neoffice — see `kept_texts`: each hosted mailbox's signature (or name: `of`) made here, none while it is the page's.
fn save_kept(
    storage: &mut dyn eframe::Storage,
    key: &str,
    session: &Session,
    page: &std::collections::HashMap<String, String>,
    of: impl Fn(&mailcraft_engine::model::AccountConfig) -> &String,
) {
    let mut kept = serde_json::Map::new();
    for a in &session.accounts {
        let email = a.config.email.to_lowercase();
        let Some(given) = page.get(&email) else { continue };
        if let Some(k) = crate::kept::keep(given, of(&a.config)) {
            kept.insert(email, serde_json::json!({"text": k.text, "base": k.base}));
        }
    }
    if kept.is_empty() {
        storage.remove_string(key);
    } else if let Ok(j) = serde_json::to_string(&kept) {
        storage.set_string(key, j);
    }
}

/// Files chosen in the picker: (purpose, name, bytes).
type Picked = Rc<RefCell<Vec<(String, String, Vec<u8>)>>>;

struct WebServices {
    picked: Picked,
    ctx: egui::Context,
    // //// Neoffice — the connection to Nora, when the page that hosts MailCraft asked for her; the addresses of
    // the mailboxes it handed over (lower case); the store, where the JMAP ids behind MailCraft's numbers are kept.
    nora: Option<crate::nora::Nora>,
    hosted: std::collections::HashSet<String>,
    store: Store,
    // //// Neoffice — the browser's time-zone rules (browser_zone) and the day they were worked out on.
    zone: Option<(i64, mailcraft_engine::model::dates::Zone)>,
}

impl Services for WebServices {
    fn clock(&self) -> (i64, i32) {
        clock()
    }
    // //// Neoffice — see `WebServices::zone`.
    fn zone(&mut self) -> Option<mailcraft_engine::model::dates::Zone> {
        let (now, _) = clock();
        let day = now.div_euclid(86_400);
        if self.zone.is_none_or(|(d, _)| d != day) {
            self.zone = Some((day, browser_zone(now)));
        }
        self.zone.map(|(_, z)| z)
    }
    fn pick_files(&mut self) -> Vec<PathBuf> {
        Vec::new()
    }
    fn save_path(&mut self, _suggested: &str) -> Option<PathBuf> {
        None
    }
    fn open_path(&mut self, _path: &Path) -> Result<(), String> {
        Err("the browser has no files to open".into())
    }
    fn open_url(&mut self, url: &str) -> Result<(), String> {
        let w = web_sys::window().ok_or("no window")?;
        // A new tab without access back to this one.
        w.open_with_url_and_target_and_features(url, "_blank", "noopener,noreferrer").map(|_| ()).map_err(|e| format!("{e:?}"))
    }
    fn downloads(&self) -> bool {
        true
    }
    fn open_message_window(&mut self, key: &str) -> bool {
        // //// Neoffice — a window of its own would have no mailbox of the page: the message shows in the page.
        if !self.hosted.is_empty() {
            return false;
        }
        let Some(w) = web_sys::window() else { return false };
        let path = w.location().pathname().unwrap_or_else(|_| "/".into());
        // The same mailbox (the sample stays the sample) in a window of its own, message-sized and
        // a little in from this one's corner, as web mail opens one (not a tab, not maximised).
        let sample = if query().contains("sample") { "sample&" } else { "" };
        let url = format!("{path}?{sample}open={}", String::from(js_sys::encode_uri_component(key)));
        let at = |v: Result<wasm_bindgen::JsValue, wasm_bindgen::JsValue>| v.ok().and_then(|x| x.as_f64()).unwrap_or(0.0) as i32 + 60;
        let features = format!("popup=yes,width=940,height=740,left={},top={}", at(w.screen_x()), at(w.screen_y()));
        // A blocked pop-up opens nothing: then the message shows in a window drawn in the page.
        matches!(w.open_with_url_and_target_and_features(&url, "_blank", &features), Ok(Some(_)))
    }
    fn close_page(&mut self) {
        if let Some(w) = web_sys::window() {
            let _ = w.close();
        }
    }
    fn download(&mut self, name: &str, bytes: &[u8]) {
        if let Err(e) = download(name, bytes) {
            log::error!("download of {name} failed: {e}");
        }
    }
    fn pick_later(&mut self, purpose: &str) -> bool {
        let (picked, ctx, purpose) = (self.picked.clone(), self.ctx.clone(), purpose.to_string());
        let dialog = if purpose == "picture" {
            rfd::AsyncFileDialog::new().add_filter("Pictures", &["png", "jpg", "jpeg", "gif", "webp", "bmp"])
        } else {
            rfd::AsyncFileDialog::new()
        };
        wasm_bindgen_futures::spawn_local(async move {
            let Some(files) = dialog.pick_files().await else { return };
            for f in files {
                let bytes = f.read().await;
                picked.borrow_mut().push((purpose.clone(), f.file_name(), bytes));
            }
            ctx.request_repaint();
        });
        true
    }
    fn take_picked(&mut self) -> Vec<(String, String, Vec<u8>)> {
        std::mem::take(&mut self.picked.borrow_mut())
    }
    // //// Neoffice — Nora, through the page's session (nora.rs).
    fn nora_available(&self) -> bool {
        self.nora.is_some()
    }
    fn nora_send(&mut self, call: mailcraft_ui_egui::NoraCall) {
        if let Some(nora) = &self.nora {
            nora.send(call);
        }
    }
    fn nora_answers(&mut self) -> Vec<mailcraft_ui_egui::NoraAnswer> {
        self.nora.as_ref().map(crate::nora::Nora::answers).unwrap_or_default()
    }
    fn nora_mailbox(&self, email: &str) -> bool {
        self.hosted.contains(&email.to_lowercase())
    }
    fn nora_message(&self, key: &mailcraft_engine::model::MessageKey) -> Option<String> {
        // For a mailbox the page handed over only: the JMAP id the backend gave this message's number.
        let account = self.store.load_accounts().ok()?.into_iter().find(|a| a.id == key.account)?;
        if !self.hosted.contains(&account.email.to_lowercase()) {
            return None;
        }
        mailcraft_jmap::email_id_of(&self.store, key)
    }
}

/// Hand bytes to the browser as a download.
fn download(name: &str, bytes: &[u8]) -> Result<(), String> {
    let err = |e: wasm_bindgen::JsValue| format!("{e:?}");
    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes));
    let props = web_sys::BlobPropertyBag::new();
    props.set_type("application/octet-stream");
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &props).map_err(err)?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(err)?;
    let document = web_sys::window().and_then(|w| w.document()).ok_or("no document")?;
    let a = document.create_element("a").map_err(err)?.dyn_into::<web_sys::HtmlAnchorElement>().map_err(|_| "not a link".to_string())?;
    a.set_href(&url);
    a.set_download(name);
    a.click();
    web_sys::Url::revoke_object_url(&url).map_err(err)
}
