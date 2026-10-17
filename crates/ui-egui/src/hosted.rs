// //// Neoffice — added file (no upstream equivalent): what the browser build keeps of its settings with a hosting page's
// mailboxes, and the account settings it offers there.

//! With the mailboxes a hosting page hands over (mailcraft-web's host.rs), the settings are kept between visits, but only
//! the interface's: never what comes of the mail (the senders whose pictures load, the conversations ignored, the
//! follow-up dates, the senders moved in or out of Focused), and what is a mailbox's (its calendars hidden, its favourite
//! folders, what is collapsed of it) by its address rather than its place, the page handing the mailboxes over in the
//! order it chooses (`a1`, `a2`… each visit). There the person's own account settings are the name shown on their mail
//! and their signature: the servers and the sign-in are the page's.

use egui::{RichText, Ui};
use serde_json::{Value, json};

use crate::theme::{self, Tokens};
use crate::{Action, widgets};

/// The settings' parts that come of the mail (`Prefs`'s, as saved).
const OF_THE_MAIL: [&str; 4] = ["trustedSenders", "ignored", "followUp", "focusOverrides"];

/// What to keep of `state` (`MailApp::saved_state`): the interface's settings, a mailbox's by its address (`accounts`:
/// each account's id and address).
pub fn saved_settings(mut state: Value, accounts: &[(String, String)]) -> Value {
    let by_id = |id: &str| accounts.iter().find(|(i, _)| i == id).map(|(_, email)| email.to_lowercase());
    if let Some(prefs) = state.get_mut("prefs").and_then(Value::as_object_mut) {
        for key in OF_THE_MAIL {
            prefs.remove(key);
        }
        rekey(prefs, &by_id);
    }
    state
}

/// `state` as kept (`saved_settings`), for the mailboxes handed over this time: a mailbox's settings by its address
/// again by its id; those of a mailbox not handed over go.
pub fn restored_settings(mut state: Value, accounts: &[(String, String)]) -> Value {
    let by_email = |email: &str| accounts.iter().find(|(_, e)| e.eq_ignore_ascii_case(email)).map(|(id, _)| id.clone());
    if let Some(prefs) = state.get_mut("prefs").and_then(Value::as_object_mut) {
        for key in OF_THE_MAIL {
            prefs.remove(key);
        }
        rekey(prefs, &by_email);
    }
    state
}

/// A mailbox's settings from one key of it (`from`, its id or its address) to the other (`to`); those `to` has none for
/// go.
fn rekey(prefs: &mut serde_json::Map<String, Value>, to: &dyn Fn(&str) -> Option<String>) {
    // `account/calendar`
    if let Some(list) = prefs.get_mut("hiddenCalendars").and_then(Value::as_array_mut) {
        let kept: Vec<Value> = list
            .iter()
            .filter_map(Value::as_str)
            .filter_map(|k| k.split_once('/'))
            .filter_map(|(a, c)| to(a).map(|a| json!(format!("{a}/{c}"))))
            .collect();
        *list = kept;
    }
    // `favorites`, `account:<account>`, `folder:<account>/<path>`
    if let Some(list) = prefs.get_mut("collapsed").and_then(Value::as_array_mut) {
        let kept: Vec<Value> = list
            .iter()
            .filter_map(Value::as_str)
            .filter_map(|k| {
                if let Some(a) = k.strip_prefix("account:") {
                    return to(a).map(|a| format!("account:{a}"));
                }
                if let Some(rest) = k.strip_prefix("folder:") {
                    let (a, path) = rest.split_once('/')?;
                    return to(a).map(|a| format!("folder:{a}/{path}"));
                }
                Some(k.to_string())
            })
            .map(Value::String)
            .collect();
        *list = kept;
    }
    // [{ account, folder }]
    if let Some(list) = prefs.get_mut("favorites").and_then(Value::as_array_mut) {
        let kept: Vec<Value> = list
            .iter()
            .filter_map(|f| {
                let a = to(f.get("account")?.as_str()?)?;
                Some(json!({"account": a, "folder": f.get("folder")?}))
            })
            .collect();
        *list = kept;
    }
}

/// The name shown on the mail an account sends, changed: the account's own setting where its servers are the page's
/// (`Dialog::AccountName`). Whether it stays open.
pub fn account_name(ui: &mut Ui, t: &Tokens, account: &str, name: &mut String, acts: &mut Vec<Action>) -> bool {
    ui.set_width(400.0);
    ui.label(RichText::new(tl!("Your name")).font(theme::bold(18.0)).color(t.text));
    ui.add_space(6.0);
    ui.add(egui::TextEdit::singleline(name).hint_text(tl!("Shown on mail you send")).desired_width(f32::INFINITY));
    ui.add_space(10.0);
    let mut open = true;
    ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), 30.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if widgets::primary_button(ui, "Save", t, true).clicked() {
            acts.push(Action::Run("account.update", json!({"account": account, "name": name.trim()})));
            open = false;
        }
        if ui.button(tl!("Cancel")).clicked() {
            open = false;
        }
    });
    open
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> Value {
        json!({
            "prefs": {
                "hiddenCalendars": ["a1/K1", "a2/K7"],
                "collapsed": ["favorites", "account:a2", "folder:a1/INBOX/Clients"],
                "favorites": [{"account": "a1", "folder": "INBOX"}, {"account": "a2", "folder": "Sent"}],
                "trustedSenders": ["news@example.com"],
                "ignored": ["<m1@example.com>"],
                "followUp": {"<m2@example.com>": 20750},
                "focusOverrides": {"news@example.com": false},
                "calendarsSideBySide": true,
                "readAfterSecs": 5
            },
            "peopleList": {"columns": ["name", "email"]},
            "dark": true
        })
    }

    #[test]
    fn the_interface_s_settings_are_kept_a_mailbox_s_by_its_address_and_nothing_of_the_mail() {
        let first = [("a1".to_string(), "Zoe@Example.org".to_string()), ("a2".to_string(), "team@example.org".to_string())];
        let kept = saved_settings(state(), &first);
        let prefs = &kept["prefs"];
        for gone in OF_THE_MAIL {
            assert!(prefs.get(gone).is_none(), "{gone}");
        }
        assert_eq!(prefs["hiddenCalendars"], json!(["zoe@example.org/K1", "team@example.org/K7"]));
        assert_eq!(prefs["collapsed"], json!(["favorites", "account:team@example.org", "folder:zoe@example.org/INBOX/Clients"]));
        assert_eq!(prefs["favorites"][1], json!({"account": "team@example.org", "folder": "Sent"}));
        assert_eq!((&prefs["calendarsSideBySide"], &prefs["readAfterSecs"], &kept["dark"]), (&json!(true), &json!(5), &json!(true)));
        assert_eq!(kept["peopleList"], state()["peopleList"]);
        // The next visit hands the mailboxes over the other way round, and one of them only: each its own settings.
        let next = [("a1".to_string(), "team@example.org".to_string())];
        let back = restored_settings(kept, &next);
        assert_eq!(back["prefs"]["hiddenCalendars"], json!(["a1/K7"]));
        assert_eq!(back["prefs"]["collapsed"], json!(["favorites", "account:a1"]));
        assert_eq!(back["prefs"]["favorites"], json!([{"account": "a1", "folder": "Sent"}]));
    }
}
