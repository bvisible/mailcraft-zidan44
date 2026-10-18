// //// Neoffice — added file (no upstream equivalent): what the browser build keeps between visits with a hosting page's
// mailboxes (web.rs), built on every target so that its tests run natively.

//! With the mailboxes a hosting page hands over (host.rs), nothing of the mail, the accounts or their secrets is kept
//! between visits: the page hands them over again each time. The settings are kept all the same, in the browser's
//! storage, as without a hosting page (the People list's columns and order, the calendars hidden or side by side, the
//! months of the side pane, when mail is marked read, how notes are arranged…: the interface's, mailcraft-ui-egui's
//! hosted.rs), and so are the signatures and the names shown on mail made or changed here: the page hands over each
//! mailbox's in Neoffice, which one made here replaces until NeoMail writes it to Neoffice, as long as the page still
//! gives the one it replaced.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

use std::collections::HashMap;

/// What the browser keeps between visits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Keeps {
    /// The settings (and, with a hosting page's mailboxes, the signatures made here).
    pub settings: bool,
    /// The accounts and folders (where IndexedDB doesn't keep the cache).
    pub files: bool,
}

/// What is kept with the sample mailbox (nothing), with a hosting page's mailboxes (the settings only), or else.
pub fn keeps(sample: bool, hosted: bool) -> Keeps {
    Keeps { settings: !sample, files: !sample && !hosted }
}

/// A signature (or a name shown on mail) as a mailbox has it here: its text, and its look (MailCraft's own HTML, since
/// signatures have one; empty when plain, as a name always is). The page's signature comes with the look Neoffice's
/// editor gave it, read line for line (`page_signature`, maintenance#1670).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Shown {
    pub text: String,
    pub html: String,
}

impl Shown {
    pub fn plain(text: &str) -> Shown {
        Shown { text: text.to_string(), html: String::new() }
    }
}

/// A signature (or a name shown on mail) made or changed here, and the hosting page's it replaced.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Kept {
    pub made: Shown,
    /// What the page gave when it was made (empty: none).
    pub base: String,
}

/// Whether the one made here still stands: the page still gives the one it replaced (one changed in Neoffice since is
/// Neoffice's). By its words, not its HTML: the instance may write the same signature's HTML anew.
fn stands(page: &Shown, k: &Kept) -> bool {
    k.base.trim() == page.text.trim()
}

/// The signature (or name) a mailbox starts with: the one made here while it stands, else the page's.
pub fn chosen(page: &Shown, kept: Option<&Kept>) -> Shown {
    match kept {
        Some(k) if stands(page, k) => k.made.clone(),
        _ => page.clone(),
    }
}

/// The longest look of a page's signature read, in characters, as the instance counts them (it hands at most 200 000).
const MAX_PAGE_LOOK: usize = 200_000;

/// The page's signature as a mailbox shows it: its words as the page gives them (what a change in Neoffice is told by),
/// and the look Neoffice's editor gave it, as MailCraft's own HTML (`mime::neoffice_signature`: a line per paragraph,
/// a table as lines, no picture). No look when its HTML says no words (a picture alone), has no formatting (the words
/// are then the page's), or is too long to read. The look is the one MailCraft's signature editor writes back for it,
/// read and written once more: a Save that changed nothing gives that very HTML, and `keep` finds nothing made here.
pub fn page_signature(text: &str, html: &str) -> Shown {
    use mailcraft_engine::mime::{neoffice_signature, pasted_html, rich};
    let html = html.trim();
    if html.is_empty() || html.chars().nth(MAX_PAGE_LOOK).is_some() {
        return Shown::plain(text);
    }
    let (words, styles) = neoffice_signature(html);
    if words.trim().is_empty() || styles.is_plain() {
        return Shown::plain(text);
    }
    let (words, styles) = pasted_html(&rich::to_html(&words, &styles));
    if words.trim().is_empty() || styles.is_plain() {
        return Shown::plain(text);
    }
    Shown { text: text.to_string(), html: rich::to_html(&words, &styles) }
}

/// What a tab knows of a mailbox's signature (or name), to tell its own changes (`changes`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Seen {
    /// What it started with, or last wrote.
    Was(Shown),
    /// At its start, what was kept no longer stood: its first save clears that very entry (not a newer one another tab
    /// wrote since).
    Stale(Kept),
}

/// What a tab starts from for a mailbox.
pub fn seen(page: &Shown, kept: Option<&Kept>) -> Seen {
    match kept {
        // //// Neoffice — one that no longer stands, or that is the page's after all (kept before a page's look was told
        // by itself, maintenance#1670): cleared once, by the first save.
        Some(k) if !stands(page, k) || keep(page, &k.made).is_none() => Seen::Stale(k.clone()),
        _ => Seen::Was(chosen(page, kept)),
    }
}

/// A change a tab writes for a mailbox: what to keep of it now (nothing: the page's), and what it has now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub email: String,
    pub kept: Option<Kept>,
    /// Clearing what no longer stood: only that entry, left as it is if another tab replaced it since.
    pub only_over: Option<Kept>,
    pub now: Shown,
}

/// What a tab changed since it last looked (`seen`): `now` gives each of its mailboxes (address in lower case, the
/// page's, what it has now). Nothing for one unchanged, so that a tab writes only its own changes.
pub fn changes(now: &[(String, Shown, Shown)], seen: &HashMap<String, Seen>) -> Vec<Change> {
    let mut out = Vec::new();
    for (email, page, current) in now {
        let only_over = match seen.get(email) {
            Some(Seen::Was(was)) if was == current => continue,
            Some(Seen::Stale(k)) => Some(k.clone()),
            _ => None,
        };
        out.push(Change { email: email.clone(), kept: keep(page, current), only_over, now: current.clone() });
    }
    out
}

/// The mailboxes' signatures and names in one number (each mailbox's address, signature, its look, name): a change of
/// one changes it. Only a hint that something may have changed: `changes` says what (web.rs keeps them at once).
pub fn fingerprint<'a>(mailboxes: impl Iterator<Item = [&'a str; 4]>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::hash::DefaultHasher::new();
    for mailbox in mailboxes {
        mailbox.hash(&mut h);
    }
    h.finish()
}

/// What to keep of a mailbox's signature (or name), `current`, now that the page gives `page`: nothing while it is the
/// page's. A page's signature with a look is told by its look alone, since MailCraft takes a formatted signature's words
/// from its look (they may differ from the page's text: a link's address, a list's bullets); one without, by its words.
pub fn keep(page: &Shown, current: &Shown) -> Option<Kept> {
    let pages = if page.html.trim().is_empty() {
        current.text.trim() == page.text.trim() && current.html.trim().is_empty()
    } else {
        current.html.trim() == page.html.trim()
    };
    (!pages).then(|| Kept { made: current.clone(), base: page.text.clone() })
}

/// What is kept under a key: by address (lower case), `{ text, base }`, and `html` for a look (none in what was kept
/// before signatures had one). Anything else reads as nothing kept.
pub fn read(stored: Option<&str>) -> HashMap<String, Kept> {
    let Some(map) = stored.and_then(|j| serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(j).ok()) else {
        return HashMap::new();
    };
    let text = |e: &serde_json::Value, key: &str| e.get(key).and_then(serde_json::Value::as_str).unwrap_or_default().to_string();
    map.iter()
        .map(|(email, e)| (email.to_lowercase(), Kept { made: Shown { text: text(e, "text"), html: text(e, "html") }, base: text(e, "base") }))
        .collect()
}

/// What to write under a key: what is stored, as it is for every mailbox this tab did not change (another tab's change,
/// a mailbox this visit does not have), with this tab's changes; None when nothing is left to keep.
pub fn write(stored: Option<&str>, changes: &[Change]) -> Option<String> {
    let before = read(stored);
    let mut map = stored.and_then(|j| serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(j).ok()).unwrap_or_default();
    for change in changes {
        let email = change.email.to_lowercase();
        match &change.kept {
            Some(k) => {
                let mut entry = serde_json::Map::new();
                entry.insert("text".into(), k.made.text.clone().into());
                entry.insert("base".into(), k.base.clone().into());
                if !k.made.html.is_empty() {
                    entry.insert("html".into(), k.made.html.clone().into());
                }
                map.insert(email, entry.into());
            }
            None => {
                if change.only_over.as_ref().is_none_or(|stale| before.get(&email) == Some(stale)) {
                    map.remove(&email);
                }
            }
        }
    }
    (!map.is_empty()).then(|| serde_json::Value::Object(map).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_a_hosting_page_s_mailboxes_the_settings_are_kept_and_nothing_else() {
        assert_eq!(keeps(false, true), Keeps { settings: true, files: false }, "the settings, never mail, accounts or secrets");
        assert_eq!(keeps(true, false), Keeps { settings: false, files: false }, "the sample mailbox: nothing");
        assert_eq!(keeps(false, false), Keeps { settings: true, files: true }, "without a hosting page, as before");
    }

    #[test]
    fn a_signature_made_here_stays_until_neoffice_s_changes() {
        let page = Shown::plain("Jeanne Muster\nAccounts");
        // None made here: the page's, which nothing needs to keep.
        assert_eq!(chosen(&page, None), page);
        assert_eq!(keep(&page, &page), None);
        // Made here: kept with the page's it replaced, and taken at the next visit while the page gives that one.
        let mine = Shown::plain("Jeanne\nHead of accounts");
        let kept = keep(&page, &mine).unwrap();
        assert_eq!(kept, Kept { made: mine.clone(), base: page.text.clone() });
        assert_eq!(chosen(&page, Some(&kept)), mine);
        assert_eq!(seen(&page, Some(&kept)), Seen::Was(mine));
        // Changed in Neoffice since: Neoffice's, and the one made here goes at the next save.
        let changed = Shown::plain("Jeanne Muster\nFinance");
        assert_eq!(chosen(&changed, Some(&kept)), changed);
        assert_eq!(seen(&changed, Some(&kept)), Seen::Stale(kept.clone()));
        assert_eq!(seen(&changed, None), Seen::Was(changed.clone()));
        assert_eq!(keep(&changed, &changed), None);
        // A mailbox the page gives no signature for: one made here is kept as well.
        let made = keep(&Shown::default(), &Shown::plain("J. M.")).unwrap();
        assert_eq!(chosen(&Shown::default(), Some(&made)), Shown::plain("J. M."));
    }

    #[test]
    fn a_look_given_here_is_kept_with_the_words_of_the_page() {
        let page = Shown::plain("Jeanne Muster\nAccounts");
        let mut styles = mailcraft_engine::mime::rich::Styles::plain(&page.text);
        let bold = styles.intern(mailcraft_engine::mime::rich::Style { bold: true, ..Default::default() });
        for c in styles.chars.iter_mut().take(13) {
            *c = bold;
        }
        let look = Shown { text: page.text.clone(), html: mailcraft_engine::mime::rich::to_html(&page.text, &styles) };
        // The same words, made bold here: kept, look and all, and taken at the next visit.
        let kept = keep(&page, &look).unwrap();
        assert_eq!(chosen(&page, Some(&kept)), look);
        // MailCraft reads its own look back line for line: no line comes out double-spaced.
        assert_eq!(mailcraft_engine::mime::pasted_html(&look.html).0, page.text);
        // Plain again: the page's, nothing kept.
        assert_eq!(keep(&page, &page), None);
    }

    // //// Neoffice — the page's signature with the look Neoffice's editor gave it (maintenance#1670).
    #[test]
    fn the_page_s_signature_shows_with_its_look_and_is_told_by_its_words() {
        let html = "<div class=\"ql-editor read-mode\"><p><strong>Jeanne Muster</strong></p><p>Accounts</p></div>";
        let page = page_signature("Jeanne Muster\nAccounts", html);
        assert_eq!(page.text, "Jeanne Muster\nAccounts");
        // Its look, read back line for line (not double-spaced), the name in bold.
        let (words, styles) = mailcraft_engine::mime::pasted_html(&page.html);
        assert_eq!(words, "Jeanne Muster\nAccounts");
        assert!(styles.style(0).bold && !styles.style(words.chars().count() - 1).bold);
        // The page's, look and all: nothing to keep; a look made here over the same words is kept.
        assert_eq!(keep(&page, &page), None);
        let plainer = Shown::plain("Jeanne Muster\nAccounts");
        assert_eq!(keep(&page, &plainer), Some(Kept { made: plainer.clone(), base: page.text.clone() }));
        // Told by its words: the same words with another look given in Neoffice still stand; other words do not.
        let kept = keep(&page, &plainer).unwrap();
        let rewritten = page_signature("Jeanne Muster\nAccounts", "<p><em>Jeanne Muster</em></p><p>Accounts</p>");
        assert_ne!(rewritten.html, page.html);
        assert_eq!(chosen(&rewritten, Some(&kept)), plainer);
        let changed = page_signature("Jeanne Muster\nFinance", "<p>Jeanne Muster</p><p>Finance</p>");
        assert_eq!(chosen(&changed, Some(&kept)), changed);
        // No look from no HTML, HTML without formatting, a picture alone, or too much of it.
        assert_eq!(page_signature("J. M.", ""), Shown::plain("J. M."));
        assert_eq!(page_signature("J. M.\nAccounts", "<p>J. M.</p><p>Accounts</p>"), Shown::plain("J. M.\nAccounts"));
        assert_eq!(page_signature("", "<p><img src=\"/private/files/logo.png\"></p>"), Shown::plain(""));
        assert_eq!(page_signature("J. M.", &"<p>x</p>".repeat(30_000)), Shown::plain("J. M."));
    }

    // //// Neoffice — a Save that changed nothing keeps nothing (maintenance#1670).
    /// The signature editor opened and saved without a change gives the page's signature back, so nothing is kept as
    /// made here, and a change of its look in Neoffice still reaches NeoMail. A real change is kept.
    #[test]
    fn a_save_that_changed_nothing_keeps_nothing() {
        use mailcraft_engine::{Session, mime::rich, model::AccountConfig, model::dates::Clock, store::Store};
        let after_save = |page: &Shown, change: Option<&str>| -> Shown {
            let store = Store::memory();
            let config = AccountConfig {
                id: "a1".into(),
                email: "jeanne@example.ch".into(),
                signature: page.text.clone(),
                signature_html: page.html.clone(),
                ..AccountConfig::default()
            };
            store.save_accounts(&[config]).unwrap();
            let mut session = Session::open(Some(store), Clock::new(0, 0));
            // As the signature editor opens it, and what its Save sends (crates/ui-egui/src/dialogs.rs).
            let (mut text, mut styles) = session.signature_rich("a1");
            if let Some(more) = change {
                let end = text.chars().count();
                rich::splice(&mut text, &mut styles, end..end, more, &rich::Styles::plain(more));
            }
            let text = text.trim_end().to_string();
            styles.fit(&text);
            let html = if styles.is_plain() { None } else { Some(rich::to_html(&text, &styles)) };
            session.run("account.setSignature", &serde_json::json!({"account": "a1", "signature": text, "html": html})).unwrap();
            let config = &session.account("a1").unwrap().config;
            Shown { text: config.signature.clone(), html: config.signature_html.clone() }
        };
        let plain = page_signature("Jeanne Muster\nAccounts", "<p>Jeanne Muster</p><p>Accounts</p>");
        assert_eq!(keep(&plain, &after_save(&plain, None)), None);
        // A bold name, an empty line, and links whose words are not their address (the page's text says « LinkedIn
        // <https://…> », the look « LinkedIn »).
        let html = "<div class=\"ql-editor read-mode\"><p><strong>Jeanne Muster</strong></p><p><br></p>\
<p><a href=\"https://www.linkedin.com/in/jeanne\">LinkedIn</a> · <a href=\"https://www.example.ch\">www.example.ch</a></p></div>";
        let look = page_signature("Jeanne Muster\n\nLinkedIn <https://www.linkedin.com/in/jeanne> · www.example.ch", html);
        assert!(!look.html.is_empty());
        assert_eq!(keep(&look, &after_save(&look, None)), None);
        assert!(keep(&look, &after_save(&look, Some("\nAccounts"))).is_some(), "a real change is kept");
        assert!(keep(&plain, &after_save(&plain, Some(" (CFO)"))).is_some(), "a real change is kept");
    }

    // //// Neoffice — an entry that is the page's after all goes (maintenance#1670).
    /// Kept before a page's look was told by itself, an entry may be the page's signature, look and all: it hid later
    /// changes of the look in Neoffice. It is cleared by the first save, that very entry only.
    #[test]
    fn an_entry_that_is_the_page_s_is_cleared_once() {
        let page = page_signature("Jeanne Muster", "<p><strong>Jeanne Muster</strong></p>");
        let same = Kept { made: page.clone(), base: page.text.clone() };
        assert_eq!(seen(&page, Some(&same)), Seen::Stale(same.clone()));
        let email = "jeanne@example.ch".to_string();
        let at_start: HashMap<String, Seen> = HashMap::from([(email.clone(), seen(&page, Some(&same)))]);
        let cleared = changes(&[(email, page.clone(), page.clone())], &at_start);
        assert_eq!((cleared[0].kept.clone(), cleared[0].only_over.clone()), (None, Some(same)));
        // One really made here stays.
        let mine = Kept { made: Shown::plain("J. M."), base: page.text.clone() };
        assert_eq!(seen(&page, Some(&mine)), Seen::Was(Shown::plain("J. M.")));
    }

    #[test]
    fn a_tab_writes_only_what_it_changed() {
        let page = Shown::plain("Jeanne Muster");
        let email = "jeanne@example.ch".to_string();
        let mut seen_now: HashMap<String, Seen> = HashMap::from([(email.clone(), seen(&page, None))]);
        // Started with the page's and nothing changed: nothing to write.
        assert!(changes(&[(email.clone(), page.clone(), page.clone())], &seen_now).is_empty());
        // Changed here: one change, kept with the page's it replaced.
        let mine = Shown::plain("J. M.");
        let made = changes(&[(email.clone(), page.clone(), mine.clone())], &seen_now);
        assert_eq!(made.len(), 1);
        assert_eq!(made[0].kept, Some(Kept { made: mine.clone(), base: page.text.clone() }));
        assert_eq!(made[0].only_over, None);
        // Once written, the same again is nothing.
        seen_now.insert(email.clone(), Seen::Was(made[0].now.clone()));
        assert!(changes(&[(email.clone(), page.clone(), mine)], &seen_now).is_empty());
        // What no longer stood at the start: cleared once, that very entry only.
        let stale = Kept { made: Shown::plain("Old"), base: "Jeanne Muster\nAccounts".into() };
        let at_start: HashMap<String, Seen> = HashMap::from([(email.clone(), seen(&page, Some(&stale)))]);
        let cleared = changes(&[(email.clone(), page.clone(), page.clone())], &at_start);
        assert_eq!((cleared[0].kept.clone(), cleared[0].only_over.clone()), (None, Some(stale)));
    }

    #[test]
    fn a_change_of_any_signature_or_name_changes_the_fingerprint() {
        let one = ["jeanne@example.ch", "Jeanne Muster", "", "Jeanne"];
        let base = fingerprint([one].into_iter());
        assert_eq!(fingerprint([one].into_iter()), base);
        for i in 0..4 {
            let mut changed = one;
            changed[i] = "other";
            assert_ne!(fingerprint([changed].into_iter()), base, "field {i}");
        }
        // Where one ends and the next begins counts too.
        assert_ne!(fingerprint([["ab", "", "", ""]].into_iter()), fingerprint([["a", "b", "", ""]].into_iter()));
        assert_ne!(fingerprint([one, one].into_iter()), base);
    }

    #[test]
    fn a_save_writes_this_tab_s_changes_and_leaves_the_rest_as_it_is() {
        // Kept before signatures had a look: read as text.
        let before = r#"{"jeanne@example.ch":{"text":"J. M.","base":"Jeanne Muster"},"shared@example.ch":{"text":"Team","base":""}}"#;
        let kept = read(Some(before));
        assert_eq!(kept.get("jeanne@example.ch"), Some(&Kept { made: Shown::plain("J. M."), base: "Jeanne Muster".into() }));
        // This tab changed Jeanne's (now with a look), and nothing of the shared mailbox it doesn't have: that one stays.
        let mine = Kept { made: Shown { text: "Jeanne".into(), html: "<div><b>Jeanne</b></div>".into() }, base: "Jeanne Muster".into() };
        let change =
            |email: &str, kept: Option<Kept>, only_over: Option<Kept>| Change { email: email.into(), kept, only_over, now: Shown::default() };
        let written = write(Some(before), &[change("Jeanne@Example.ch", Some(mine.clone()), None)]).unwrap();
        let again = read(Some(&written));
        assert_eq!(again.get("jeanne@example.ch"), Some(&mine));
        assert_eq!(again.get("shared@example.ch"), Some(&Kept { made: Shown::plain("Team"), base: String::new() }));
        // The first save after a change in Neoffice clears what no longer stood, when it is still what is stored.
        let jeanne = read(Some(before)).get("jeanne@example.ch").cloned();
        let cleared = read(write(Some(before), &[change("jeanne@example.ch", None, jeanne)]).as_deref());
        assert_eq!((cleared.contains_key("jeanne@example.ch"), cleared.contains_key("shared@example.ch")), (false, true));
        // Clearing what no longer stood leaves alone what another tab wrote since.
        let stale = Kept { made: Shown::plain("J. M."), base: "Jeanne Muster".into() };
        assert_eq!(write(Some(&written), &[change("jeanne@example.ch", None, Some(stale))]), Some(written.clone()));
        // Back to the page's: that entry goes; the last one gone, nothing is left to keep.
        let fewer = write(Some(&written), &[change("jeanne@example.ch", None, None)]).unwrap();
        assert_eq!(read(Some(&fewer)).len(), 1);
        assert_eq!(write(Some(&fewer), &[change("shared@example.ch", None, None)]), None);
        // What can't be read is nothing kept.
        assert!(read(Some("not json")).is_empty());
        assert!(read(None).is_empty());
    }
}
