// //// Neoffice — added file (no upstream equivalent): what the People pane and the calendar don't offer, and why, and
// the question asked before one of Neoffice's contacts is deleted.

//! What can't be done to a person, or added at all, and the reason the greyed control gives on hover (a card in a
//! read-only address book is upstream's: `Session::person_read_only`, and People says so in place of Edit and Delete; the
//! ribbon's greyed Edit and Delete say why on hover):
//! - a card of a book whose server doesn't let the person delete its cards (a JMAP book's rights) isn't deleted;
//! - « New contact », « Add sender to contacts » and « New event » need somewhere to go: a writable book or calendar, and
//!   the one on this computer only where it is kept (not in the browser build with a hosting page's mailboxes, which
//!   keeps nothing between visits: `Session::keeps_local_pim`). The native app always has its own.
//! - in the browser build with a hosting page's mailboxes, Neoffice's cards keep no notes and its events no categories,
//!   the calendar on this computer, empty and where nothing can be put, isn't listed, and deleting one of Neoffice's
//!   contacts (the whole team's) asks first.

use egui::{RichText, Sense, Ui, vec2};
use mailcraft_engine::model::pim::LOCAL;
use mailcraft_engine::{Person, Session};
use serde_json::json;

use crate::theme::{self, Tokens};
use crate::{Action, Dialog};

/// Why this person's card can't be deleted, if so: its server doesn't let the person delete its book's cards
/// (`Pim::no_delete_books`; one in a read-only book has neither Edit nor Delete).
pub fn no_delete(s: &Session, p: &Person) -> Option<&'static str> {
    s.pim.get(&p.account).is_some_and(|pim| pim.no_delete_books.contains(&p.book)).then(|| tl!("Your rights do not let you delete this contact"))
}

/// Why the selected person (`uid`) can't be changed, if so: they are in a read-only address book
/// (`Session::person_read_only`, where upstream's People says so in place of Edit and Delete): the ribbon's greyed Edit
/// and Delete say it on hover.
pub fn no_change(s: &Session, uid: Option<&str>) -> Option<&'static str> {
    uid.filter(|u| s.person_read_only(u)).map(|_| tl!("Your rights do not let you change this contact"))
}

/// What Delete does to this person: one question first. In the browser build with a hosting page's mailboxes, a card of
/// the server's book is the company's (Neoffice's contacts, shared by the whole team, which an administrator can
/// restore), so Neoffice's own question says so; elsewhere upstream's (`Dialog::ConfirmDeleteContact`), never both.
pub fn delete(s: &Session, p: &Person) -> Action {
    if !s.keeps_local_pim && p.book != LOCAL {
        return Action::Dialog(Dialog::DeleteContact { uid: p.card.uid.clone(), name: p.card.display() });
    }
    Action::Dialog(Dialog::ConfirmDeleteContact { uid: p.card.uid.clone(), name: p.card.display() })
}

/// The question `delete` asks: who, that the card goes from Neoffice's contacts for the whole team, and that an
/// administrator can restore it; Cancel, or Delete (the danger). Escape is Cancel, as for every dialog. Whether it stays open.
pub fn confirm_delete(ui: &mut Ui, t: &Tokens, uid: &str, name: &str, acts: &mut Vec<Action>) -> bool {
    ui.set_width(400.0);
    ui.label(RichText::new(tl!("Delete this contact?")).font(theme::bold(18.0)).color(t.text));
    ui.add_space(6.0);
    let what = tf!("{name}'s card will be removed from Neoffice's contacts for the whole team. An administrator can restore it.", name = name);
    ui.label(RichText::new(what).color(t.dim));
    ui.add_space(10.0);
    let mut open = true;
    ui.allocate_ui_with_layout(vec2(ui.available_width(), 30.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if danger_button(ui, tl!("Delete"), t).clicked() {
            acts.push(Action::Run("contact.delete", json!({"uid": uid})));
            open = false;
        }
        if ui.button(tl!("Cancel")).clicked() {
            open = false;
        }
    });
    open
}

/// A dialog's button for what can't be taken back here: filled in the danger colour.
fn danger_button(ui: &mut Ui, label: &str, t: &Tokens) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(label.to_string(), theme::bold(13.0), egui::Color32::WHITE);
    let (rect, resp) = ui.allocate_exact_size(vec2((galley.size().x + 28.0).max(76.0), 30.0), Sense::click());
    let fill = if resp.hovered() { t.danger.gamma_multiply(0.88) } else { t.danger };
    ui.painter().rect_filled(rect, 6.0, fill);
    ui.painter().galley_with_override_text_color(rect.center() - galley.size() / 2.0, galley, egui::Color32::WHITE);
    resp
}

/// Whether deleting an event of `calendar` asks first: in the browser build with a hosting page's mailboxes, an event of
/// a Neoffice calendar, which the desk never deletes without asking (a meeting with guests sends them cancellations).
pub fn asks_before_delete(s: &Session, calendar: &str) -> bool {
    !s.keeps_local_pim && calendar != LOCAL
}

/// What is deleted: the event, one occurrence of its series, or the whole series.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Deleting {
    Event,
    Occurrence,
    Series,
}

/// An event's deletion waiting for the person's answer: what `event.delete` is given, what is deleted, the event's title,
/// whether guests are told (a meeting the person organizes), and the event's form to go back to.
#[derive(Clone, Debug)]
pub struct EventDeletion {
    pub params: serde_json::Value,
    pub what: Deleting,
    pub title: String,
    pub guests: bool,
    pub form: Box<crate::pim_dialogs::EventForm>,
    pub error: Option<String>,
}

/// What a Delete of the event form does: asks first where `f` says so (`EventForm::asks_before_delete`), else deletes at
/// once, as upstream does.
pub fn delete_event(f: &crate::pim_dialogs::EventForm, what: Deleting, params: serde_json::Value) -> Action {
    if !f.asks_before_delete {
        return Action::Submit("event.delete", params);
    }
    let guests = f.can_invite && !f.attendees.is_empty();
    let title = f.summary.trim().to_string();
    Action::Dialog(Dialog::DeleteEvent(Box::new(EventDeletion { params, what, title, guests, form: Box::new(f.clone()), error: None })))
}

/// The question `delete_event` asks: what is removed from the person's Neoffice calendar, and that the guests are told
/// when there are; Cancel (back to the event), or Delete (the danger). Escape is Cancel. Whether it stays open.
pub fn confirm_delete_event(ui: &mut Ui, t: &Tokens, d: &mut EventDeletion, acts: &mut Vec<Action>) -> bool {
    ui.set_width(420.0);
    let (question, what) = match d.what {
        Deleting::Event => (tl!("Delete this event?"), tf!("The event {title} will be removed from your Neoffice calendar.", title = d.title)),
        Deleting::Occurrence => {
            (tl!("Delete this occurrence?"), tf!("This occurrence of {title} will be removed from your Neoffice calendar.", title = d.title))
        }
        Deleting::Series => {
            (tl!("Delete the whole series?"), tf!("Every occurrence of {title} will be removed from your Neoffice calendar.", title = d.title))
        }
    };
    ui.label(RichText::new(question).font(theme::bold(18.0)).color(t.text));
    ui.add_space(6.0);
    ui.label(RichText::new(what).color(t.dim));
    if d.guests {
        ui.label(RichText::new(tl!("Its guests will be told.")).color(t.dim));
    }
    if let Some(e) = &d.error {
        ui.label(RichText::new(crate::messages::translate(e)).color(t.danger).small());
    }
    ui.add_space(10.0);
    let mut open = true;
    ui.allocate_ui_with_layout(vec2(ui.available_width(), 30.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if danger_button(ui, tl!("Delete"), t).clicked() {
            // Closed once done (as a form is), or showing why not.
            acts.push(Action::Submit("event.delete", d.params.clone()));
        }
        if ui.button(tl!("Cancel")).clicked() {
            acts.push(Action::Dialog(Dialog::Event(d.form.clone())));
            open = false;
        }
    });
    open
}

/// Why an event's categories can't be picked, if so: in the browser build with a hosting page's mailboxes, the event is
/// in a Neoffice calendar (`calendar`, or any for a new event: the calendar here isn't offered there), and Neoffice keeps
/// no categories on its events (the server refuses a change that has them).
pub fn no_categories(s: &Session, calendar: Option<&str>) -> Option<&'static str> {
    (!s.keeps_local_pim && calendar.is_none_or(|c| c != LOCAL)).then(|| tl!("Neoffice keeps no categories on events"))
}

/// Why the contact form's notes can't be written, if so: in the browser build with a hosting page's mailboxes, the card
/// (`uid`, or a new one) is in a Neoffice book, and Neoffice keeps no notes on contacts.
pub fn no_notes(s: &Session, uid: Option<&str>) -> Option<&'static str> {
    let book = match uid {
        Some(uid) => s.find_person(uid).map(|p| p.book),
        None => s.new_card_target(None).map(|(_, book)| book),
    };
    (!s.keeps_local_pim && book.is_some_and(|b| b != LOCAL)).then(|| tl!("Neoffice keeps no notes on contacts"))
}

/// Why no contact can be added, if so (with no account at all, the controls are greyed already).
pub fn no_new_cards(s: &Session) -> Option<&'static str> {
    (!s.accounts.is_empty() && !s.can_add_cards()).then(|| tl!("Your rights do not let you add contacts"))
}

/// Why no event can be added, if so: the server's calendars can't be added to (the person's rights), or there is no
/// calendar to add to at all.
pub fn no_new_events(s: &Session) -> Option<&'static str> {
    if s.accounts.is_empty() || s.can_add_events() {
        return None;
    }
    let servers = s.pim.values().any(|p| p.calendars.iter().any(|c| c.id != LOCAL));
    Some(if servers { tl!("Your rights do not let you add events") } else { tl!("MailCraft cannot add events yet") })
}

/// `r` with the reason it is greyed, if there is one, on hover (a greyed control of an enabled `Ui`).
pub fn why(r: egui::Response, reason: Option<&'static str>) -> egui::Response {
    match reason {
        Some(text) => r.on_hover_text(text),
        None => r,
    }
}

/// The same for a control drawn in a disabled `Ui` (`add_enabled_ui`), which egui shows only this way.
pub fn why_disabled(r: egui::Response, reason: Option<&'static str>) -> egui::Response {
    match reason {
        Some(text) => r.on_disabled_hover_text(text),
        None => r,
    }
}

#[cfg(test)]
mod tests {
    use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect, Shape, vec2};
    use mailcraft_engine::model::dates::Clock;
    use mailcraft_engine::model::pim::{BookInfo, LOCAL, StoredCard};
    use mailcraft_engine::model::vcard::Card;
    use mailcraft_engine::store::Store;
    use mailcraft_engine::{Session, sample};
    use serde_json::json;

    use crate::{Dialog, MailApp, NullBackend, NullServices};

    const NOW: i64 = 1_791_979_200;

    /// Frames of one app, the clock moving on half a second each, on a screen `height` high.
    struct Frames {
        ctx: egui::Context,
        time: f64,
        height: f32,
    }

    impl Frames {
        /// One frame with these events: the text drawn, and where.
        fn run(&mut self, app: &mut MailApp, events: Vec<Event>) -> Vec<(String, Rect)> {
            fn collect(shape: &Shape, out: &mut Vec<(String, Rect)>) {
                match shape {
                    Shape::Text(t) => out.push((t.galley.text().to_string(), Rect::from_min_size(t.pos, t.galley.size()))),
                    Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, out)),
                    _ => {}
                }
            }
            self.time += 0.5;
            let screen = Rect::from_min_size(Pos2::ZERO, vec2(1280.0, self.height));
            let input = RawInput { screen_rect: Some(screen), time: Some(self.time), events, ..Default::default() };
            let mut out = self.ctx.run_ui(input, |ui| app.ui(ui));
            out.textures_delta.clear();
            let mut texts = Vec::new();
            out.shapes.iter().for_each(|clipped| collect(&clipped.shape, &mut texts));
            texts
        }

        /// A click at `at`, then a frame for what it asked.
        fn click(&mut self, app: &mut MailApp, at: Pos2) {
            let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
            for events in [vec![Event::PointerMoved(at)], vec![button(true)], vec![button(false)], vec![]] {
                self.run(app, events);
            }
        }

        /// One frame: the text drawn, and its colour.
        fn colors(&mut self, app: &mut MailApp) -> Vec<(String, egui::Color32)> {
            fn collect(shape: &Shape, out: &mut Vec<(String, egui::Color32)>) {
                match shape {
                    Shape::Text(t) => {
                        out.push((t.galley.text().to_string(), t.galley.job.sections.first().map_or(t.fallback_color, |s| s.format.color)))
                    }
                    Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, out)),
                    _ => {}
                }
            }
            self.time += 0.5;
            let screen = Rect::from_min_size(Pos2::ZERO, vec2(1280.0, self.height));
            let input = RawInput { screen_rect: Some(screen), time: Some(self.time), ..Default::default() };
            let mut out = self.ctx.run_ui(input, |ui| app.ui(ui));
            out.textures_delta.clear();
            let mut texts = Vec::new();
            out.shapes.iter().for_each(|clipped| collect(&clipped.shape, &mut texts));
            texts
        }

        /// The text drawn once the pointer has rested on `at` (tooltips included).
        fn hovered(&mut self, app: &mut MailApp, at: Pos2) -> Vec<String> {
            self.run(app, vec![Event::PointerMoved(at)]);
            for _ in 0..3 {
                self.run(app, vec![]);
            }
            self.run(app, vec![]).into_iter().map(|(text, _)| text).collect()
        }
    }

    fn sample_app() -> (MailApp, Frames) {
        let store = Store::memory();
        sample::seed_if_empty(&store, NOW).unwrap();
        let mut app = MailApp::new(Session::open(Some(store), Clock::new(NOW, 0)), Box::new(NullBackend), Box::new(NullServices(NOW)));
        let mut frames = Frames { ctx: egui::Context::default(), time: 0.0, height: 800.0 };
        // The first frames set the fonts up.
        for _ in 0..2 {
            frames.run(&mut app, vec![]);
        }
        (app, frames)
    }

    /// Where `text` is drawn, top to bottom.
    fn at(texts: &[(String, Rect)], text: &str) -> Vec<Pos2> {
        let mut all: Vec<Pos2> = texts.iter().filter(|(t, _)| t == text).map(|(_, r)| r.center()).collect();
        all.sort_by(|a, b| a.y.total_cmp(&b.y));
        all
    }

    #[test]
    fn a_card_of_a_read_only_book_is_neither_edited_nor_deleted() {
        let (mut app, mut f) = sample_app();
        let p = app.session.pim_mut("sample");
        p.books.push(BookInfo { id: "B1".into(), name: "Neoffice".into(), read_only: true, ctag: String::new() });
        let card = Card { uid: "c1".into(), name: "Jeanne Muster".into(), ..Default::default() };
        p.cards.push(StoredCard { book: "B1".into(), href: "jmap:C1".into(), card, ..Default::default() });
        app.run("view.module", json!({"name": "people"}));
        app.ui.person = Some("c1".into());
        let shown = f.run(&mut app, vec![]);
        // As upstream shows a card of a shared address book: said, with neither Edit nor Delete on it.
        assert!(shown.iter().any(|(t, _)| t == "In a shared address book: read-only"), "the card says so");
        let (edits, deletes) = (at(&shown, "Edit"), at(&shown, "Delete"));
        assert_eq!((edits.len(), deletes.len()), (1, 1), "the ribbon's only");
        for spot in edits.iter().chain(&deletes) {
            f.click(&mut app, *spot);
            assert!(app.ui.dialog.is_none(), "no form for a read-only card");
            // The ribbon's, greyed, say why on hover.
            assert!(f.hovered(&mut app, *spot).iter().any(|t| t == "Your rights do not let you change this contact"), "why, on hover");
        }
        assert!(app.session.find_person("c1").is_some(), "not deleted");

        // A card of the book here is edited as before.
        app.ui.person = app.session.people("").into_iter().find(|p| p.book == LOCAL).map(|p| p.card.uid);
        let shown = f.run(&mut app, vec![]);
        assert!(!shown.iter().any(|(t, _)| t == "In a shared address book: read-only"));
        f.click(&mut app, at(&shown, "Edit")[1]);
        assert!(matches!(app.ui.dialog, Some(Dialog::Contact(_))), "the form opens");
    }

    #[test]
    fn delete_is_greyed_where_the_book_s_rights_forbid_it_and_notes_where_none_are_kept() {
        let (mut app, mut f) = sample_app();
        let p = app.session.pim_mut("sample");
        p.books.push(BookInfo { id: "B1".into(), name: "Neoffice".into(), read_only: false, ctag: String::new() });
        p.no_delete_books = vec!["B1".into()];
        p.cards.push(StoredCard {
            book: "B1".into(),
            href: "jmap:C1".into(),
            card: Card { uid: "c1".into(), name: "Jeanne Muster".into(), ..Default::default() },
            ..Default::default()
        });
        app.run("view.module", json!({"name": "people"}));
        app.ui.person = Some("c1".into());
        let shown = f.run(&mut app, vec![]);
        let deletes = at(&shown, "Delete");
        for spot in &deletes {
            f.click(&mut app, *spot);
        }
        assert!(app.session.find_person("c1").is_some(), "not deleted");
        assert!(f.hovered(&mut app, deletes[1]).iter().any(|t| t == "Your rights do not let you delete this contact"), "why, on hover");
        // Edit stays: the person may change it.
        f.click(&mut app, at(&shown, "Edit")[1]);
        assert!(matches!(app.ui.dialog, Some(Dialog::Contact(_))), "the form opens");
        // Where the server keeps no notes on its contacts (the browser build with a hosting page's mailboxes), the form says so;
        // nor a postal address or a birthday, which it doesn't offer there.
        let says = |shown: &[(String, Rect)], text: &str| shown.iter().any(|(t, _)| t == text);
        app.session.keeps_local_pim = false;
        let shown = f.run(&mut app, vec![]);
        assert!(says(&shown, "Neoffice keeps no notes on contacts"), "the notes field says why it is greyed");
        assert!(!says(&shown, "Postal code") && !says(&shown, "Birthday"), "no address nor birthday offered");
        assert!(says(&shown, "Mobile phone"), "the numbers by kind still are");
        app.session.keeps_local_pim = true;
        let shown = f.run(&mut app, vec![]);
        assert!(!says(&shown, "Neoffice keeps no notes on contacts"));
        assert!(says(&shown, "Postal code") && says(&shown, "Birthday"), "upstream's form elsewhere");
    }

    /// What a dialog just opened shows, once it has faded in.
    fn shows(f: &mut Frames, app: &mut MailApp) -> Vec<(String, Rect)> {
        for _ in 0..3 {
            f.run(app, vec![]);
        }
        f.run(app, vec![])
    }

    /// Where `text` was drawn last (a dialog draws over the window).
    fn last(texts: &[(String, Rect)], text: &str) -> Option<Pos2> {
        texts.iter().rev().find(|(t, _)| t == text).map(|(_, r)| r.center())
    }

    #[test]
    fn in_a_hosting_page_deleting_a_contact_asks_first() {
        let (mut app, mut f) = sample_app();
        // A card of the server's book (Neoffice's contacts, shared by the whole team), which the person may delete.
        let p = app.session.pim_mut("sample");
        p.books.push(BookInfo { id: "B1".into(), name: "Neoffice".into(), read_only: false, ctag: String::new() });
        let card = Card { uid: "c1".into(), name: "Jeanne Muster".into(), ..Default::default() };
        p.cards.push(StoredCard { book: "B1".into(), href: "jmap:C1".into(), card, ..Default::default() });
        app.session.keeps_local_pim = false;
        app.run("view.module", json!({"name": "people"}));
        app.ui.person = Some("c1".into());
        let shown = f.run(&mut app, vec![]);
        let deletes = at(&shown, "Delete");
        assert_eq!(deletes.len(), 2, "the ribbon's and the card's");
        let escape = Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
        for spot in &deletes {
            f.click(&mut app, *spot);
            assert!(matches!(&app.ui.dialog, Some(Dialog::DeleteContact { uid, .. }) if uid == "c1"), "asked first");
            let shown = f.run(&mut app, vec![]);
            let says = |part: &str| shown.iter().any(|(t, _)| t.contains(part));
            assert!(says("Jeanne Muster") && says("for the whole team") && says("An administrator can restore it"), "who, and what happens");
            // Escape is Cancel.
            f.run(&mut app, vec![escape.clone()]);
            f.run(&mut app, vec![]);
            assert!(app.ui.dialog.is_none() && app.session.find_person("c1").is_some(), "kept");
        }
        // Cancel keeps it.
        f.click(&mut app, deletes[1]);
        let shown = f.run(&mut app, vec![]);
        f.click(&mut app, last(&shown, "Cancel").unwrap());
        assert!(app.ui.dialog.is_none() && app.session.find_person("c1").is_some(), "Cancel: kept");
        // Delete deletes it, to be deleted on the server.
        f.click(&mut app, deletes[1]);
        let shown = f.run(&mut app, vec![]);
        f.click(&mut app, last(&shown, "Delete").unwrap());
        assert!(app.ui.dialog.is_none() && app.session.find_person("c1").is_none(), "deleted");
        assert!(app.session.pim["sample"].deleted.iter().any(|t| t.href == "jmap:C1"));

        // Elsewhere (a card of the book here, or the desktop app): upstream's question, and only it.
        app.session.keeps_local_pim = true;
        app.ui.person = app.session.people("").into_iter().find(|p| p.book == LOCAL).map(|p| p.card.uid);
        let gone = app.ui.person.clone().unwrap();
        let shown = f.run(&mut app, vec![]);
        f.click(&mut app, at(&shown, "Delete")[1]);
        assert!(matches!(&app.ui.dialog, Some(Dialog::ConfirmDeleteContact { uid, .. }) if *uid == gone), "upstream's question");
        let shown = f.run(&mut app, vec![]);
        f.click(&mut app, last(&shown, "Delete").unwrap());
        assert!(app.ui.dialog.is_none() && app.session.find_person(&gone).is_none(), "deleted after that one question");
    }

    #[test]
    fn a_deletion_the_server_refuses_says_why_however_many_notices_came_before() {
        let (mut app, mut f) = sample_app();
        // A long session: more notices than the engine keeps (it drops the oldest as each new one comes).
        for n in 0..60 {
            app.session.notice(format!("notice {n}"), false);
            f.run(&mut app, vec![]);
        }
        // The server refused to delete a card (linked to documents): its reason, in a toast.
        let errors = vec!["a contact was not deleted: This contact is linked to documents".to_string()];
        let sync = mailcraft_engine::model::pim::DavSync { errors, ..Default::default() };
        app.session.apply(mailcraft_engine::model::Event::DavSynced { account: "sample".into(), sync: Box::new(sync) });
        let shown = f.run(&mut app, vec![]);
        assert!(shown.iter().any(|(t, _)| t.ends_with("a contact was not deleted: This contact is linked to documents")), "said, with why");
    }

    /// The sample's account with a server calendar (K1, writable) holding three occurrences of the series B1 it expanded,
    /// a week apart from `start`: what NeoMail reads from a JMAP server.
    fn with_expanded_series(app: &mut MailApp, start: i64) {
        use mailcraft_engine::model::ical::CalEvent;
        use mailcraft_engine::model::pim::{CalendarInfo, SERIES_LINE, SERIES_RULE_LINE, StoredEvent};
        let p = app.session.pim_mut("sample");
        p.calendars.push(CalendarInfo { id: "K1".into(), name: "Agenda".into(), read_only: false, ..Default::default() });
        for n in 0..3i64 {
            let at = start + n * 7 * 86_400;
            let event = CalEvent {
                uid: "w1".into(),
                summary: "Weekly".into(),
                start: at,
                end: at + 3600,
                recurrence_id: Some(at),
                extra: vec![format!("{SERIES_LINE}jmap:B1"), format!("{SERIES_RULE_LINE}FREQ=WEEKLY")],
                ..Default::default()
            };
            p.events.push(StoredEvent { calendar: "K1".into(), href: format!("jmap:E{n}"), etag: String::new(), event, dirty: false });
        }
    }

    #[test]
    fn an_occurrence_a_server_expanded_is_saved_alone_or_for_the_whole_series() {
        use mailcraft_engine::model::pim::{SERIES_CHANGE, SERIES_CHANGE_LINE, series_line};
        let (mut app, mut f) = sample_app();
        let start = NOW + 2 * 86_400;
        with_expanded_series(&mut app, start);
        let at = start + 7 * 86_400;
        // It opens, at that occurrence, with the series' repeat.
        let Some(Dialog::Event(form)) = Dialog::open_event(&app.session, "w1", at) else { panic!("it opens") };
        assert_eq!((form.occurrence, form.recurring, form.repeat.as_str()), (Some(at), true, "weekly"));
        app.ui.dialog = Some(Dialog::Event(form));
        f.height = 1400.0;
        let shown = shows(&mut f, &mut app);
        for choice in ["This occurrence", "The whole series", "Delete this occurrence", "Delete the series"] {
            assert!(shown.iter().any(|(t, _)| t == choice), "{choice}");
        }
        // Saved for the whole series: the occurrence carries it there, the others show it at once.
        if let Some(Dialog::Event(form)) = app.ui.dialog.as_mut() {
            form.summary = "Weekly, renamed".into();
        }
        f.click(&mut app, last(&shown, "The whole series").unwrap());
        let shown = shows(&mut f, &mut app);
        f.click(&mut app, last(&shown, "Save").unwrap());
        assert!(app.ui.dialog.is_none(), "saved");
        let events = &app.session.pim["sample"].events;
        let saved = events.iter().find(|e| e.event.recurrence_id == Some(at)).unwrap();
        assert_eq!((saved.dirty, series_line(&saved.event, SERIES_CHANGE_LINE)), (true, Some(SERIES_CHANGE)));
        assert_eq!(events.iter().filter(|e| e.event.uid == "w1" && e.event.summary == "Weekly, renamed").count(), 3);
        // Saved alone: only it.
        let Some(Dialog::Event(mut form)) = Dialog::open_event(&app.session, "w1", start) else { panic!("it opens") };
        form.summary = "This once".into();
        app.ui.dialog = Some(Dialog::Event(form));
        let shown = shows(&mut f, &mut app);
        f.click(&mut app, last(&shown, "Save").unwrap());
        let events = &app.session.pim["sample"].events;
        assert_eq!(events.iter().filter(|e| e.event.summary == "This once").count(), 1);
        let alone = events.iter().find(|e| e.event.recurrence_id == Some(start)).unwrap();
        assert_eq!(series_line(&alone.event, SERIES_CHANGE_LINE), None, "for it alone");
    }

    #[test]
    fn in_a_hosting_page_a_server_event_s_categories_are_greyed_with_why() {
        let (mut app, mut f) = sample_app();
        let start = NOW + 2 * 86_400;
        with_expanded_series(&mut app, start);
        f.height = 1400.0;
        let category = |f: &mut Frames, app: &mut MailApp| shows(f, app).into_iter().find(|(t, _)| t == "● Work").map(|(_, r)| r.center());
        // Neoffice keeps no categories on its events.
        app.session.keeps_local_pim = false;
        app.ui.dialog = Dialog::open_event(&app.session, "w1", start);
        let chip = category(&mut f, &mut app).unwrap();
        f.click(&mut app, chip);
        assert!(matches!(&app.ui.dialog, Some(Dialog::Event(form)) if form.categories.is_empty()), "greyed");
        assert!(f.hovered(&mut app, chip).iter().any(|t| t == "Neoffice keeps no categories on events"), "why, on hover");
        // Elsewhere, as upstream has it.
        app.session.keeps_local_pim = true;
        app.ui.dialog = Dialog::open_event(&app.session, "w1", start);
        let chip = category(&mut f, &mut app).unwrap();
        f.click(&mut app, chip);
        assert!(matches!(&app.ui.dialog, Some(Dialog::Event(form)) if form.categories == ["Work"]), "picked");
    }

    /// Europe/Zurich's rules, as the browser or the system gives them: CEST (+2) from the last Sunday of March, 01:00
    /// UTC, to the last Sunday of October, 01:00 UTC; CET (+1) otherwise.
    fn zurich(t: i64) -> i32 {
        use mailcraft_engine::model::dates::{civil_from_days, days_from_civil};
        let (y, _, _) = civil_from_days(t.div_euclid(86_400));
        let last_sunday = |m: u32| {
            let end = days_from_civil(y, m, 31);
            (end - (end - 3).rem_euclid(7)) * 86_400 + 3600
        };
        if t >= last_sunday(3) && t < last_sunday(10) { 7200 } else { 3600 }
    }

    // //// Neoffice — daylight saving: a meeting at 10:00 in Zurich shows at 10:00 in the week view and in its form on both
    // sides of a change, the day of the change included (25 October 2026, 29 March 2026).
    #[test]
    fn a_meeting_at_ten_shows_at_ten_on_both_sides_of_a_change_of_offset() {
        use mailcraft_engine::model::dates::{Zone, days_from_civil};
        use mailcraft_engine::model::ical::CalEvent;
        use mailcraft_engine::model::pim::{CalendarInfo, StoredEvent};
        for (days, change) in [
            ([(2026, 10, 20), (2026, 10, 24), (2026, 10, 25), (2026, 10, 27)], "October"),
            ([(2026, 3, 24), (2026, 3, 28), (2026, 3, 29), (2026, 3, 31)], "March"),
        ] {
            let (mut app, mut f) = sample_app();
            f.height = 1400.0;
            let zone = Zone::from_offsets(NOW - 3 * 366 * 86_400, NOW + 5 * 366 * 86_400, zurich);
            app.session.set_zone(zone);
            // 10:00 in Zurich on each day, as the server gives it (UTC).
            let p = app.session.pim_mut("sample");
            p.calendars.push(CalendarInfo { id: "K1".into(), name: "Agenda".into(), read_only: false, ..Default::default() });
            let mut starts = Vec::new();
            for (n, (y, m, d)) in days.iter().enumerate() {
                let local = days_from_civil(*y, *m, *d) * 86_400 + 10 * 3600;
                let start = local - i64::from(zurich(local - 7200));
                starts.push(start);
                let event = CalEvent { uid: format!("m{n}"), summary: format!("Meeting {d}"), start, end: start + 3600, ..Default::default() };
                p.events.push(StoredEvent { calendar: "K1".into(), href: format!("jmap:M{n}"), etag: String::new(), event, dirty: false });
            }
            app.run("view.module", json!({"name": "calendar"}));
            let mut tops = Vec::new();
            for (n, (y, m, d)) in days.iter().enumerate() {
                app.run("calendar.goto", json!({"date": format!("{y}-{m:02}-{d:02}")}));
                let shown = shows(&mut f, &mut app);
                let chip = shown.iter().find(|(t, _)| t.starts_with(&format!("Meeting {d}"))).unwrap_or_else(|| panic!("{change} {d} shown"));
                assert!(chip.0.contains("10:00 – 11:00"), "{change} {d}: {}", chip.0);
                tops.push(chip.1.min.y);
                // Its form says 10:00 too.
                let Some(Dialog::Event(form)) = Dialog::open_event(&app.session, &format!("m{n}"), starts[n]) else { panic!("it opens") };
                assert_eq!(form.start, "10:00", "{change} {d}: the form");
            }
            // Drawn at the same height on every day, the day of the change included.
            assert!(tops.iter().all(|y| (y - tops[0]).abs() < 1.0), "{change}: {tops:?}");
            // A day-long event after the change opens on its own day.
            let (y, m, d) = days[3];
            let day = days_from_civil(y, m, d) * 86_400;
            let event = CalEvent { uid: "all".into(), summary: "Closed".into(), start: day, end: day + 86_400, all_day: true, ..Default::default() };
            app.session.pim_mut("sample").events.push(StoredEvent {
                calendar: "K1".into(),
                href: "jmap:A".into(),
                etag: String::new(),
                event,
                dirty: false,
            });
            // With today's offset as the app reports it (this test's services report none).
            app.session.set_clock(NOW, zurich(NOW));
            let Some(Dialog::Event(form)) = Dialog::open_event(&app.session, "all", day) else { panic!("it opens") };
            assert_eq!(
                (form.date.as_str(), form.end_date.as_str()),
                (format!("{y}-{m:02}-{d:02}").as_str(), format!("{y}-{m:02}-{d:02}").as_str()),
                "{change}"
            );
        }
    }

    // //// Neoffice — deleting an event of a Neoffice calendar in a hosting page asks first, as the desk does.
    #[test]
    fn in_a_hosting_page_deleting_an_event_asks_first() {
        use mailcraft_engine::model::ical::{Attendee, CalEvent};
        use mailcraft_engine::model::pim::StoredEvent;
        let (mut app, mut f) = sample_app();
        f.height = 1400.0;
        let start = NOW + 2 * 86_400;
        with_expanded_series(&mut app, start);
        // A meeting the person organizes, with a guest.
        let event = CalEvent {
            uid: "meet".into(),
            summary: "Budget review".into(),
            start,
            end: start + 3600,
            organizer: Some(Attendee { email: app.session.accounts[0].config.email.clone(), ..Default::default() }),
            attendees: vec![Attendee { email: "ana@example.org".into(), partstat: "NEEDS-ACTION".into(), ..Default::default() }],
            ..Default::default()
        };
        app.session.pim_mut("sample").events.push(StoredEvent {
            calendar: "K1".into(),
            href: "jmap:M1".into(),
            etag: String::new(),
            event,
            dirty: false,
        });
        app.session.keeps_local_pim = false;
        let open = |app: &mut MailApp, f: &mut Frames, uid: &str, at: i64| {
            app.ui.dialog = Dialog::open_event(&app.session, uid, at);
            shows(f, app)
        };
        // The meeting: asked first, saying the guests are told; Cancel goes back to it, nothing deleted.
        let shown = open(&mut app, &mut f, "meet", start);
        f.click(&mut app, last(&shown, "Delete").unwrap());
        let shown = shows(&mut f, &mut app);
        let says = |shown: &[(String, Rect)], part: &str| shown.iter().any(|(t, _)| t.contains(part));
        assert!(matches!(app.ui.dialog, Some(Dialog::DeleteEvent { .. })), "asked first");
        assert!(says(&shown, "Delete this event?") && says(&shown, "Budget review") && says(&shown, "Its guests will be told."));
        f.click(&mut app, last(&shown, "Cancel").unwrap());
        assert!(matches!(app.ui.dialog, Some(Dialog::Event(_))), "back to the event");
        assert!(app.session.find_event("meet").is_some(), "not deleted");
        // Delete deletes it.
        let shown = shows(&mut f, &mut app);
        f.click(&mut app, last(&shown, "Delete").unwrap());
        let shown = shows(&mut f, &mut app);
        f.click(&mut app, last(&shown, "Delete").unwrap());
        assert!(app.session.find_event("meet").is_none() && app.ui.dialog.is_none(), "deleted");
        // An occurrence: each of the two buttons asks, saying which.
        for (button, question) in [("Delete this occurrence", "Delete this occurrence?"), ("Delete the series", "Delete the whole series?")] {
            let shown = open(&mut app, &mut f, "w1", start);
            f.click(&mut app, last(&shown, button).unwrap());
            let shown = shows(&mut f, &mut app);
            assert!(says(&shown, question) && says(&shown, "Weekly") && !says(&shown, "Its guests will be told."), "{button}");
            f.click(&mut app, last(&shown, "Cancel").unwrap());
        }
        assert_eq!(app.session.pim["sample"].events.iter().filter(|e| e.event.uid == "w1").count(), 3, "nothing deleted");
        // Upstream's behaviour elsewhere: deleted at once.
        app.session.keeps_local_pim = true;
        let shown = open(&mut app, &mut f, "w1", start);
        f.click(&mut app, last(&shown, "Delete this occurrence").unwrap());
        assert_eq!(app.session.pim["sample"].events.iter().filter(|e| e.event.uid == "w1").count(), 2, "at once");
    }

    // //// Neoffice — an event of a Neoffice calendar has its categories greyed, as they look and as they act, for a new
    // event too; and the categories made at first show in the interface language.
    #[test]
    fn categories_are_greyed_where_none_are_kept_and_shown_in_the_interface_language() {
        let (mut app, mut f) = sample_app();
        f.height = 1400.0;
        app.ui.language = "fr".into();
        let start = NOW + 2 * 86_400;
        with_expanded_series(&mut app, start);
        let faint = app.ui.tokens.faint;
        let chip = |f: &mut Frames, app: &mut MailApp| {
            shows(f, app);
            f.colors(app).into_iter().find(|(t, _)| t == "● Travail").map(|(_, c)| c)
        };
        app.session.keeps_local_pim = false;
        for form in [Dialog::open_event(&app.session, "w1", start), Some(Dialog::new_event(&app.session, Some(start)))] {
            app.ui.dialog = form;
            assert_eq!(chip(&mut f, &mut app), Some(faint), "greyed, in French");
        }
        // Where the categories are kept, as upstream draws them.
        app.session.keeps_local_pim = true;
        app.ui.dialog = Dialog::open_event(&app.session, "w1", start);
        assert!(chip(&mut f, &mut app).is_some_and(|c| c != faint), "in its colour");
    }

    // //// Neoffice — in a hosting page, the account page offers what is the person's: the name shown on their mail and
    // their signature; not another account, the servers or the sign-in (the page's), nor automatic replies the server
    // doesn't offer.
    #[test]
    fn in_a_hosting_page_the_account_page_offers_only_what_is_the_person_s() {
        use mailcraft_engine::VacationState;
        let (mut app, mut f) = sample_app();
        f.height = 1400.0;
        let id = app
            .session
            .run("account.add", &json!({"email": "zoe@example.org", "password": "pw", "jmap": "https://jmap.example.org/session"}))
            .unwrap()["account"]
            .as_str()
            .unwrap()
            .to_string();
        // The page asks the server for the automatic replies; it answers it offers none.
        let open = |app: &mut MailApp, f: &mut Frames| {
            let mut b = crate::backstage::Backstage::new(crate::backstage::Page::Info);
            b.account = Some(id.clone());
            app.ui.backstage = Some(b);
            f.run(app, vec![]);
            if let Some(a) = app.session.accounts.iter_mut().find(|a| a.config.id == id) {
                a.vacation = VacationState::Failed("the server refused: this server doesn't offer automatic replies".into());
            }
            shows(f, app)
        };
        let says = |shown: &[(String, Rect)], part: &str| shown.iter().any(|(t, _)| t.contains(part));
        // Elsewhere, as upstream has it, the server's refusal said in the interface's language.
        app.ui.language = "fr".into();
        let shown = open(&mut app, &mut f);
        assert!(!says(&shown, "doesn't offer"), "translated");
        app.ui.language = "en".into();
        let shown = open(&mut app, &mut f);
        assert!(says(&shown, "Add account…") && says(&shown, "Server: https://jmap.example.org/session (JMAP)") && says(&shown, "Automatic replies"));
        // In a hosting page.
        app.session.keeps_local_pim = false;
        let shown = open(&mut app, &mut f);
        assert!(!says(&shown, "Add account…"), "no other account");
        assert!(!says(&shown, "Server: "), "no servers");
        assert!(
            !says(&shown, "Automatic replies"),
            "no automatic replies the server doesn't offer: {:?}",
            shown.iter().filter(|(t, _)| t.contains("Automatic")).collect::<Vec<_>>()
        );
        assert!(says(&shown, "Change the name shown on mail you send, or your signature."), "what the account's settings are there");
        // The name shown on the person's mail, changed there (its menu's « Your name… »).
        app.ui.dialog = Some(Dialog::AccountName { account: id.clone(), name: "Zoé Martin".into() });
        let shown = shows(&mut f, &mut app);
        f.click(&mut app, last(&shown, "Save").unwrap());
        assert!(app.ui.dialog.is_none());
        assert_eq!(app.session.account(&id).map(|a| a.config.name.as_str()), Some("Zoé Martin"));
    }

    #[test]
    fn the_browser_build_offers_no_mail_kept_on_this_computer() {
        // The browser keeps its own window of each folder (backend.rs): the setting would do nothing there.
        let (mut app, mut f) = sample_app();
        f.height = 4000.0;
        app.ui.backstage = Some(crate::backstage::Backstage::new(crate::backstage::Page::Options));
        let offered = |f: &mut Frames, app: &mut MailApp| f.run(app, vec![]).iter().any(|(t, _)| t == "Mail kept on this computer");
        assert!(offered(&mut f, &mut app), "the desktop app offers it");
        app.session.jmap_only = true;
        assert!(!offered(&mut f, &mut app), "not the browser build");
        assert!(f.run(&mut app, vec![]).iter().any(|(t, _)| t == "Inbox and notifications"), "the rest of the options stay");
    }

    #[test]
    fn the_empty_calendar_here_is_not_listed_where_nothing_can_be_put() {
        let (mut app, mut f) = sample_app();
        app.run("view.module", json!({"name": "calendar"}));
        let listed = |f: &mut Frames, app: &mut MailApp| f.run(app, vec![]).iter().filter(|(t, _)| t == "Calendar").count();
        let with = listed(&mut f, &mut app);
        assert!(with >= 1, "the calendar here, listed");
        // Where nothing here is kept, an empty one isn't listed; one holding events from before still is.
        app.session.keeps_local_pim = false;
        assert_eq!(listed(&mut f, &mut app), with, "the sample's calendar holds events: still listed");
        app.session.pim_mut("sample").events.clear();
        assert_eq!(listed(&mut f, &mut app), with - 1, "empty: not listed");
        assert!(app.session.shown_calendars().iter().all(|(_, c, _, _)| c != LOCAL), "nor given a column of its own side by side");
    }

    #[test]
    fn with_nowhere_to_add_them_new_contacts_and_events_are_greyed_with_why() {
        // The sample's book and calendar are this computer's: where those aren't kept (the browser build in a hosting
        // page), there is nowhere to add a contact or an event.
        let (mut app, mut f) = sample_app();
        app.session.keeps_local_pim = false;
        // Without the sample's studio calendar.
        app.session.pim_mut("sample").calendars.retain(|c| c.id == LOCAL);
        app.session.pim_mut("sample").events.retain(|e| e.calendar == LOCAL);
        for (module, button, why) in
            [("people", "New contact", "Your rights do not let you add contacts"), ("calendar", "New event", "MailCraft cannot add events yet")]
        {
            app.run("view.module", json!({"name": module}));
            let shown = f.run(&mut app, vec![]);
            let spots = at(&shown, button);
            assert_eq!(spots.len(), 2, "the ribbon's and the pane's {button}");
            for spot in &spots {
                f.click(&mut app, *spot);
                assert!(app.ui.dialog.is_none(), "{button} greyed");
                assert!(f.hovered(&mut app, *spot).iter().any(|t| t == why), "{button}: why, on hover");
            }
        }
        // Where the book and calendar here are kept (the native app), as before.
        app.session.keeps_local_pim = true;
        let shown = f.run(&mut app, vec![]);
        f.click(&mut app, at(&shown, "New event")[0]);
        assert!(matches!(app.ui.dialog, Some(Dialog::Event(_))), "the form opens");
    }

    // //// Neoffice — a day-long event is its days, not times: in a zone whose clock changes at midnight (00:00 is 01:00 the
    // night summer time starts there), it stays on its own days in the calendar and in its form, whatever today's offset.
    #[test]
    fn a_day_long_event_stays_on_its_days_where_the_clock_changes_at_midnight() {
        use mailcraft_engine::model::dates::{Zone, days_from_civil};
        use mailcraft_engine::model::ical::CalEvent;
        use mailcraft_engine::model::pim::{CalendarInfo, StoredEvent};
        // -04:00, then -03:00 from Sunday 4 October 2026 at midnight (04:00 UTC): today, the 14th, is past it.
        let day = days_from_civil(2026, 10, 4);
        let change = day * 86_400 + 4 * 3600;
        let offset = move |t: i64| if t >= change { -3 * 3600 } else { -4 * 3600 };
        let (mut app, mut f) = sample_app();
        f.height = 1400.0;
        app.session.set_zone(Zone::from_offsets(NOW - 3 * 366 * 86_400, NOW + 5 * 366 * 86_400, offset));
        app.session.set_clock(NOW, offset(NOW));
        let p = app.session.pim_mut("sample");
        p.calendars.push(CalendarInfo { id: "K1".into(), name: "Agenda".into(), read_only: false, ..Default::default() });
        for (uid, first, last) in [("one", day, day), ("two", day - 1, day)] {
            let (start, end) = (first * 86_400, (last + 1) * 86_400);
            let event = CalEvent { uid: uid.into(), summary: format!("Closed {uid}"), start, end, all_day: true, ..Default::default() };
            p.events.push(StoredEvent { calendar: "K1".into(), href: format!("jmap:{uid}"), etag: String::new(), event, dirty: false });
        }
        // The calendar: on its days.
        let shown = app.session.agenda(app.session.local_midnight(day - 7), app.session.local_midnight(day + 7));
        let days: Vec<(&str, Option<(i64, i64)>)> =
            shown.iter().filter(|i| ["one", "two"].contains(&i.uid.as_str())).map(|i| (i.uid.as_str(), i.days)).collect();
        assert_eq!(days, vec![("two", Some((day - 1, day + 1))), ("one", Some((day, day + 1)))]);
        app.run("view.module", json!({"name": "calendar"}));
        app.run("calendar.goto", json!({"date": "2026-10-04"}));
        assert!(shows(&mut f, &mut app).iter().any(|(t, _)| t == "Closed one"), "drawn");
        // Its form: its days, from midnight.
        let Some(Dialog::Event(form)) = Dialog::open_event(&app.session, "one", day * 86_400) else { panic!("it opens") };
        assert_eq!(
            (form.date.as_str(), form.end_date.as_str(), form.start.as_str(), form.end.as_str()),
            ("2026-10-04", "2026-10-04", "00:00", "00:00")
        );
        let Some(Dialog::Event(form)) = Dialog::open_event(&app.session, "two", (day - 1) * 86_400) else { panic!("it opens") };
        assert_eq!((form.date.as_str(), form.end_date.as_str()), ("2026-10-03", "2026-10-04"));
    }

    // //// Neoffice — the event form sends what the person changed in it, not all it showed: a change made elsewhere and
    // read while it was open (someone moving the meeting in the desk) stays.
    #[test]
    fn the_event_form_sends_only_what_the_person_changed() {
        use mailcraft_engine::model::ical::CalEvent;
        use mailcraft_engine::model::pim::{CalendarInfo, StoredEvent};
        let (mut app, _f) = sample_app();
        let start = NOW + 2 * 86_400;
        let p = app.session.pim_mut("sample");
        p.calendars.push(CalendarInfo { id: "K1".into(), name: "Agenda".into(), read_only: false, ..Default::default() });
        let event = CalEvent { uid: "m1".into(), summary: "Meeting".into(), start, end: start + 3600, ..Default::default() };
        p.events.push(StoredEvent { calendar: "K1".into(), href: "jmap:M1".into(), etag: String::new(), event, dirty: false });
        let Some(Dialog::Event(mut form)) = Dialog::open_event(&app.session, "m1", start) else { panic!("it opens") };
        // Moved an hour later elsewhere, and read, while the form is open; the person renames it.
        for e in app.session.pim_mut("sample").events.iter_mut().filter(|e| e.event.uid == "m1") {
            (e.event.start, e.event.end) = (start + 3600, start + 7200);
        }
        form.summary = "Meeting, renamed".into();
        let params = form.params();
        for unchanged in ["start", "end", "allDay", "location", "description", "repeat", "categories", "attendees"] {
            assert!(params.get(unchanged).is_none(), "{unchanged}: {params}");
        }
        app.ui.dialog = Some(Dialog::Event(form));
        app.act(crate::Action::Submit("event.update", params));
        assert!(app.ui.dialog.is_none(), "saved");
        let e = app.session.find_event("m1").unwrap().1.event.clone();
        assert_eq!((e.summary.as_str(), e.start, e.end), ("Meeting, renamed", start + 3600, start + 7200), "the move made elsewhere stays");
        // Its time changed: its time goes whole (its start, its end, whether it is all day), as the form shows it.
        let Some(Dialog::Event(mut form)) = Dialog::open_event(&app.session, "m1", start + 3600) else { panic!("it opens") };
        form.end = "15:00".into();
        let params = form.params();
        assert!(["start", "end", "allDay"].iter().all(|k| params.get(*k).is_some()) && params.get("summary").is_none(), "{params}");
    }
}
