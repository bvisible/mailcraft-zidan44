// //// Neoffice — added file (no upstream equivalent): the mailboxes a hosting page hands MailCraft (maintenance#1630).

//! The mailboxes a hosting page hands MailCraft, so that it opens them without a form or a password.
//!
//! The page (Neoffice's `/mailcraft`, which frames the browser build) sets, before MailCraft starts:
//!
//! ```js
//! window.mailcraftJmap = { accounts: [{
//!   email: "jane@example.com",   // the mailbox, MailCraft's account key
//!   name: "Jane Doe",            // the name on sent mail
//!   session: "https://…",        // its JMAP session URL
//!   username: "jane@example.com",
//!   secret: "<token>",           // kept in this tab's memory only, never stored
//!   scheme: "bearer"             // or "basic"
//! }], language: "fr",           // the person's language: MailCraft draws in it for « Automatic »
//!   product: "NeoMail" };        // the name the person reads, shown wherever MailCraft names itself
//! ```
//!
//! It is read on this page, or on the page that frames it when that one is of the same origin (reading another
//! origin's window throws, which reads as nothing). Nothing secret travels in the address. With host accounts,
//! MailCraft keeps nothing between visits: the mail is in this tab's memory, as with the sample mailbox.

use std::collections::HashMap;

use mailcraft_engine::model::AccountConfig;
use mailcraft_engine::store::Store;
use wasm_bindgen::JsValue;

/// A mailbox the page handed over.
#[derive(Clone, Debug)]
pub struct HostAccount {
    pub email: String,
    pub name: String,
    pub session: String,
    pub username: String,
    pub secret: String,
    pub bearer: bool,
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
    let Some(list) = get(&host, "accounts").filter(js_sys::Array::is_array) else { return Vec::new() };
    let mut out: Vec<HostAccount> = Vec::new();
    for item in js_sys::Array::from(&list).iter().take(20) {
        let account = HostAccount {
            email: text(&item, "email"),
            name: text(&item, "name"),
            session: text(&item, "session"),
            username: text(&item, "username"),
            secret: text(&item, "secret"),
            bearer: text(&item, "scheme").eq_ignore_ascii_case("bearer"),
        };
        let complete = account.email.contains('@') && account.session.starts_with("https://") && !account.secret.is_empty();
        if complete && !out.iter().any(|a| a.email.eq_ignore_ascii_case(&account.email)) {
            out.push(account);
        }
    }
    out
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

/// A store in memory holding these mailboxes' accounts (`a1`, `a2`…), before the session opens it, so each starts once.
pub fn store(accounts: &[HostAccount]) -> Store {
    let store = Store::memory();
    let configs: Vec<AccountConfig> = accounts
        .iter()
        .enumerate()
        .map(|(i, a)| AccountConfig {
            id: format!("a{}", i + 1),
            name: a.name.clone(),
            email: a.email.clone(),
            username: if a.username.is_empty() { a.email.clone() } else { a.username.clone() },
            jmap: Some(a.session.clone()),
            ..AccountConfig::default()
        })
        .collect();
    if let Err(e) = store.save_accounts(&configs) {
        log::error!("the mailboxes of the page couldn't be set up: {e}");
    }
    store
}

/// What each mailbox signs in with: its address (lower case) → (secret, Bearer rather than Basic).
pub fn secrets(accounts: &[HostAccount]) -> HashMap<String, (String, bool)> {
    accounts.iter().map(|a| (a.email.to_lowercase(), (a.secret.clone(), a.bearer))).collect()
}
