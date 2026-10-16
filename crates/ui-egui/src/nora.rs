// //// Neoffice — added file (no upstream equivalent): Nora, the assistant of the Neoffice server, in MailCraft.

//! Nora, the assistant of the Neoffice server: « Reply with Nora » writes the answer to the open
//! message.
//!
//! Nora lives on the server. The app reaches her only when a Neoffice page hosts it (the browser
//! build opened with `?nora`), which [`crate::Services::nora_available`] says. A call names a method
//! of the server and its arguments; the host sends it and hands the answer back a frame or more
//! later ([`crate::Services::nora_answers`]).
//!
//! The open message is untrusted: the server runs its injection guard on it before any model reads
//! it (`generate_email` and its `reply_context`). Nothing is sent: the draft goes into the reply
//! being written, above the signature and the quoted message, as one step Undo takes back. The
//! person reads it and sends it, or not.

use egui::Ui;
use mailcraft_engine::mime::Parsed;
use serde_json::{Value, json};

use crate::icons::Icon;
use crate::theme::Tokens;
use crate::{Action, MailApp, widgets};

/// The server method that writes an email body from an instruction and the email it answers.
const METHOD: &str = "nora.api.nora_webmail.generate_email";

/// What Nora is asked along with the received message. The model reads instructions in English;
/// the reply is written in the language of the message it answers.
const INSTRUCTION: &str = "Write the reply to the email being replied to, in the language that email is written in. \
Answer what it asks; when information is missing, say that it will be checked.";

/// Most characters of the received message sent along (the server keeps 4 000 of them).
const CONTEXT_MAX: usize = 6_000;

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

/// What the interface knows of Nora.
#[derive(Default)]
pub(crate) struct NoraState {
    /// A Neoffice page hosts the app: Nora can be asked.
    pub(crate) available: bool,
    last: u64,
    /// The reply Nora is writing: the call, and the message being written that waits for it.
    pending: Option<(u64, u64)>,
}

/// The reading pane's Nora button, beside Reply, Reply all and Forward. Its menu writes the reply.
pub(crate) fn reading_button(ui: &mut Ui, t: &Tokens, acts: &mut Vec<Action>) {
    let nora = widgets::pill_button(ui, Icon::Reply, "Nora", t).on_hover_text(tl!("Nora, the Neoffice assistant"));
    egui::Popup::menu(&nora).show(|ui| {
        if ui.button(tl!("Reply with Nora")).clicked() {
            acts.push(Action::NoraReply(false));
            ui.close();
        }
        if ui.button(tl!("Reply all with Nora")).clicked() {
            acts.push(Action::NoraReply(true));
            ui.close();
        }
    });
}

/// Start the reply (to everyone, with `all`) and ask Nora to write it.
pub(crate) fn reply(app: &mut MailApp, all: bool) {
    if app.ui.nora.pending.is_some() {
        app.session.notice(tl!("Nora is already writing a reply.").to_string(), false);
        return;
    }
    let Some(context) = app.session.reading.as_ref().and_then(|r| r.parsed.as_ref()).map(reply_context) else {
        return;
    };
    let before = app.session.compose.as_ref().map(|c| c.id);
    app.run(if all { "mail.replyAll" } else { "mail.reply" }, json!({}));
    // The reply didn't open (the message is still downloading, say): nothing to write into.
    let Some(compose) = app.session.compose.as_ref().map(|c| c.id).filter(|id| Some(*id) != before) else {
        return;
    };
    app.ui.nora.last += 1;
    let id = app.ui.nora.last;
    app.ui.nora.pending = Some((id, compose));
    app.services.nora_send(NoraCall { id, method: METHOD, args: json!({"prompt": INSTRUCTION, "account_name": "", "reply_context": context}) });
    app.session.notice(tl!("Nora is writing the reply…").to_string(), false);
}

/// The received message as the server reads it: who wrote it, its subject and its text.
fn reply_context(p: &Parsed) -> String {
    let name = p.from.name.trim();
    let from = if name.is_empty() { p.from.email.clone() } else { format!("{name} <{}>", p.from.email) };
    let text: String = p.text.chars().take(CONTEXT_MAX).collect();
    format!("From: {from}\nSubject: {}\n\n{}", p.subject.trim(), text.trim())
}

/// The answers that came back: the draft goes into the reply that waits for it.
pub(crate) fn pump(app: &mut MailApp) {
    for answer in app.services.nora_answers() {
        let Some((id, compose)) = app.ui.nora.pending else { continue };
        if answer.id != id {
            continue;
        }
        app.ui.nora.pending = None;
        match draft(&answer) {
            Ok(text) => put_in_reply(app, compose, &text),
            Err(why) => app.session.notice(why, true),
        }
    }
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
        app.act(Action::NoraReply(false));
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
        app.act(Action::NoraReply(false));
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
        assert!(app.ui.nora.pending.is_none());
    }

    #[test]
    fn a_refusal_leaves_the_reply_as_it_was() {
        let (mut app, host, ctx) = hosted();
        app.act(Action::NoraReply(true));
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
        app.act(Action::NoraReply(false));
        assert_eq!(host.sent.borrow().len(), 2);
    }

    #[test]
    fn a_draft_for_a_reply_that_was_closed_goes_nowhere() {
        let (mut app, host, ctx) = hosted();
        app.act(Action::NoraReply(false));
        frame(&ctx, &mut app);
        let id = host.sent.borrow().first().map(|c| c.id).unwrap();
        app.run("compose.discard", json!({}));
        answers(&host, id, 200, json!({"message": {"draft_text": "Bonjour"}}));
        frame(&ctx, &mut app);
        assert!(app.session.compose.is_none());
        assert_eq!(app.session.notices.last().map(|n| n.text.as_str()), Some("The reply was closed before Nora's draft came back."));
    }
}
