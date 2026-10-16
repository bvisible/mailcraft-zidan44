// //// Neoffice — added file (no upstream equivalent): Nora, the assistant of the Neoffice server, in MailCraft.

//! Nora, the assistant of the Neoffice server, in MailCraft: « Reply with Nora » writes the answer
//! to the open message, « Translate » shows it in French, German or English, and the band of the
//! message being written corrects, improves or translates the person's own text.
//!
//! Nora lives on the server. The app reaches her only when a Neoffice page hosts it (the browser
//! build opened with `?nora`), which [`crate::Services::nora_available`] says. A call names a method
//! of the server and its arguments; the host sends it and hands the answer back a frame or more
//! later ([`crate::Services::nora_answers`]).
//!
//! The open message is untrusted: the server runs its injection guard on it before any model reads
//! it (`generate_email` and its `reply_context`, `translate`). Nothing is sent: a draft goes into the
//! reply being written, above the signature and the quoted message, as one step Undo takes back; a
//! translation is shown in place of the message, with the way back to the original.

use egui::{RichText, Ui};
use mailcraft_engine::mime::Parsed;
use mailcraft_engine::model::MessageKey;
use serde_json::{Value, json};

use crate::icons::Icon;
use crate::theme::{self, Tokens};
use crate::{Action, MailApp, UiState, widgets};

/// The server method that writes an email body from an instruction and the email it answers.
const REPLY_METHOD: &str = "nora.api.nora_webmail.generate_email";

/// The server method that translates an email (French, German, English).
const TRANSLATE_METHOD: &str = "nora.api.nora_webmail.translate";

/// The server methods that rewrite the text being written, and the field of their answer.
const CORRECT: (&str, &str) = ("nora.api.nora_webmail.proofread", "corrected_text");
const IMPROVE: (&str, &str) = ("nora.api.nora_webmail.improve", "improved_text");

/// What Nora is asked along with the received message. The model reads instructions in English;
/// the reply is written in the language of the message it answers.
const INSTRUCTION: &str = "Write the reply to the email being replied to, in the language that email is written in. \
Answer what it asks; when information is missing, say that it will be checked.";

/// Most characters of the received message sent along (the server keeps 4 000 of them for a reply).
const CONTEXT_MAX: usize = 6_000;

/// The languages Translate offers: code, and the English name the interface translates.
const LANGUAGES: [(&str, &str); 3] = [("fr", "French"), ("de", "German"), ("en", "English")];

/// One call to the Neoffice server: a whitelisted method and its arguments.
#[derive(Clone, Debug, PartialEq)]
pub struct NoraCall {
    pub id: u64,
    pub method: &'static str,
    pub args: Value,
}

/// The server's answer to a call: its HTTP status (0 when the network gave none, the body then
/// saying why) and its body.
#[derive(Clone, Debug, PartialEq)]
pub struct NoraAnswer {
    pub id: u64,
    pub status: u16,
    pub body: String,
}

/// What the person asks of Nora.
#[derive(Clone, Debug, PartialEq)]
pub enum Ask {
    /// Write the reply to the open message (to everyone with `all`).
    Reply { all: bool },
    /// Show the open message in this language (`fr`, `de`, `en`).
    Translate(&'static str),
    /// Back to the open message as it came.
    Original,
    /// Correct the spelling and grammar of the text being written.
    Correct,
    /// Make the text being written clearer.
    Improve,
    /// Translate the text being written into this language.
    TranslateDraft(&'static str),
}

/// What an answer is for.
#[derive(Clone, Debug, PartialEq)]
enum Waiting {
    /// The draft of the reply being written (its compose id).
    Reply(u64),
    /// The translation of a message into a language.
    Translation(MessageKey, &'static str),
    /// A new version of the person's own text in a message being written: the compose id, the
    /// text sent (the answer goes in only over that same text) and the field the answer carries.
    Rewrite { compose: u64, sent: String, field: &'static str },
}

/// What the interface knows of Nora.
#[derive(Default)]
pub(crate) struct NoraState {
    /// A Neoffice page hosts the app: Nora can be asked.
    pub(crate) available: bool,
    last: u64,
    /// The calls waiting for their answer.
    waiting: Vec<(u64, Waiting)>,
    /// A message as Nora translated it: its key, the language and the text.
    translation: Option<(MessageKey, &'static str, String)>,
}

impl NoraState {
    fn asked(&self, what: fn(&Waiting) -> bool) -> bool {
        self.waiting.iter().any(|(_, w)| what(w))
    }
}

/// The reading pane's Nora button, beside Reply, Reply all and Forward.
pub(crate) fn reading_button(ui: &mut Ui, t: &Tokens, acts: &mut Vec<Action>) {
    let nora = widgets::pill_button(ui, Icon::Reply, "Nora", t).on_hover_text(tl!("Nora, the Neoffice assistant"));
    egui::Popup::menu(&nora).show(|ui| {
        if ui.button(tl!("Reply with Nora")).clicked() {
            acts.push(Action::Nora(Ask::Reply { all: false }));
            ui.close();
        }
        if ui.button(tl!("Reply all with Nora")).clicked() {
            acts.push(Action::Nora(Ask::Reply { all: true }));
            ui.close();
        }
        ui.separator();
        ui.menu_button(tl!("Translate"), |ui| {
            for (code, name) in LANGUAGES {
                if ui.button(tl!(name)).clicked() {
                    acts.push(Action::Nora(Ask::Translate(code)));
                    ui.close();
                }
            }
        });
    });
}

/// Do what the person asked.
pub(crate) fn act(app: &mut MailApp, ask: Ask) {
    match ask {
        Ask::Reply { all } => reply(app, all),
        Ask::Translate(lang) => translate(app, lang),
        Ask::Original => app.ui.nora.translation = None,
        Ask::Correct => rewrite(app, CORRECT, None),
        Ask::Improve => rewrite(app, IMPROVE, None),
        Ask::TranslateDraft(lang) => rewrite(app, (TRANSLATE_METHOD, "translated_text"), Some(lang)),
    }
}

/// The band of the message being written: Nora corrects, improves or translates the person's text.
pub(crate) fn compose_band(ui: &mut Ui, t: &Tokens, acts: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(RichText::new(tl!("Nora")).color(t.dim).small());
        if ui.small_button(tl!("Correct")).on_hover_text(tl!("Spelling and grammar of your text")).clicked() {
            acts.push(Action::Nora(Ask::Correct));
        }
        if ui.small_button(tl!("Improve")).on_hover_text(tl!("A clearer version of your text")).clicked() {
            acts.push(Action::Nora(Ask::Improve));
        }
        ui.menu_button(RichText::new(tl!("Translate")).small(), |ui| {
            for (code, name) in LANGUAGES {
                if ui.button(tl!(name)).clicked() {
                    acts.push(Action::Nora(Ask::TranslateDraft(code)));
                    ui.close();
                }
            }
        });
    });
}

/// Ask Nora for a new version of the person's own text (above the signature and the quoted
/// message) of the message being written.
fn rewrite(app: &mut MailApp, (method, field): (&'static str, &'static str), lang: Option<&'static str>) {
    if app.ui.nora.asked(|w| matches!(w, Waiting::Rewrite { .. })) {
        app.session.notice(tl!("Nora is already rewriting the text.").to_string(), false);
        return;
    }
    let Some(c) = app.session.compose.as_ref() else { return };
    let sent = own_text(c);
    if sent.trim().is_empty() {
        app.session.notice(tl!("Write your text first: Nora works on it.").to_string(), false);
        return;
    }
    let mut args = json!({"html_content": as_html(&sent), "account_name": mailbox(app, &c.account)});
    if let Some(lang) = lang {
        args["target_language"] = json!(lang);
    }
    let compose = c.id;
    send(app, method, args, Waiting::Rewrite { compose, sent, field });
    app.session.notice(tl!("Nora is rewriting the text…").to_string(), false);
}

/// The person's own text in a message being written: what comes before the signature (`-- `) and
/// the quoted message (its ruled paragraph), without the blank lines that end it.
fn own_text(c: &mailcraft_engine::Compose) -> String {
    let lines: Vec<&str> = c.body.split('\n').collect();
    let mut own: Vec<&str> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if *line == "-- " || c.styles.para(i).rule {
            break;
        }
        own.push(line);
    }
    own.join("\n").trim_end().to_string()
}

fn send(app: &mut MailApp, method: &'static str, args: Value, waiting: Waiting) {
    app.ui.nora.last += 1;
    let id = app.ui.nora.last;
    app.ui.nora.waiting.push((id, waiting));
    app.services.nora_send(NoraCall { id, method, args });
}

/// Start the reply (to everyone, with `all`) and ask Nora to write it.
fn reply(app: &mut MailApp, all: bool) {
    if app.ui.nora.asked(|w| matches!(w, Waiting::Reply(_))) {
        app.session.notice(tl!("Nora is already writing a reply.").to_string(), false);
        return;
    }
    let Some((account, context)) = app.session.reading.as_ref().and_then(|r| Some((r.key.account.clone(), reply_context(r.parsed.as_ref()?)))) else {
        return;
    };
    let before = app.session.compose.as_ref().map(|c| c.id);
    app.run(if all { "mail.replyAll" } else { "mail.reply" }, json!({}));
    // The reply didn't open (the message is still downloading, say): nothing to write into.
    let Some(compose) = app.session.compose.as_ref().map(|c| c.id).filter(|id| Some(*id) != before) else {
        return;
    };
    let mailbox = mailbox(app, &account);
    send(app, REPLY_METHOD, json!({"prompt": INSTRUCTION, "account_name": mailbox, "reply_context": context}), Waiting::Reply(compose));
    app.session.notice(tl!("Nora is writing the reply…").to_string(), false);
}

/// The mailbox of the Neoffice server an account of MailCraft is (one the hosting page handed over), for Nora to
/// write in its style and check the person may open it; empty for any other account (the sample mailbox).
fn mailbox(app: &MailApp, account: &str) -> String {
    app.session.account(account).map(|a| a.config.email.clone()).filter(|email| app.services.nora_mailbox(email)).unwrap_or_default()
}

/// The received message as the server reads it: who wrote it, its subject and its text.
fn reply_context(p: &Parsed) -> String {
    let name = p.from.name.trim();
    let from = if name.is_empty() { p.from.email.clone() } else { format!("{name} <{}>", p.from.email) };
    let text: String = p.text.chars().take(CONTEXT_MAX).collect();
    format!("From: {from}\nSubject: {}\n\n{}", p.subject.trim(), text.trim())
}

/// Ask Nora for the open message in `lang`.
fn translate(app: &mut MailApp, lang: &'static str) {
    if app.ui.nora.asked(|w| matches!(w, Waiting::Translation(..))) {
        app.session.notice(tl!("Nora is already translating a message.").to_string(), false);
        return;
    }
    let Some((key, text)) = app.session.reading.as_ref().and_then(|r| Some((r.key.clone(), r.parsed.as_ref()?.text.clone()))) else {
        return;
    };
    let html = as_html(&text.chars().take(CONTEXT_MAX).collect::<String>());
    send(app, TRANSLATE_METHOD, json!({"html_content": html, "target_language": lang, "account_name": ""}), Waiting::Translation(key, lang));
    app.session.notice(tl!("Nora is translating the message…").to_string(), false);
}

/// Plain text as the HTML the server reads: paragraphs, line breaks, nothing it could take for markup.
fn as_html(text: &str) -> String {
    let escape = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    text.split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| format!("<p>{}</p>", p.lines().map(escape).collect::<Vec<_>>().join("<br>")))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The answers that came back: a draft goes into the reply that waits for it, a translation is
/// shown in place of its message.
pub(crate) fn pump(app: &mut MailApp) {
    for answer in app.services.nora_answers() {
        let Some(at) = app.ui.nora.waiting.iter().position(|(id, _)| *id == answer.id) else { continue };
        let (_, waiting) = app.ui.nora.waiting.remove(at);
        match waiting {
            Waiting::Reply(compose) => match draft(&answer) {
                Ok(text) => put_in_reply(app, compose, &text),
                Err(why) => app.session.notice(why, true),
            },
            Waiting::Rewrite { compose, sent, field } => match rewritten(&answer, field) {
                Ok(Some(text)) => put_over_own_text(app, compose, &sent, &text),
                Ok(None) => app.session.notice(tl!("Nora found nothing to change.").to_string(), false),
                Err(why) => app.session.notice(why, true),
            },
            Waiting::Translation(key, lang) => match translation(&answer, lang) {
                Ok(Some(text)) => app.ui.nora.translation = Some((key, lang, text)),
                Ok(None) => app.session.notice(tf!("The message is already in {language}.", language = tl!(language(lang))), false),
                Err(why) => app.session.notice(why, true),
            },
        }
    }
}

/// The English name of a language Translate offers.
fn language(code: &str) -> &'static str {
    LANGUAGES.iter().find(|(c, _)| *c == code).map_or("English", |(_, name)| *name)
}

/// The draft an answer carries, or what to tell the person (in their language).
fn draft(answer: &NoraAnswer) -> Result<String, String> {
    if answer.status == 0 {
        return Err(tf!("Nora couldn't be reached: {reason}", reason = answer.body.trim()));
    }
    let body: Value = serde_json::from_str(&answer.body).unwrap_or(Value::Null);
    let text = body.get("message").and_then(|m| m.get("draft_text")).and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty());
    match text {
        Some(text) if answer.status == 200 => Ok(text.to_string()),
        _ => Err(tf!("Nora couldn't write the reply: {reason}", reason = reason(&body, answer.status))),
    }
}

/// The translation an answer carries (`None` when the message already is in that language), or
/// what to tell the person.
fn translation(answer: &NoraAnswer, lang: &str) -> Result<Option<String>, String> {
    if answer.status == 0 {
        return Err(tf!("Nora couldn't be reached: {reason}", reason = answer.body.trim()));
    }
    let body: Value = serde_json::from_str(&answer.body).unwrap_or(Value::Null);
    let message = body.get("message").filter(|_| answer.status == 200);
    if message.and_then(|m| m.get("has_changes")).and_then(Value::as_bool) == Some(false)
        && message.and_then(|m| m.get("source_language")).and_then(Value::as_str) == Some(lang)
    {
        return Ok(None);
    }
    match message.and_then(|m| m.get("translated_text")).and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty()) {
        Some(text) => Ok(Some(text.to_string())),
        None => Err(tf!("Nora couldn't translate the message: {reason}", reason = reason(&body, answer.status))),
    }
}

/// The new text an answer carries (`None` when Nora changed nothing), or what to tell the person.
fn rewritten(answer: &NoraAnswer, field: &str) -> Result<Option<String>, String> {
    if answer.status == 0 {
        return Err(tf!("Nora couldn't be reached: {reason}", reason = answer.body.trim()));
    }
    let body: Value = serde_json::from_str(&answer.body).unwrap_or(Value::Null);
    let message = body.get("message").filter(|_| answer.status == 200);
    if message.and_then(|m| m.get("has_changes")).and_then(Value::as_bool) == Some(false) {
        return Ok(None);
    }
    match message.and_then(|m| m.get(field)).and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty()) {
        Some(text) => Ok(Some(text.to_string())),
        None => Err(tf!("Nora couldn't rewrite the text: {reason}", reason = reason(&body, answer.status))),
    }
}

/// Why the server said no: the message it shows (Frappe's `_server_messages`), else the last part
/// of its exception, else the HTTP status.
fn reason(body: &Value, status: u16) -> String {
    let shown = body
        .get("_server_messages")
        .and_then(Value::as_str)
        .and_then(|list| serde_json::from_str::<Vec<String>>(list).ok())
        .and_then(|list| list.iter().find_map(|m| serde_json::from_str::<Value>(m).ok()?.get("message")?.as_str().map(plain)))
        .filter(|m| !m.is_empty());
    let exception =
        || body.get("exception").and_then(Value::as_str).map(|e| e.rsplit(": ").next().unwrap_or(e).trim().to_string()).filter(|e| !e.is_empty());
    shown.or_else(exception).unwrap_or_else(|| format!("HTTP {status}"))
}

/// A server message without its HTML tags.
fn plain(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut tag = false;
    for c in html.chars() {
        match c {
            '<' => tag = true,
            '>' => tag = false,
            c if !tag => out.push(c),
            _ => {}
        }
    }
    out.trim().to_string()
}

/// Put Nora's draft at the top of the reply it was asked for, while that reply is being written.
fn put_in_reply(app: &mut MailApp, compose: u64, text: &str) {
    let Some(c) = app.session.compose.as_mut().filter(|c| c.id == compose && !c.sending) else {
        app.session.notice(tl!("The reply was closed before Nora's draft came back.").to_string(), false);
        return;
    };
    crate::rich_edit::insert_at_start(&mut app.ui.editor, c, text);
    app.session.notice(tl!("Nora's draft is in the reply: read it before sending.").to_string(), false);
}

/// Put Nora's version over the person's own text, if that text is still what was sent (anything
/// typed meanwhile wins), as one step Undo takes back.
fn put_over_own_text(app: &mut MailApp, compose: u64, sent: &str, text: &str) {
    let Some(c) = app.session.compose.as_mut().filter(|c| c.id == compose && !c.sending) else {
        app.session.notice(tl!("The message was closed before Nora's version came back.").to_string(), false);
        return;
    };
    if own_text(c) != sent {
        app.session.notice(tl!("Your text changed meanwhile: Nora's version wasn't put in.").to_string(), false);
        return;
    }
    crate::rich_edit::replace_start(&mut app.ui.editor, c, sent.chars().count(), text);
    app.session.notice(tl!("Nora's version is in the message: Undo takes it back.").to_string(), false);
}

/// The open message as Nora translated it, in place of the message: `true` when it was drawn.
pub(crate) fn translated_body(st: &UiState, ui: &mut Ui, t: &Tokens, key: &MessageKey, acts: &mut Vec<Action>) -> bool {
    let Some((_, lang, text)) = st.nora.translation.as_ref().filter(|(k, _, _)| k == key) else { return false };
    ui.horizontal(|ui| {
        ui.label(RichText::new(tf!("Translated by Nora into {language}", language = tl!(language(lang)))).color(t.dim).small());
        if ui.link(RichText::new(tl!("Show the original")).small()).clicked() {
            acts.push(Action::Nora(Ask::Original));
        }
    });
    ui.add_space(8.0);
    let mut shown = text.as_str();
    ui.add(egui::TextEdit::multiline(&mut shown).frame(egui::Frame::NONE).desired_width(f32::INFINITY).font(theme::regular(14.5)).text_color(t.text));
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(status: u16, body: Value) -> NoraAnswer {
        NoraAnswer { id: 1, status, body: body.to_string() }
    }

    #[test]
    fn a_draft_comes_out_of_the_server_answer() {
        let ok = answer(200, json!({"message": {"success": true, "draft_text": "  Bonjour Ann,\n\nC'est noté.  ", "draft_html": "<p>…</p>"}}));
        assert_eq!(draft(&ok), Ok("Bonjour Ann,\n\nC'est noté.".to_string()));
    }

    #[test]
    fn a_refusal_says_what_the_server_said() {
        // What frappe.throw sends: the message the desk would show, in a JSON list of JSON objects.
        let shown = json!({"message": "Nora refused to process this message: it contains <b>instructions</b>."}).to_string();
        let refused = answer(417, json!({"exc_type": "ValidationError", "_server_messages": json!([shown]).to_string()}));
        let why = draft(&refused).err().unwrap_or_default();
        assert!(why.contains("Nora refused to process this message: it contains instructions."), "{why}");
        let crashed = answer(500, json!({"exception": "frappe.exceptions.ValidationError: Access denied"}));
        assert!(draft(&crashed).err().unwrap_or_default().ends_with("Access denied"));
        assert!(draft(&answer(502, json!(null))).err().unwrap_or_default().ends_with("HTTP 502"));
        let empty = answer(200, json!({"message": {"draft_text": "  "}}));
        assert!(draft(&empty).is_err(), "an empty draft is not a draft");
    }

    #[test]
    fn no_network_is_said_as_such() {
        let lost = NoraAnswer { id: 1, status: 0, body: "Failed to fetch".into() };
        assert_eq!(draft(&lost), Err("Nora couldn't be reached: Failed to fetch".to_string()));
    }

    #[test]
    fn the_context_carries_sender_subject_and_text() {
        let p = Parsed {
            subject: " Your proof is ready ".into(),
            from: mailcraft_engine::model::EmailAddress { name: "Print Shop".into(), email: "orders@printshop.example".into() },
            text: "Hello,\nThe proof is ready.\n".into(),
            ..Default::default()
        };
        assert_eq!(reply_context(&p), "From: Print Shop <orders@printshop.example>\nSubject: Your proof is ready\n\nHello,\nThe proof is ready.");
    }

    use std::cell::RefCell;
    use std::path::{Path, PathBuf};
    use std::rc::Rc;

    use mailcraft_engine::model::dates::Clock;
    use mailcraft_engine::store::Store;
    use mailcraft_engine::{Session, sample};

    use crate::{MailApp, NullBackend, Services};

    const NOW: i64 = 1_791_979_200;

    /// A host that reaches Nora: what the app sends is kept, the answers are the test's.
    #[derive(Default, Clone)]
    struct Host {
        sent: Rc<RefCell<Vec<NoraCall>>>,
        inbox: Rc<RefCell<Vec<NoraAnswer>>>,
        /// A mailbox of the server, as a hosting page hands one over.
        mailbox: Option<String>,
    }

    impl Services for Host {
        fn clock(&self) -> (i64, i32) {
            (NOW, 0)
        }
        fn pick_files(&mut self) -> Vec<PathBuf> {
            Vec::new()
        }
        fn save_path(&mut self, _: &str) -> Option<PathBuf> {
            None
        }
        fn open_path(&mut self, _: &Path) -> Result<(), String> {
            Err("not available".into())
        }
        fn nora_available(&self) -> bool {
            true
        }
        fn nora_send(&mut self, call: NoraCall) {
            self.sent.borrow_mut().push(call);
        }
        fn nora_answers(&mut self) -> Vec<NoraAnswer> {
            std::mem::take(&mut self.inbox.borrow_mut())
        }
        fn nora_mailbox(&self, email: &str) -> bool {
            self.mailbox.as_deref().is_some_and(|m| m.eq_ignore_ascii_case(email))
        }
    }

    fn frame(ctx: &egui::Context, app: &mut MailApp) {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0));
        let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ui| app.ui(ui));
        out.textures_delta.clear();
    }

    /// The sample mailbox in a page that reaches Nora, with the print shop's message open.
    fn hosted() -> (MailApp, Host, egui::Context) {
        let store = Store::memory();
        sample::seed_if_empty(&store, NOW).unwrap();
        let host = Host::default();
        let mut app = MailApp::new(Session::open(Some(store), Clock::new(NOW, 0)), Box::new(NullBackend), Box::new(host.clone()));
        let ctx = egui::Context::default();
        frame(&ctx, &mut app);
        let key = app.session.list.iter().find(|m| m.subject.starts_with("Your proof")).map(|m| m.key.clone()).unwrap();
        app.session.open_message(&key);
        frame(&ctx, &mut app);
        assert!(app.ui.nora.available);
        (app, host, ctx)
    }

    fn answers(host: &Host, id: u64, status: u16, body: Value) {
        host.inbox.borrow_mut().push(NoraAnswer { id, status, body: body.to_string() });
    }

    #[test]
    fn reply_with_nora_puts_her_draft_above_the_signature_and_the_quote() {
        let (mut app, host, ctx) = hosted();
        app.act(Action::Nora(Ask::Reply { all: false }));
        frame(&ctx, &mut app);
        let call = host.sent.borrow().first().cloned().unwrap();
        assert_eq!(call.method, "nora.api.nora_webmail.generate_email");
        assert_eq!(call.args["account_name"], "");
        let context = call.args["reply_context"].as_str().unwrap_or_default();
        assert!(
            context.starts_with("From: Print Shop <orders@printshop.example>\nSubject: Your proof for the festival poster is ready"),
            "{context}"
        );
        assert!(context.contains("The proof for order 4471"), "{context}");
        let before = app.session.compose.as_ref().map(|c| c.body.clone()).unwrap();
        assert!(before.starts_with('\n'), "a reply starts on an empty line: {before:?}");

        // A second click while she writes asks nothing more.
        app.act(Action::Nora(Ask::Reply { all: false }));
        assert_eq!(host.sent.borrow().len(), 1);

        answers(
            &host,
            call.id,
            200,
            json!({"message": {"success": true, "draft_text": "Bonjour,\n\nC'est approuvé, merci.\n\nMeilleures salutations"}}),
        );
        frame(&ctx, &mut app);
        let c = app.session.compose.as_ref().unwrap();
        assert_eq!(c.body, format!("Bonjour,\n\nC'est approuvé, merci.\n\nMeilleures salutations{before}"), "the draft, then the reply as it was");
        assert!(c.body.contains("Riley Moreno · Studio Lantern") && c.body.contains("\nFrom: Print Shop <orders@printshop.example>\nSent: "));
        assert_eq!(c.styles.chars.len(), c.body.chars().count(), "the styles follow the text");
        assert_eq!(
            app.session.notices.last().map(|n| (n.text.as_str(), n.error)),
            Some(("Nora's draft is in the reply: read it before sending.", false))
        );
        assert!(app.ui.nora.waiting.is_empty());
    }

    #[test]
    fn a_refusal_leaves_the_reply_as_it_was() {
        let (mut app, host, ctx) = hosted();
        app.act(Action::Nora(Ask::Reply { all: true }));
        frame(&ctx, &mut app);
        let id = host.sent.borrow().first().map(|c| c.id).unwrap();
        let before = app.session.compose.as_ref().map(|c| c.body.clone()).unwrap();
        let shown = json!({"message": "Nora refused to process this message"}).to_string();
        answers(&host, id, 417, json!({"_server_messages": json!([shown]).to_string()}));
        frame(&ctx, &mut app);
        assert_eq!(app.session.compose.as_ref().map(|c| c.body.clone()), Some(before));
        let last = app.session.notices.last().map(|n| (n.text.clone(), n.error)).unwrap();
        assert_eq!(last, ("Nora couldn't write the reply: Nora refused to process this message".to_string(), true));
        // She can be asked again.
        app.act(Action::Nora(Ask::Reply { all: false }));
        assert_eq!(host.sent.borrow().len(), 2);
    }

    #[test]
    fn a_draft_for_a_reply_that_was_closed_goes_nowhere() {
        let (mut app, host, ctx) = hosted();
        app.act(Action::Nora(Ask::Reply { all: false }));
        frame(&ctx, &mut app);
        let id = host.sent.borrow().first().map(|c| c.id).unwrap();
        app.run("compose.discard", json!({}));
        answers(&host, id, 200, json!({"message": {"draft_text": "Bonjour"}}));
        frame(&ctx, &mut app);
        assert!(app.session.compose.is_none());
        assert_eq!(app.session.notices.last().map(|n| n.text.as_str()), Some("The reply was closed before Nora's draft came back."));
    }

    #[test]
    fn translate_shows_the_message_in_the_language_asked_then_the_original() {
        let (mut app, host, ctx) = hosted();
        let key = app.session.reading.as_ref().map(|r| r.key.clone()).unwrap();
        app.act(Action::Nora(Ask::Translate("fr")));
        let call = host.sent.borrow().last().cloned().unwrap();
        assert_eq!(call.method, "nora.api.nora_webmail.translate");
        assert_eq!((call.args["target_language"].as_str(), call.args["account_name"].as_str()), (Some("fr"), Some("")));
        let html = call.args["html_content"].as_str().unwrap_or_default();
        assert!(html.starts_with("<p>") && html.contains("The proof for order 4471"), "{html}");
        assert!(!html.contains("<script"), "the text can't turn into markup");

        // A second ask while she translates asks nothing more.
        app.act(Action::Nora(Ask::Translate("de")));
        assert_eq!(host.sent.borrow().len(), 1);

        answers(
            &host,
            call.id,
            200,
            json!({"message": {"success": true, "translated_text": "Bonjour,\n\nLe bon à tirer est prêt.", "source_language": "en", "target_language": "fr", "has_changes": true}}),
        );
        frame(&ctx, &mut app);
        assert_eq!(app.ui.nora.translation, Some((key.clone(), "fr", "Bonjour,\n\nLe bon à tirer est prêt.".to_string())));
        frame(&ctx, &mut app);
        app.act(Action::Nora(Ask::Original));
        assert!(app.ui.nora.translation.is_none());
    }

    #[test]
    fn a_message_already_in_that_language_is_said_so() {
        let (mut app, host, ctx) = hosted();
        app.act(Action::Nora(Ask::Translate("en")));
        let id = host.sent.borrow().last().map(|c| c.id).unwrap();
        answers(
            &host,
            id,
            200,
            json!({"message": {"translated_text": "Hello", "source_language": "en", "target_language": "en", "has_changes": false}}),
        );
        frame(&ctx, &mut app);
        assert!(app.ui.nora.translation.is_none());
        assert_eq!(app.session.notices.last().map(|n| (n.text.as_str(), n.error)), Some(("The message is already in English.", false)));
    }

    #[test]
    fn a_refused_translation_says_why_and_shows_nothing() {
        let (mut app, host, ctx) = hosted();
        app.act(Action::Nora(Ask::Translate("de")));
        let id = host.sent.borrow().last().map(|c| c.id).unwrap();
        let shown = json!({"message": "Nora refused to process this message"}).to_string();
        answers(&host, id, 417, json!({"_server_messages": json!([shown]).to_string()}));
        frame(&ctx, &mut app);
        assert!(app.ui.nora.translation.is_none());
        let last = app.session.notices.last().map(|n| (n.text.clone(), n.error)).unwrap();
        assert_eq!(last, ("Nora couldn't translate the message: Nora refused to process this message".to_string(), true));
    }

    #[test]
    fn text_becomes_html_without_markup() {
        assert_eq!(
            as_html("Hello <b>Ann</b> & co,\nline two\n\n\nSecond"),
            "<p>Hello &lt;b&gt;Ann&lt;/b&gt; &amp; co,<br>line two</p>\n<p>Second</p>"
        );
    }

    /// A reply opened on the print shop's message, with the person's own text typed above the signature.
    fn writing(text: &str) -> (MailApp, Host, egui::Context, String) {
        let (mut app, host, ctx) = hosted();
        app.run("mail.reply", json!({}));
        frame(&ctx, &mut app);
        let rest = app.session.compose.as_ref().map(|c| c.body.clone()).unwrap();
        let c = app.session.compose.as_mut().unwrap();
        crate::rich_edit::insert_at_start(&mut app.ui.editor, c, text);
        frame(&ctx, &mut app);
        (app, host, ctx, rest)
    }

    #[test]
    fn correct_sends_only_the_person_s_text_and_puts_the_new_one_in_its_place() {
        let (mut app, host, ctx, rest) = writing("Bonjour,\n\nc'est aprouvé, merci");
        app.act(Action::Nora(Ask::Correct));
        let call = host.sent.borrow().last().cloned().unwrap();
        assert_eq!(call.method, "nora.api.nora_webmail.proofread");
        assert_eq!(call.args["html_content"], "<p>Bonjour,</p>\n<p>c'est aprouvé, merci</p>");
        assert!(!call.args.to_string().contains("Studio Lantern") && !call.args.to_string().contains("4471"), "no signature, no quote");

        answers(
            &host,
            call.id,
            200,
            json!({"message": {"success": true, "corrected_text": "Bonjour,\n\nC'est approuvé, merci.", "has_changes": true}}),
        );
        frame(&ctx, &mut app);
        let c = app.session.compose.as_ref().unwrap();
        assert_eq!(c.body, format!("Bonjour,\n\nC'est approuvé, merci.{rest}"), "the new text, then the reply as it was");
        assert_eq!(c.styles.chars.len(), c.body.chars().count());
        assert_eq!(app.session.notices.last().map(|n| n.text.as_str()), Some("Nora's version is in the message: Undo takes it back."));
    }

    #[test]
    fn text_typed_meanwhile_wins_over_nora_s_version() {
        let (mut app, host, ctx, _rest) = writing("Bonjour, merci");
        app.act(Action::Nora(Ask::TranslateDraft("de")));
        let call = host.sent.borrow().last().cloned().unwrap();
        assert_eq!((call.method, call.args["target_language"].as_str()), ("nora.api.nora_webmail.translate", Some("de")));
        let c = app.session.compose.as_mut().unwrap();
        crate::rich_edit::insert_at_start(&mut app.ui.editor, c, "Encore un mot.");
        let before = app.session.compose.as_ref().map(|c| c.body.clone());
        answers(&host, call.id, 200, json!({"message": {"translated_text": "Guten Tag, danke", "has_changes": true}}));
        frame(&ctx, &mut app);
        assert_eq!(app.session.compose.as_ref().map(|c| c.body.clone()), before);
        assert_eq!(app.session.notices.last().map(|n| n.text.as_str()), Some("Your text changed meanwhile: Nora's version wasn't put in."));
    }

    #[test]
    fn nothing_to_rewrite_and_nothing_to_change_are_said() {
        let (mut app, host, ctx) = hosted();
        app.run("mail.reply", json!({}));
        app.act(Action::Nora(Ask::Improve));
        assert!(host.sent.borrow().is_empty(), "an empty text isn't sent");
        assert_eq!(app.session.notices.last().map(|n| n.text.as_str()), Some("Write your text first: Nora works on it."));
        let c = app.session.compose.as_mut().unwrap();
        crate::rich_edit::insert_at_start(&mut app.ui.editor, c, "Merci.");
        app.act(Action::Nora(Ask::Improve));
        let id = host.sent.borrow().last().map(|c| c.id).unwrap();
        answers(&host, id, 200, json!({"message": {"improved_text": "Merci.", "has_changes": false}}));
        frame(&ctx, &mut app);
        assert_eq!(app.session.notices.last().map(|n| n.text.as_str()), Some("Nora found nothing to change."));
    }

    #[test]
    fn a_mailbox_of_the_server_goes_with_the_call_and_the_sample_one_does_not() {
        let (mut app, host, _ctx) = hosted();
        app.act(Action::Nora(Ask::Reply { all: false }));
        assert_eq!(host.sent.borrow().last().map(|c| c.args["account_name"].clone()), Some(json!("")), "the sample mailbox");

        let store = Store::memory();
        sample::seed_if_empty(&store, NOW).unwrap();
        let host = Host { mailbox: Some("Riley@Studio.example".into()), ..Host::default() };
        let mut app = MailApp::new(Session::open(Some(store), Clock::new(NOW, 0)), Box::new(NullBackend), Box::new(host.clone()));
        let ctx = egui::Context::default();
        frame(&ctx, &mut app);
        let key = app.session.list.iter().find(|m| m.subject.starts_with("Your proof")).map(|m| m.key.clone()).unwrap();
        app.session.open_message(&key);
        frame(&ctx, &mut app);
        app.act(Action::Nora(Ask::Reply { all: false }));
        assert_eq!(host.sent.borrow().last().map(|c| c.args["account_name"].clone()), Some(json!("riley@studio.example")));
    }
}
