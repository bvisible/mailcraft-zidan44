// //// Neoffice — added file (no upstream equivalent): the ribbon of the modern layout, on one line and without a scroll
// bar (maintenance#1701, #1706).

//! The new Outlook's ribbon, its « simplified ribbon »: one line; when the window is narrow, the buttons' names give way
//! first (each keeps its icon, its name on hover), then what still doesn't fit goes under « ⋯ » at the end of the line,
//! in its order; never a scroll bar (Microsoft, « Use the Simplified Ribbon »). Upstream's classic layout scrolls its
//! ribbon sideways (lib.rs); the layout setting (`Prefs::classic_layout`) chooses.
//!
//! The ribbon's buttons are upstream's (widgets.rs). Drawn inside `container`, each asks `item` how it shows (whole, its
//! icon alone, not in the line, or as a row of « ⋯ ») and gives its widths. What a frame measured is the next frame's
//! plan; a plan that changes asks egui for another pass before anything is shown, so a resized window never shows a line
//! that doesn't fit. The items are drawn a second time inside « ⋯ » when it is open, as rows; an item past the cut keeps
//! its own menu (Categorize, Snooze…): chosen there, it opens under « ⋯ ».
//!
//! The Home tab is fitted this way; the other tabs, whose words are plain labels, wrap onto a second line rather than
//! scroll.

use egui::{Id, Rect, Response, Sense, Stroke, Ui, vec2};

use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};

/// How the ribbon is laid out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Upstream's: one line that scrolls sideways.
    Classic,
    /// One line that fits: names give way, then « ⋯ ».
    Fit,
    /// One line that wraps onto another.
    Wrap,
}

/// How an item of a fitted line shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Show {
    /// Its icon and its name.
    Whole,
    /// Its icon alone, its name on hover.
    Icon,
    /// Not in the line: it waits under « ⋯ ».
    Hidden,
    /// A row of the « ⋯ » menu.
    Row,
}

/// The width of a button showing its icon alone.
pub const ICON_W: f32 = 32.0;
/// The width « ⋯ » takes at the end of the line.
const MORE_W: f32 = 34.0;
/// The space between two items (lib.rs sets it for the ribbon).
const GAP: f32 = 2.0;
/// The width of the rows of « ⋯ », as a menu's (a longer name widens it).
const ROW_MIN_W: f32 = 200.0;
const ROW_MAX_W: f32 = 260.0;

/// What is kept from one frame to the next: the plan.
#[derive(Clone, Debug, Default, PartialEq)]
struct Plan {
    /// The items before this one keep their names, the others show their icon alone (`None`: every name).
    names: Option<usize>,
    /// The first item that waits under « ⋯ » (`None`: all of them are in the line).
    cut: Option<usize>,
    more: Option<Rect>,
}

/// While a line is drawn: where it stands, and the widths measured.
#[derive(Clone, Debug, Default)]
struct Fit {
    on: bool,
    wrap: bool,
    rows: bool,
    menu_open: bool,
    next: usize,
    /// Rows drawn in « ⋯ » so far (a separator never opens it).
    drawn: usize,
    plan: Plan,
    whole: Vec<f32>,
    icon: Vec<f32>,
}

fn base() -> Id {
    Id::new("neoffice-ribbon")
}

fn fit(ui: &Ui) -> Fit {
    ui.data(|d| d.get_temp::<Fit>(base().with("fit"))).unwrap_or_default()
}

fn keep(ui: &Ui, f: Fit) {
    ui.data_mut(|d| d.insert_temp(base().with("fit"), f));
}

/// Draw the ribbon's items (`items`, with the buttons of widgets.rs) as `mode` lays them out. `items` may be drawn twice
/// in a frame: in the line, then in « ⋯ » when it is open.
pub fn container(ui: &mut Ui, t: &Tokens, mode: Mode, mut items: impl FnMut(&mut Ui)) {
    match mode {
        Mode::Classic => {
            egui::ScrollArea::horizontal().id_salt("ribbon").show(ui, |ui| items(ui));
        }
        Mode::Wrap => {
            keep(ui, Fit { on: true, wrap: true, ..Fit::default() });
            items(ui);
            keep(ui, Fit::default());
        }
        Mode::Fit => line(ui, t, items),
    }
}

/// The row the items are laid in: a line, a wrapping line, or the column of « ⋯ ».
pub fn row(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    let f = fit(ui);
    if f.on && f.rows {
        ui.vertical(|ui| {
            ui.set_min_width(ROW_MIN_W);
            ui.set_max_width(ROW_MAX_W);
            add(ui);
        });
    } else if f.on && f.wrap {
        ui.horizontal_wrapped(add);
    } else {
        ui.horizontal(add);
    }
}

fn line(ui: &mut Ui, t: &Tokens, mut items: impl FnMut(&mut Ui)) {
    let ctx = ui.ctx().clone();
    let available = ui.available_width();
    let plan: Plan = ui.data(|d| d.get_temp(base().with("plan"))).unwrap_or_default();
    let more_id = base().with("more");
    // « ⋯ » open, or opening: egui opens a popup in the frame of the click, so its rows are drawn in this frame too, and
    // the line's waiting items must not take their ids (one id in two layers in a frame is a debug assertion of egui).
    let clicked_open =
        plan.more.is_some_and(|more| ui.input(|i| i.pointer.any_click() && i.pointer.interact_pos().is_some_and(|at| more.contains(at))));
    let menu_open = clicked_open || egui::Popup::is_id_open(&ctx, more_id.with("popup"));
    keep(ui, Fit { on: true, menu_open, plan: plan.clone(), ..Fit::default() });
    let mut more_at = None;
    ui.horizontal(|ui| {
        items(ui);
        if plan.cut.is_some() {
            let (rect, _) = ui.allocate_exact_size(vec2(MORE_W, 28.0), Sense::hover());
            let more = ui.interact(rect, more_id, Sense::click());
            let fill = if more.hovered() { t.hover } else { t.surface };
            ui.painter().rect(rect, 6.0, fill, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
            for dx in [-6.0, 0.0, 6.0] {
                ui.painter().circle_filled(rect.center() + vec2(dx, 0.0), 1.6, t.dim);
            }
            more_at = Some(rect);
            let more = more.on_hover_text(tl!("More actions"));
            egui::Popup::menu(&more).show(|ui| {
                let f = fit(ui);
                keep(ui, Fit { rows: true, next: 0, drawn: 0, ..f });
                items(ui);
                let f = fit(ui);
                keep(ui, Fit { rows: false, ..f });
            });
        }
    });
    let f = fit(ui);
    let (names, cut) = plan_for(&f.whole, &f.icon, available);
    let next = Plan { names, cut, more: more_at.or(plan.more) };
    let changed = (next.names, next.cut) != (plan.names, plan.cut);
    ui.data_mut(|d| d.insert_temp(base().with("plan"), next));
    keep(ui, Fit::default());
    if changed {
        ctx.request_discard("the ribbon's line changed");
    }
}

/// The plan for a line of items of these widths (whole, and icon alone) in `available` points: the names give way from
/// the end of the line, as few as needed, so the first actions keep theirs longest; when even icons alone don't fit, the
/// first item that goes under « ⋯ » (never the first one).
fn plan_for(whole: &[f32], icon: &[f32], available: f32) -> (Option<usize>, Option<usize>) {
    let total = |widths: &[f32]| widths.iter().map(|w| w + GAP).sum::<f32>();
    let n = whole.len().min(icon.len());
    if n == 0 || total(&whole[..n]) <= available {
        return (None, None);
    }
    for keep in (0..n).rev() {
        if total(&whole[..keep]) + total(&icon[keep..n]) <= available {
            return (Some(keep), None);
        }
    }
    let mut used = MORE_W + GAP;
    for (i, w) in icon[..n].iter().enumerate() {
        if used + w + GAP > available {
            return (Some(0), Some(i.max(1)));
        }
        used += w + GAP;
    }
    (Some(0), None)
}

/// How the next item shows (`whole` and `icon`: its widths), `Whole` outside a fitted line.
pub fn item(ui: &Ui, whole: f32, icon: f32) -> Show {
    let mut f = fit(ui);
    if !f.on || f.wrap {
        return Show::Whole;
    }
    let i = f.next;
    f.next += 1;
    let past = f.plan.cut.is_some_and(|c| i >= c);
    let show = if f.rows {
        if past { Show::Row } else { Show::Hidden }
    } else {
        f.whole.push(whole);
        f.icon.push(icon);
        if past {
            Show::Hidden
        } else if f.plan.names.is_some_and(|keep| i >= keep) {
            Show::Icon
        } else {
            Show::Whole
        }
    };
    keep(ui, f);
    show
}

/// The id of the item just asked about (`item`), the same in the line and in « ⋯ », so that a menu of its own opened
/// from « ⋯ » shows under it once « ⋯ » closes.
fn item_id(ui: &Ui) -> Id {
    base().with(("item", fit(ui).next.saturating_sub(1)))
}

/// What an item not in the line answers: never clicked; anchored at « ⋯ » for a menu of its own.
pub fn hidden(ui: &mut Ui) -> Response {
    let f = fit(ui);
    // In the line, anchored at « ⋯ » for a menu of its own; in « ⋯ », nowhere (an item over « ⋯ » in the menu's layer
    // would take its hover and pile up tooltips over the menu).
    let nowhere = Rect::from_min_size(ui.cursor().min, egui::Vec2::ZERO);
    let at = if f.rows { nowhere } else { f.plan.more.unwrap_or(nowhere) };
    // While « ⋯ » is open its rows carry the items' ids: these take others.
    let id = if f.menu_open || f.rows { base().with(("hidden", f.rows, f.next)) } else { item_id(ui) };
    ui.interact(at, id, Sense::hover())
}

/// A button showing its icon alone, its name on hover.
pub fn icon_button(ui: &mut Ui, icon: Icon, label: &str, t: &Tokens, enabled: bool, on: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ICON_W, 30.0), if enabled { Sense::click() } else { Sense::hover() });
    if on {
        ui.painter().rect(rect, 6.0, t.accent_soft, Stroke::new(1.0, t.accent.gamma_multiply(0.5)), egui::StrokeKind::Inside);
    } else if enabled && (resp.hovered() || resp.is_pointer_button_down_on()) {
        ui.painter().rect_filled(rect, 6.0, if resp.is_pointer_button_down_on() { t.accent_soft } else { t.hover });
    }
    icons::paint(ui.painter(), icons::centered(rect, 16.0), icon, if enabled { t.accent } else { t.faint });
    resp.on_hover_text(label)
}

/// A row of « ⋯ »: the item's icon and name (a tick when it is on), the whole width of the menu.
pub fn row_button(ui: &mut Ui, icon: Option<Icon>, label: &str, t: &Tokens, enabled: bool, on: bool) -> Response {
    let mut f = fit(ui);
    f.drawn += 1;
    keep(ui, f);
    let galley = ui.painter().layout_no_wrap(label.to_string(), theme::regular(13.0), t.text);
    let w = ui.available_width().clamp(ROW_MIN_W, ROW_MAX_W).max(galley.size().x + 56.0);
    let (rect, _) = ui.allocate_exact_size(vec2(w, 30.0), Sense::hover());
    let resp = ui.interact(rect, item_id(ui), if enabled { Sense::click() } else { Sense::hover() });
    if enabled && resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, t.hover);
    }
    let color = if enabled { t.text } else { t.faint };
    if let Some(icon) = icon {
        icons::paint(ui.painter(), Rect::from_min_size(rect.min + vec2(8.0, 7.0), vec2(16.0, 16.0)), icon, if enabled { t.accent } else { t.faint });
    }
    ui.painter().galley_with_override_text_color(rect.min + vec2(32.0, (30.0 - galley.size().y) / 2.0), galley, color);
    if on {
        let r = Rect::from_min_size(egui::pos2(rect.max.x - 24.0, rect.min.y + 8.0), vec2(14.0, 14.0));
        ui.painter().line_segment([r.left_center(), r.center_bottom()], Stroke::new(1.6, t.accent));
        ui.painter().line_segment([r.center_bottom(), r.right_top()], Stroke::new(1.6, t.accent));
    }
    resp
}

/// A separator of « ⋯ » (a group starts), never above its first row.
pub fn row_separator(ui: &mut Ui) {
    if fit(ui).drawn > 0 {
        ui.separator();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_give_way_from_the_end_then_what_does_not_fit_waits_under_more() {
        let whole = [100.0, 80.0, 80.0, 80.0];
        let icon = [100.0, ICON_W, ICON_W, ICON_W];
        assert_eq!(plan_for(&whole, &icon, 400.0), (None, None), "all of it fits");
        assert_eq!(plan_for(&whole, &icon, 320.0), (Some(3), None), "the last name gives way: 100 + 80 + 80 + 32 and the gaps");
        assert_eq!(plan_for(&whole, &icon, 260.0), (Some(2), None), "two names out: 100 + 80 + 32 + 32 and the gaps (252)");
        assert_eq!(plan_for(&whole, &icon, 205.0), (Some(1), None), "every name out but the first item's, which has none to lose");
        assert_eq!(plan_for(&whole, &icon, 180.0), (Some(0), Some(2)), "« ⋯ » after two: 34 + 100 + 32 and the gaps");
        assert_eq!(plan_for(&whole, &icon, 150.0), (Some(0), Some(1)), "« ⋯ » after the first");
        // The first item never waits under « ⋯ », however narrow the line.
        assert_eq!(plan_for(&whole, &icon, 10.0), (Some(0), Some(1)));
        assert_eq!(plan_for(&[], &[], 10.0), (None, None), "nothing to fit");
    }
}
