// //// Neoffice — added file (no upstream equivalent): the HTML of what is pasted in the browser, for the editor
// (maintenance#1705).

//! A page reads what is pasted only through the browser's `paste` event (`clipboardData`), and eframe passes on its
//! text alone: in the browser, text pasted from Word, Excel, a web page or a message lost its bold, its links and its
//! tables, where the desktop app keeps them (it reads the system's clipboard, `set_clipboard_html` in its main.rs).
//! This keeps the HTML of the last paste, read in the capture phase before eframe sees the event, and gives it to the
//! editor the same way: `clipboard_html` finds it while that paste is handled. Every paste replaces it (with nothing
//! when it brings no HTML, or HTML too big to read), so an older one is never pasted again.

use std::cell::RefCell;

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::Closure;

/// The most HTML read from one paste (a page copied whole can weigh megabytes of markup and pictures).
const MAX_HTML: usize = 2 << 20;

thread_local! {
    static PASTED: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Listen to the page's pastes and hand the editor their HTML. Once, at start.
pub fn install() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let listener = Closure::<dyn FnMut(web_sys::ClipboardEvent)>::new(|event: web_sys::ClipboardEvent| {
        let html =
            event.clipboard_data().and_then(|data| data.get_data("text/html").ok()).filter(|html| !html.trim().is_empty() && html.len() <= MAX_HTML);
        PASTED.with(|kept| *kept.borrow_mut() = html);
    });
    // The capture phase of the window: before eframe's own listener turns the paste into text.
    if window.add_event_listener_with_callback_and_bool("paste", listener.as_ref().unchecked_ref(), true).is_err() {
        log::warn!("pastes keep their text only: the page refused a paste listener");
        return;
    }
    // The page's lifetime: the listener stays as long as the app.
    listener.forget();
    mailcraft_ui_egui::set_clipboard_html(read);
}

/// The HTML of the paste being handled, if it brought any.
fn read() -> Option<String> {
    PASTED.with(|kept| kept.borrow().clone())
}
