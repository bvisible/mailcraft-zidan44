// //// Neoffice — added file (no upstream equivalent): what the People pane and the calendar don't offer, and why.

//! What can't be done to a person, or added at all, and the reason the greyed control gives on hover (a card in a
//! read-only address book is upstream's: `Session::person_read_only`, and People says so in place of Edit and Delete):
//! - a card of a book whose server doesn't let the person delete its cards (a JMAP book's rights) isn't deleted;
//! - « New contact », « Add sender to contacts » and « New event » need somewhere to go: a writable book or calendar, and
//!   the one on this computer only where it is kept (not in the browser build with a hosting page's mailboxes, which
//!   keeps nothing between visits: `Session::keeps_local_pim`). The native app always has its own.
//! - in the browser build with a hosting page's mailboxes, Neoffice's cards keep no notes, and the calendar on this
//!   computer, empty and where nothing can be put, isn't listed.

use mailcraft_engine::model::pim::LOCAL;
use mailcraft_engine::{Person, Session};

/// Why this person's card can't be deleted, if so: its server doesn't let the person delete its book's cards
/// (`Pim::no_delete_books`; one in a read-only book has neither Edit nor Delete).
pub fn no_delete(s: &Session, p: &Person) -> Option<&'static str> {
    s.pim.get(&p.account).is_some_and(|pim| pim.no_delete_books.contains(&p.book)).then(|| tl!("Your rights do not let you delete this contact"))
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

    /// Frames of one app, the clock moving on half a second each.
    struct Frames {
        ctx: egui::Context,
        time: f64,
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
            let screen = Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0));
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
        let mut frames = Frames { ctx: egui::Context::default(), time: 0.0 };
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
        // Where the server keeps no notes on its contacts (the browser build with a hosting page's mailboxes), the form says so.
        app.session.keeps_local_pim = false;
        let shown = f.run(&mut app, vec![]);
        assert!(shown.iter().any(|(t, _)| t == "Neoffice keeps no notes on contacts"), "the notes field says why it is greyed");
        app.session.keeps_local_pim = true;
        assert!(!f.run(&mut app, vec![]).iter().any(|(t, _)| t == "Neoffice keeps no notes on contacts"));
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
}
