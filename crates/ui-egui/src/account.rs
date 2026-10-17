// //// Neoffice — added file (no upstream equivalent): the account button of the person signed in to the hosting page.

//! The person signed in to the page that hosts MailCraft (Neodesk names them in the address, as it does for the document
//! editors): a round button at the right end of the top bar, the size of the bar's controls, filled with the theme's
//! accent colour, with their initials in the accent's text colour and their name on hover. It is drawn as the document
//! editors draw theirs (WordCraft's account button), so the person finds the same button in every app; on MailCraft's
//! bar, which is the accent colour too, a ring of the initials' colour draws its edge.

use egui::{Align2, Color32, Sense, Stroke, Ui, vec2};

use crate::theme::{self, Tokens};

/// The size of the bar's controls (`widgets::bar_icon_button`).
const SIZE: f32 = 30.0;

/// Up to two initials: the first letter of each of the name's first two words.
pub fn initials(name: &str) -> String {
    name.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect::<String>().to_uppercase()
}

/// The button, where the layout places it next.
pub fn button(ui: &mut Ui, name: &str, t: &Tokens) {
    let (r, resp) = ui.allocate_exact_size(vec2(SIZE, SIZE), Sense::click());
    let radius = SIZE / 2.0 - 1.0;
    let painter = ui.painter();
    painter.circle_filled(r.center(), radius, t.accent);
    if resp.hovered() {
        painter.circle_filled(r.center(), radius, Color32::from_white_alpha(40));
    }
    painter.circle_stroke(r.center(), radius, Stroke::new(1.5, t.on_accent));
    painter.text(r.center(), Align2::CENTER_CENTER, initials(name), theme::bold(12.0), t.on_accent);
    resp.on_hover_text(name);
}

#[cfg(test)]
mod tests {
    use egui::{Event, Pos2, RawInput, Rect, Shape};
    use mailcraft_engine::model::dates::Clock;
    use mailcraft_engine::store::Store;
    use mailcraft_engine::{Session, sample};

    use super::*;
    use crate::{MailApp, NullBackend, NullServices};

    const NOW: i64 = 1_791_979_200;

    /// The text one frame draws, with where each piece starts.
    fn drawn(ctx: &egui::Context, app: &mut MailApp, time: f64, events: Vec<Event>) -> Vec<(String, Pos2)> {
        fn collect(shape: &Shape, out: &mut Vec<(String, Pos2)>) {
            match shape {
                Shape::Text(t) => out.push((t.galley.text().to_string(), t.pos)),
                Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, out)),
                _ => {}
            }
        }
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0));
        let mut frame = ctx.run_ui(RawInput { screen_rect: Some(screen), time: Some(time), events, ..Default::default() }, |ui| app.ui(ui));
        frame.textures_delta.clear();
        let mut out = Vec::new();
        frame.shapes.iter().for_each(|clipped| collect(&clipped.shape, &mut out));
        out
    }

    #[test]
    fn the_signed_in_person_shows_at_the_right_end_of_the_top_bar() {
        assert_eq!(initials("Jeanne Muster"), "JM");
        assert_eq!(initials("  jeanne   marie muster "), "JM", "the first two words");
        assert_eq!((initials("Émile"), initials("")), ("É".to_string(), String::new()));

        let store = Store::memory();
        sample::seed_if_empty(&store, NOW).unwrap();
        let mut app = MailApp::new(Session::open(Some(store), Clock::new(NOW, 0)), Box::new(NullBackend), Box::new(NullServices(NOW)));
        let ctx = egui::Context::default();
        let none = drawn(&ctx, &mut app, 0.0, vec![]);
        assert!(none.iter().all(|(text, _)| text != "JM"), "no button without a name from the page");

        app.ui.signed_in = Some("Jeanne Muster".into());
        let shown = drawn(&ctx, &mut app, 0.1, vec![]);
        let at = shown.iter().find(|(text, _)| text == "JM").map(|(_, at)| *at).expect("the initials are drawn");
        assert!(at.x > 1280.0 - 12.0 - SIZE && at.y < 46.0, "at the right end of the top bar: {at:?}");

        // On hover, the name.
        let centre = Pos2::new(at.x + 6.0, at.y + 6.0);
        let mut tip = Vec::new();
        for (n, time) in [0.2, 1.0, 2.0, 3.0].into_iter().enumerate() {
            let events = if n == 0 { vec![Event::PointerMoved(centre)] } else { vec![] };
            tip = drawn(&ctx, &mut app, time, events);
        }
        assert!(tip.iter().any(|(text, _)| text == "Jeanne Muster"), "the name on hover: {:?}", tip.iter().map(|(t, _)| t).collect::<Vec<_>>());
    }
}
