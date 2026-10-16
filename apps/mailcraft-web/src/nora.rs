// //// Neoffice — added file (no upstream equivalent): how the browser build reaches Nora, the assistant of the Neoffice server.

//! Nora's connection for the browser build: a `fetch` to the Neoffice server made with the page's
//! own session, as WordCraft's Nora pane does.
//!
//! The page that hosts MailCraft turns the connection on in its address: `?nora` (calls go to the
//! page's own origin, the instance that serves MailCraft) or `?nora=<origin>` (another origin).
//! The host can also describe the connection in `window.mailcraftNora`, read at every call:
//! `{ origin?: string, headers?: { [name]: value }, credentials?: "include" | "same-origin" | "omit" }`.
//! A Frappe page's CSRF token (`csrf_token` or `frappe.csrf_token`) is sent as `X-Frappe-CSRF-Token`
//! without being asked; when MailCraft runs in a frame of a page of the same origin, that page's
//! settings and token count too. Nothing secret travels in the address.

use std::cell::RefCell;
use std::rc::Rc;

use mailcraft_ui_egui::{NoraAnswer, NoraCall};
use wasm_bindgen::{JsCast as _, JsValue};
use wasm_bindgen_futures::{JsFuture, spawn_local};

/// The origin Nora is asked at when the page asked for her (`""`: the page's own). `None` when the
/// page didn't ask. `query` is the page's `location.search`.
pub fn requested(query: &str) -> Option<String> {
    for item in query.trim_start_matches('?').split('&') {
        let (key, value) = item.split_once('=').unwrap_or((item, ""));
        if key == "nora" {
            let decoded = js_sys::decode_uri_component(value).ok().and_then(|text| text.as_string()).unwrap_or_default();
            return Some(if matches!(decoded.as_str(), "1" | "true") { String::new() } else { decoded });
        }
    }
    None
}

/// The connection: calls go out as `fetch`es, answers wait in `inbox` for the next frame.
pub struct Nora {
    origin: String,
    inbox: Rc<RefCell<Vec<NoraAnswer>>>,
    ctx: egui::Context,
}

impl Nora {
    pub fn new(origin: String, ctx: egui::Context) -> Nora {
        Nora { origin, inbox: Rc::default(), ctx }
    }

    /// Send a call; its answer, whatever happens, is handed back by [`Nora::answers`].
    pub fn send(&self, call: NoraCall) {
        let (origin, inbox, ctx) = (self.origin.clone(), self.inbox.clone(), self.ctx.clone());
        spawn_local(async move {
            let answer = match fetch(&origin, &call).await {
                Ok((status, body)) => NoraAnswer { id: call.id, status, body },
                // No answer from the network: status 0, and why.
                Err(reason) => NoraAnswer { id: call.id, status: 0, body: reason },
            };
            inbox.borrow_mut().push(answer);
            ctx.request_repaint();
        });
    }

    /// The answers that came back since the last call.
    pub fn answers(&self) -> Vec<NoraAnswer> {
        std::mem::take(&mut self.inbox.borrow_mut())
    }
}

/// What the host said in `mailcraftNora`.
struct Host {
    origin: Option<String>,
    headers: Vec<(String, String)>,
    credentials: Option<web_sys::RequestCredentials>,
}

fn get(target: &JsValue, key: &str) -> Option<JsValue> {
    js_sys::Reflect::get(target, &JsValue::from_str(key)).ok().filter(|value| !value.is_undefined() && !value.is_null())
}

/// The page itself, then the page that frames it when it is of the same origin (reading another
/// origin's window throws, which reads as nothing here).
fn windows(window: &web_sys::Window) -> Vec<JsValue> {
    let own = JsValue::from(window.clone());
    let mut all = vec![own.clone()];
    if let Ok(Some(parent)) = window.parent() {
        let parent = JsValue::from(parent);
        // A top page is its own parent; another origin's `document` can't be read.
        if parent != own && get(&parent, "document").is_some() {
            all.push(parent);
        }
    }
    all
}

fn host(windows: &[JsValue]) -> Host {
    let mut host = Host { origin: None, headers: Vec::new(), credentials: None };
    let Some(settings) = windows.iter().find_map(|w| get(w, "mailcraftNora").filter(JsValue::is_object)) else {
        return host;
    };
    host.origin = get(&settings, "origin").and_then(|value| value.as_string());
    host.credentials = get(&settings, "credentials").and_then(|value| value.as_string()).and_then(|name| match name.as_str() {
        "include" => Some(web_sys::RequestCredentials::Include),
        "same-origin" => Some(web_sys::RequestCredentials::SameOrigin),
        "omit" => Some(web_sys::RequestCredentials::Omit),
        _ => None,
    });
    if let Some(headers) = get(&settings, "headers").filter(JsValue::is_object) {
        for entry in js_sys::Object::entries(&js_sys::Object::from(headers)).iter() {
            let pair = js_sys::Array::from(&entry);
            if let (Some(name), Some(value)) = (pair.get(0).as_string(), pair.get(1).as_string()) {
                host.headers.push((name, value));
            }
        }
    }
    host
}

/// The CSRF token of a Frappe page, if the page (or the page that frames it) has one.
fn csrf_token(windows: &[JsValue]) -> Option<String> {
    windows.iter().find_map(|w| {
        get(w, "csrf_token")
            .and_then(|token| token.as_string())
            .or_else(|| get(w, "frappe").and_then(|frappe| get(&frappe, "csrf_token")).and_then(|token| token.as_string()))
            .filter(|token| !token.is_empty() && token != "None")
    })
}

fn reason(error: JsValue) -> String {
    error
        .as_string()
        .or_else(|| js_sys::Reflect::get(&error, &JsValue::from_str("message")).ok().and_then(|message| message.as_string()))
        .unwrap_or_else(|| format!("{error:?}"))
}

/// One call: a POST of the arguments to the method, answered with its status and body.
async fn fetch(default_origin: &str, call: &NoraCall) -> Result<(u16, String), String> {
    let window = web_sys::window().ok_or("no window")?;
    let windows = windows(&window);
    let host = host(&windows);
    let origin = host.origin.as_deref().unwrap_or(default_origin).trim_end_matches('/');
    let url = format!("{origin}/api/method/{}", call.method);
    let headers = web_sys::Headers::new().map_err(reason)?;
    headers.set("Content-Type", "application/json").map_err(reason)?;
    headers.set("Accept", "application/json").map_err(reason)?;
    if let Some(token) = csrf_token(&windows) {
        headers.set("X-Frappe-CSRF-Token", &token).map_err(reason)?;
    }
    for (name, value) in &host.headers {
        headers.set(name, value).map_err(reason)?;
    }
    let init = web_sys::RequestInit::new();
    init.set_method("POST");
    init.set_headers(&headers);
    init.set_body(&JsValue::from_str(&call.args.to_string()));
    if let Some(credentials) = host.credentials {
        init.set_credentials(credentials);
    }
    let answer = JsFuture::from(window.fetch_with_str_and_init(&url, &init)).await.map_err(reason)?;
    let response: web_sys::Response = answer.dyn_into().map_err(|_| "the answer is not a response".to_string())?;
    let text = JsFuture::from(response.text().map_err(reason)?).await.map_err(reason)?;
    Ok((response.status(), text.as_string().unwrap_or_default()))
}
