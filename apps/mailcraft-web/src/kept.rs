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

/// A signature (or a name shown on mail) made or changed here, and the hosting page's it replaced.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Kept {
    pub text: String,
    /// What the page gave when it was made (empty: none).
    pub base: String,
}

/// The signature (or name) a mailbox starts with: the one made here, while the page still gives the one it replaced (one
/// changed in Neoffice since is Neoffice's); else the page's.
pub fn chosen(page: &str, kept: Option<&Kept>) -> String {
    match kept {
        Some(k) if k.base.trim() == page.trim() => k.text.clone(),
        _ => page.to_string(),
    }
}

/// What to keep of a mailbox's signature (or name), `current`, now that the page gives `page`: nothing while it is the
/// page's.
pub fn keep(page: &str, current: &str) -> Option<Kept> {
    (current.trim() != page.trim()).then(|| Kept { text: current.to_string(), base: page.to_string() })
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
        // None made here: the page's, which nothing needs to keep.
        assert_eq!(chosen("Jeanne Muster\nAccounts", None), "Jeanne Muster\nAccounts");
        assert_eq!(keep("Jeanne Muster\nAccounts", "Jeanne Muster\nAccounts"), None);
        // Made here: kept with the page's it replaced, and taken at the next visit while the page gives that one.
        let kept = keep("Jeanne Muster\nAccounts", "Jeanne\nHead of accounts").unwrap();
        assert_eq!(kept, Kept { text: "Jeanne\nHead of accounts".into(), base: "Jeanne Muster\nAccounts".into() });
        assert_eq!(chosen("Jeanne Muster\nAccounts", Some(&kept)), "Jeanne\nHead of accounts");
        // Changed in Neoffice since: Neoffice's, and the one made here goes at the next save.
        assert_eq!(chosen("Jeanne Muster\nFinance", Some(&kept)), "Jeanne Muster\nFinance");
        assert_eq!(keep("Jeanne Muster\nFinance", "Jeanne Muster\nFinance"), None);
        // A mailbox the page gives no signature for: one made here is kept as well.
        let made = keep("", "J. M.").unwrap();
        assert_eq!(chosen("", Some(&made)), "J. M.");
    }
}
