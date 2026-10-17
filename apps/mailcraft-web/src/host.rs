// //// Neoffice — added file (no upstream equivalent): the mailboxes a hosting page hands MailCraft (maintenance#1630).

//! The mailboxes a hosting page hands MailCraft, so that it opens them without a form or a password.
//!
//! The page (Neoffice's `/neomail` in the desk, or Neodesk's window, which frame the browser build) sets, before
//! MailCraft starts:
//!
//! ```js
//! window.mailcraftJmap = { accounts: [{
//!   email: "jane@example.com",   // the mailbox, MailCraft's account key
//!   name: "Jane Doe",            // the name on sent mail
//!   session: "https://…",        // its JMAP session URL
//!   username: "jane@example.com",
//!   secret: "<token>",           // kept in this tab's memory only, never stored
//!   scheme: "bearer",            // or "basic"
//!   signature: { html: "…", text: "…" }  // optional: the mailbox's signature in Neoffice, its default here
//! }], language: "fr",           // the person's language: MailCraft draws in it for « Automatic »
//!   product: "NeoMail" };        // the name the person reads, shown wherever MailCraft names itself
//! ```
//!
//! When the server refuses a token (it expired, or died with the session that asked for it), MailCraft calls
//! `window.mailcraftJmapRenew()` if the page has it: a promise of `{ accounts: [...] }` with new secrets, as above; the
//! mailbox then signs in again, once. An answer that leaves a mailbox out (`{ accounts: [] }` leaves out every one)
//! means the page gives it no more: the mailbox asks for a reload. No answer (a promise that fails, or gives no
//! `accounts`: the page has nothing yet, its own credential being renewed), or one that names the mailbox without a
//! secret, is asked again a minute later, for an hour.
//!
//! A page whose window may not reach other sites itself (Neodesk) offers `window.mailcraftFetchUrl(url)`, a promise of
//! `{ status, contentType, bytes }` (`bytes` a `Uint8Array`): the pictures a message links to, once the person asked to
//! see them, are then fetched through it.
//!
//! The person signed in to the page may be named in the address, as for the document editors (`?author=Jeanne%20Muster`):
//! MailCraft shows their initials at the right end of its top bar.
//!
//! It is read on this page, or on the page that frames it when that one is of the same origin (reading another
//! origin's window throws, which reads as nothing). Nothing secret travels in the address. With host accounts,
//! MailCraft keeps no mail, account or secret between visits (the mail is in this tab's memory, as with the sample
//! mailbox), only the settings and the signatures made here (kept.rs).

use std::collections::HashMap;

use mailcraft_engine::model::AccountConfig;
use mailcraft_engine::store::Store;
use wasm_bindgen::{JsCast as _, JsValue};
use wasm_bindgen_futures::JsFuture;

/// The longest name of the signed-in person taken from the address.
const MAX_AUTHOR_CHARS: usize = 128;
/// The longest signature taken from the page.
const MAX_SIGNATURE_CHARS: usize = 10_000;

/// A mailbox the page handed over.
#[derive(Clone, Debug)]
pub struct HostAccount {
    pub email: String,
    pub name: String,
    pub session: String,
    pub username: String,
    pub secret: String,
    pub bearer: bool,
    /// Its signature in Neoffice, as text (empty: none given).
    pub signature: String,
}

fn get(target: &JsValue, key: &str) -> Option<JsValue> {
    js_sys::Reflect::get(target, &JsValue::from_str(key)).ok().filter(|value| !value.is_undefined() && !value.is_null())
}

fn text(target: &JsValue, key: &str) -> String {
    get(target, key).and_then(|v| v.as_string()).map(|s| s.trim().to_string()).unwrap_or_default()
}

/// The page itself, then the page that frames it when it is of the same origin.
fn windows() -> Vec<JsValue> {
    let Some(window) = web_sys::window() else { return Vec::new() };
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

/// The mailboxes handed over, the incomplete ones left out (at most 20).
pub fn accounts() -> Vec<HostAccount> {
    let Some(host) = windows().iter().find_map(|w| get(w, "mailcraftJmap").filter(JsValue::is_object)) else { return Vec::new() };
    parse(&host)
}

/// The hosting page's answer to `mailcraftJmapRenew()`.
pub struct Renewed {
    /// The complete mailboxes it gives, with their new secrets.
    pub accounts: Vec<HostAccount>,
    /// Every address it names (lower case), complete or not: one named without a secret has none to give yet.
    pub named: Vec<String>,
}

/// New tokens for the mailboxes, from the hosting page's `window.mailcraftJmapRenew()` (a promise of
/// `{ accounts: [...] }`, the same as `mailcraftJmap`'s), when the server refused the ones it had given: the page's
/// answer, which may leave mailboxes out, or name none (`{ accounts: [] }`). `None` when there is no answer: the page
/// offers no such function, its promise fails, or it gives no `accounts`.
pub async fn renew() -> Option<Renewed> {
    let (window, renew) = windows().into_iter().find_map(|w| {
        let f = get(&w, "mailcraftJmapRenew").filter(JsValue::is_function)?;
        Some((w, f))
    })?;
    let promise = js_sys::Function::from(renew).call0(&window).ok()?;
    let answer = JsFuture::from(js_sys::Promise::resolve(&promise)).await.ok()?;
    let list = get(&answer, "accounts").filter(js_sys::Array::is_array)?;
    let named = js_sys::Array::from(&list).iter().take(20).map(|item| text(&item, "email").to_lowercase()).filter(|e| e.contains('@')).collect();
    Some(Renewed { accounts: parse(&answer), named })
}

/// A picture a message links to, fetched by the hosting page's `window.mailcraftFetchUrl(url)` (a promise of
/// `{ status, contentType, bytes }`, `bytes` a `Uint8Array`) for a window that may not reach other sites itself
/// (Neodesk): its bytes when the page got it (status 2xx, at most `max` bytes), else why not. `None` when the page offers
/// no such function: MailCraft fetches it itself.
pub async fn fetch_url(url: &str, max: usize) -> Option<Result<Vec<u8>, String>> {
    let (window, fetch) = windows().into_iter().find_map(|w| {
        let f = get(&w, "mailcraftFetchUrl").filter(JsValue::is_function)?;
        Some((w, js_sys::Function::from(f)))
    })?;
    Some(fetched(&window, &fetch, url, max).await)
}

async fn fetched(window: &JsValue, fetch: &js_sys::Function, url: &str, max: usize) -> Result<Vec<u8>, String> {
    let promise = fetch.call1(window, &JsValue::from_str(url)).map_err(|e| reason(&e))?;
    let answer = JsFuture::from(js_sys::Promise::resolve(&promise)).await.map_err(|e| reason(&e))?;
    match get(&answer, "status").and_then(|s| s.as_f64()).filter(|s| (0.0..1000.0).contains(s)).map(|s| s as u16) {
        Some(status) if (200..300).contains(&status) => {}
        Some(status) => return Err(format!("HTTP {status}")),
        None => return Err("the page could not fetch it".into()),
    }
    // The picture is read from its bytes: `contentType` isn't needed. The page made them in its own window (or Neodesk's
    // bridge did): `instanceof` fails from one window to another, so they are viewed as bytes whatever they are (a
    // Uint8Array or an ArrayBuffer of any window), and nothing else gives any.
    let bytes = get(&answer, "bytes").filter(JsValue::is_object).ok_or("the page gave no picture")?;
    let bytes = js_sys::Uint8Array::new(&bytes);
    if bytes.length() == 0 {
        return Err("the page gave no picture".into());
    }
    // Measured before it is copied in.
    if usize::try_from(bytes.length()).unwrap_or(usize::MAX) > max {
        return Err(format!("the picture is larger than {max} bytes"));
    }
    Ok(bytes.to_vec())
}

fn reason(error: &JsValue) -> String {
    error.as_string().or_else(|| error.dyn_ref::<js_sys::Error>().map(|e| String::from(e.message()))).unwrap_or_else(|| format!("{error:?}"))
}

/// The name of the person signed in to the hosting page, from the address (`?author=Jeanne%20Muster`), as the document
/// editors read it: trimmed, at most 128 characters; `None` when there is none.
pub fn author() -> Option<String> {
    let name: String = query_param("author")?.trim().chars().take(MAX_AUTHOR_CHARS).collect();
    let name = name.trim_end().to_string();
    (!name.is_empty()).then_some(name)
}

/// A `?key=value` of the page's address, decoded as a form is (`+` is a space).
fn query_param(key: &str) -> Option<String> {
    let search = web_sys::window()?.location().search().ok()?;
    web_sys::UrlSearchParams::new_with_str(&search).ok()?.get(key)
}

/// The complete mailboxes of `{ accounts: [...] }` (at most 20).
fn parse(host: &JsValue) -> Vec<HostAccount> {
    let Some(list) = get(host, "accounts").filter(js_sys::Array::is_array) else { return Vec::new() };
    let mut out: Vec<HostAccount> = Vec::new();
    for item in js_sys::Array::from(&list).iter().take(20) {
        let account = HostAccount {
            email: text(&item, "email"),
            name: text(&item, "name"),
            session: text(&item, "session"),
            username: text(&item, "username"),
            secret: text(&item, "secret"),
            bearer: text(&item, "scheme").eq_ignore_ascii_case("bearer"),
            signature: get(&item, "signature").map(|s| signature(&s)).unwrap_or_default(),
        };
        let complete = account.email.contains('@') && account.session.starts_with("https://") && !account.secret.is_empty();
        if complete && !out.iter().any(|a| a.email.eq_ignore_ascii_case(&account.email)) {
            out.push(account);
        }
    }
    out
}

/// A mailbox's signature as the page gives it (`{ html, text }`), as text: its text, else its HTML made text (MailCraft's
/// signatures are text), at most MAX_SIGNATURE_CHARS characters.
fn signature(given: &JsValue) -> String {
    let plain = get(given, "text").and_then(|v| v.as_string()).unwrap_or_default();
    let plain = if plain.trim().is_empty() {
        get(given, "html").and_then(|v| v.as_string()).map(|h| mailcraft_engine::mime::html_to_text(&h)).unwrap_or_default()
    } else {
        plain
    };
    plain.trim().chars().take(MAX_SIGNATURE_CHARS).collect()
}

/// The product's name the person reads (Neoffice's "NeoMail"), if the page gives one.
pub fn product() -> Option<String> {
    let host = windows().iter().find_map(|w| get(w, "mailcraftJmap").filter(JsValue::is_object))?;
    Some(text(&host, "product")).filter(|name| !name.is_empty())
}

/// The language of the person the page is for (a tag such as `fr` or `de-CH`), if it says one.
pub fn language() -> Option<String> {
    let host = windows().iter().find_map(|w| get(w, "mailcraftJmap").filter(JsValue::is_object))?;
    Some(text(&host, "language")).filter(|tag| !tag.is_empty() && tag.len() <= 35)
}

/// A store in memory holding these mailboxes' accounts (`a1`, `a2`…), before the session opens it, so each starts once;
/// each with the page's signature and name, or the ones made here that replaced them (`signatures`, `names`, by address
/// in lower case).
pub fn store(accounts: &[HostAccount], signatures: &HashMap<String, crate::kept::Kept>, names: &HashMap<String, crate::kept::Kept>) -> Store {
    let store = Store::memory();
    let configs: Vec<AccountConfig> = accounts
        .iter()
        .enumerate()
        .map(|(i, a)| AccountConfig {
            id: format!("a{}", i + 1),
            name: crate::kept::chosen(&a.name, names.get(&a.email.to_lowercase())),
            email: a.email.clone(),
            username: if a.username.is_empty() { a.email.clone() } else { a.username.clone() },
            jmap: Some(a.session.clone()),
            signature: crate::kept::chosen(&a.signature, signatures.get(&a.email.to_lowercase())),
            ..AccountConfig::default()
        })
        .collect();
    if let Err(e) = store.save_accounts(&configs) {
        log::error!("the mailboxes of the page couldn't be set up: {e}");
    }
    store
}

/// The browser's time zone, by its IANA name (`Intl.DateTimeFormat().resolvedOptions().timeZone`): new and changed times
/// of events are written in it (mailcraft-jmap's calendars.rs). Empty when the browser doesn't say.
pub fn time_zone() -> String {
    let format = js_sys::Intl::DateTimeFormat::new(&js_sys::Array::new(), &js_sys::Object::new());
    get(&format.resolved_options(), "timeZone").and_then(|z| z.as_string()).unwrap_or_default()
}

/// The local time (seconds, as if UTC) of the instant `t` in a zone, the browser's for `None`, by the browser's time-zone
/// rules (MailCraft has none); `None` for a zone the browser doesn't know.
pub fn local_time(zone: Option<&str>, t: i64) -> Option<i64> {
    let date = js_sys::Date::new(&JsValue::from_f64(t as f64 * 1000.0));
    let Some(zone) = zone else {
        // Minutes behind UTC.
        let offset = date.get_timezone_offset();
        return (!offset.is_nan()).then(|| t - offset as i64 * 60);
    };
    let options = js_sys::Object::new();
    for (key, value) in [
        ("timeZone", zone),
        ("hourCycle", "h23"),
        ("year", "numeric"),
        ("month", "2-digit"),
        ("day", "2-digit"),
        ("hour", "2-digit"),
        ("minute", "2-digit"),
        ("second", "2-digit"),
    ] {
        js_sys::Reflect::set(&options, &JsValue::from_str(key), &JsValue::from_str(value)).ok()?;
    }
    let intl = js_sys::Reflect::get(&js_sys::global(), &JsValue::from_str("Intl")).ok()?;
    let make = js_sys::Reflect::get(&intl, &JsValue::from_str("DateTimeFormat")).ok()?.dyn_into::<js_sys::Function>().ok()?;
    // A zone the browser doesn't know throws (a RangeError): caught, as none.
    let format = js_sys::Reflect::construct(&make, &js_sys::Array::of2(&JsValue::from_str("en-US"), &options)).ok()?;
    let parts = get(&format, "formatToParts")?.dyn_into::<js_sys::Function>().ok()?.call1(&format, &date).ok()?;
    let mut civil = [0i64; 6];
    for part in js_sys::Array::from(&parts).iter() {
        let at = match text(&part, "type").as_str() {
            "year" => 0,
            "month" => 1,
            "day" => 2,
            "hour" => 3,
            "minute" => 4,
            "second" => 5,
            _ => continue,
        };
        if let Some(slot) = civil.get_mut(at) {
            *slot = text(&part, "value").parse().ok()?;
        }
    }
    let [year, month, day, hour, minute, second] = civil;
    let days = mailcraft_engine::model::dates::days_from_civil(year, u32::try_from(month).ok()?, u32::try_from(day).ok()?);
    Some(days * 86_400 + hour * 3600 + minute * 60 + second)
}

/// What each mailbox signs in with: its address (lower case) → (secret, Bearer rather than Basic).
pub fn secrets(accounts: &[HostAccount]) -> HashMap<String, (String, bool)> {
    accounts.iter().map(|a| (a.email.to_lowercase(), (a.secret.clone(), a.bearer))).collect()
}
