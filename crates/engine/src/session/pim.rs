//! Calendar and People: each account's events and contacts, kept in its cache (`pim.json`).
//! Accounts whose server offers CalDAV / CardDAV sync them (see `net::dav`); the others, and the
//! sample mailbox, keep a calendar and an address book on this computer.

use mailcraft_model::dates::{civil_from_days, days_from_civil};
use mailcraft_model::ical::{self, CalEvent};
use mailcraft_model::pim::{LOCAL, Pim, StoredCard, StoredEvent, Tombstone};
// //// Neoffice — the occurrences a server expanded (JMAP), which carry their series there (update_occurrence).
use mailcraft_model::pim::{SERIES_CHANGE, SERIES_CHANGE_LINE, SERIES_CHANGE_RULE, SERIES_LINE, SERIES_RULE_LINE, series_line, set_series_line};
use mailcraft_model::vcard::Card;

use super::*;

pub use mailcraft_model::pim::PIM_DOC;

/// The part of the app on screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Module {
    #[default]
    Mail,
    Calendar,
    People,
    /// Outlook's Notes.
    Notes,
}

/// How much of the calendar is shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CalMode {
    Day,
    #[default]
    Week,
    Month,
}

/// One occurrence of an event, ready to draw. Times are Unix seconds; all-day events start at
/// local midnight of their first day.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgendaItem {
    pub account: String,
    pub calendar: String,
    pub uid: String,
    /// When this occurrence starts as the event's rule has it (names it for edits).
    pub occurrence: i64,
    pub recurring: bool,
    pub summary: String,
    pub location: String,
    pub start: i64,
    pub end: i64,
    pub all_day: bool,
    pub color: u8,
    /// The colour of its first category (an index into the category palette), which Outlook
    /// draws an appointment in rather than its calendar's.
    pub category: Option<u8>,
    /// `TENTATIVE`, `CANCELLED`…
    pub status: String,
    /// The user's reply is awaited (`NEEDS-ACTION`).
    pub needs_reply: bool,
    // //// Neoffice — a day-long event's days as they are (its first, and the day after its last), not times: in a zone
    // whose clock changes at midnight, a day's midnight is no time on its clock (`start` and `end` above are the times its
    // days start, where the clock shows them), and the calendar draws it on its own days whatever the offset.
    pub days: Option<(i64, i64)>,
}

/// A person in an address book.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Person {
    pub account: String,
    pub book: String,
    pub card: Card,
}

// //// Neoffice — said when a change of an occurrence for the whole series meets one for that occurrence alone that hasn't
// gone yet, or the other way round (Session::update_occurrence).
pub const CHANGE_NOT_SENT: &str = "an earlier change of this event has not been sent yet: send and receive, then try again";

impl Session {
    pub(crate) fn load_pim(&mut self) {
        let Some(store) = self.store.clone() else { return };
        for a in &self.accounts {
            let mut p: Pim = store.load_account_doc(&a.config.id, PIM_DOC).unwrap_or_default();
            p.ensure_defaults();
            self.pim.insert(a.config.id.clone(), p);
        }
    }

    pub(crate) fn save_pim(&mut self, account: &str) {
        self.contacts = None;
        self.pim_revision = self.pim_revision.wrapping_add(1);
        let (Some(store), Some(p)) = (&self.store, self.pim.get(account)) else { return };
        if let Err(e) = store.save_account_doc(account, PIM_DOC, p) {
            self.notice(format!("calendar and contacts could not be saved: {e}"), true);
        }
    }

    /// An account's calendars and address books (made on first use).
    pub fn pim_mut(&mut self, account: &str) -> &mut Pim {
        self.pim.entry(account.to_string()).or_insert_with(|| {
            let mut p = Pim::default();
            p.ensure_defaults();
            p
        })
    }

    /// The account new events and people go to: the one on screen, else the first.
    fn pim_account(&self, wanted: Option<&str>) -> Result<String, CmdError> {
        match wanted {
            Some(a) if self.account(a).is_some() => Ok(a.to_string()),
            Some(a) => Err(CmdError::Params(format!("no account `{a}`"))),
            None => self.current_account().map(|a| a.config.id.clone()).ok_or_else(|| CmdError::Disabled("add an account first".into())),
        }
    }

    // //// Neoffice — where new contacts and events can go: a writable book or calendar, and the one on this computer
    // only where it is kept (`keeps_local_pim`: not in the browser build with a hosting page's mailboxes). The UI greys
    // « New contact », « Add sender to contacts » and « New event » when there is none.
    /// The first book of `account` new contacts can go to.
    pub fn new_card_book(&self, account: &str) -> Option<String> {
        let Some(p) = self.pim.get(account) else { return self.keeps_local_pim.then(|| LOCAL.to_string()) };
        p.books.iter().find(|b| !b.read_only && (self.keeps_local_pim || b.id != LOCAL)).map(|b| b.id.clone())
    }

    /// The first calendar of `account` new events can go to.
    pub fn new_event_calendar(&self, account: &str) -> Option<String> {
        let Some(p) = self.pim.get(account) else { return self.keeps_local_pim.then(|| LOCAL.to_string()) };
        p.calendars.iter().find(|c| !c.read_only && (self.keeps_local_pim || c.id != LOCAL)).map(|c| c.id.clone())
    }

    /// The account a new contact goes to, and its book: `wanted` (else the one on screen) when it has one; where the
    /// book here isn't kept, else the first account that has one (the contacts of a Neoffice instance are offered in one
    /// of the person's mailboxes).
    pub fn new_card_target(&self, wanted: Option<&str>) -> Option<(String, String)> {
        let first = match wanted {
            Some(a) => self.account(a),
            None => self.current_account(),
        }
        .map(|a| a.config.id.clone())?;
        if let Some(book) = self.new_card_book(&first) {
            return Some((first, book));
        }
        if self.keeps_local_pim {
            return None;
        }
        self.accounts.iter().find_map(|a| self.new_card_book(&a.config.id).map(|book| (a.config.id.clone(), book)))
    }

    /// Whether a new contact can be added (« New contact »).
    pub fn can_add_cards(&self) -> bool {
        self.new_card_target(None).is_some()
    }

    /// Whether a new event can be added (« New event », whose form offers the calendars of every account).
    pub fn can_add_events(&self) -> bool {
        self.accounts.iter().any(|a| self.new_event_calendar(&a.config.id).is_some())
    }

    // ----- the calendar view -----

    /// The local day (days since the epoch) the calendar shows.
    pub fn cal_day(&self) -> i64 {
        // //// Neoffice — by today's own offset (Clock::zone).
        self.cal_day.unwrap_or_else(|| self.clock.local(self.clock.now).div_euclid(86_400))
    }

    /// The local days on screen: a day, the week (Monday first) or the weeks of the month.
    pub fn cal_days(&self) -> (i64, i64) {
        let d = self.cal_day();
        let monday = |x: i64| x - (x + 3).rem_euclid(7);
        match self.cal_mode {
            CalMode::Day => (d, d + 1),
            CalMode::Week => (monday(d), monday(d) + 7),
            CalMode::Month => {
                let (y, m, _) = civil_from_days(d);
                let first = monday(days_from_civil(y, m, 1));
                (first, first + 42)
            }
        }
    }

    /// The Unix time a local day starts.
    pub fn local_midnight(&self, day: i64) -> i64 {
        // //// Neoffice — by that day's own offset (Clock::zone: daylight saving), not today's.
        self.clock.utc(day.saturating_mul(86_400))
    }

    /// Move the calendar: to a date, today, or by `step` days, weeks or months (its mode).
    pub fn cal_goto(&mut self, date: Option<&str>, step: i64) -> CmdResult {
        let mut d = self.cal_day();
        match date.map(str::trim) {
            // //// Neoffice — by today's own offset (Clock::zone).
            Some("today") | Some("") => d = self.clock.local(self.clock.now).div_euclid(86_400),
            Some(s) => {
                d = parse_local(s, 0)
                    .map(|(t, _)| t.div_euclid(86_400))
                    .ok_or_else(|| CmdError::Params(format!("`{s}` is not a date (YYYY-MM-DD)")))?;
            }
            None => {}
        }
        let step = step.clamp(-1200, 1200);
        d = match self.cal_mode {
            CalMode::Day => d + step,
            CalMode::Week => d + 7 * step,
            CalMode::Month => {
                let (y, m, day) = civil_from_days(d);
                let total = i64::from(m) - 1 + step;
                let (yy, mm) = (y + total.div_euclid(12), u32::try_from(total.rem_euclid(12) + 1).unwrap_or(1));
                days_from_civil(yy, mm, day.min(28))
            }
        };
        self.cal_day = Some(d);
        let (y, m, dd) = civil_from_days(d);
        Ok(serde_json::json!({"date": format!("{y:04}-{m:02}-{dd:02}"), "mode": self.cal_mode}))
    }

    /// The key a calendar is shown or hidden under (`Prefs::hidden_calendars`).
    pub fn calendar_key(account: &str, calendar: &str) -> String {
        format!("{account}/{calendar}")
    }

    /// Whether a calendar is shown (ticked) in the calendar.
    pub fn calendar_shown(&self, account: &str, calendar: &str) -> bool {
        !self.prefs.hidden_calendars.contains(&Session::calendar_key(account, calendar))
    }

    // //// Neoffice — whether a calendar is offered at all (the side pane, side by side): not the one on this computer,
    // empty, where nothing put there is kept (the browser build in a hosting page: `keeps_local_pim`); one holding events
    // from before still is.
    pub fn calendar_listed(&self, account: &str, calendar: &str) -> bool {
        self.keeps_local_pim || calendar != LOCAL || self.pim.get(account).is_some_and(|p| p.events.iter().any(|e| e.calendar == LOCAL))
    }

    /// The calendars shown, in the order the side pane lists them: (account, id, name, colour).
    pub fn shown_calendars(&self) -> Vec<(String, String, String, u8)> {
        self.accounts
            .iter()
            .filter_map(|a| self.pim.get(&a.config.id).map(|p| (a, p)))
            .flat_map(|(a, p)| p.calendars.iter().map(move |c| (a.config.id.clone(), c.id.clone(), c.name.clone(), c.color)))
            // //// Neoffice — and offered (calendar_listed).
            .filter(|(a, c, _, _)| self.calendar_shown(a, c) && self.calendar_listed(a, c))
            .collect()
    }

    /// The occurrences overlapping `[from, to)`, by start, from every calendar that's shown.
    pub fn agenda(&self, from: i64, to: i64) -> Vec<AgendaItem> {
        // //// Neoffice — no longer today's offset for every date (below: each date's own, Clock::zone).
        let mut out = Vec::new();
        for (account, p) in &self.pim {
            for cal in p.calendars.iter().filter(|c| self.calendar_shown(account, &c.id)) {
                let stored: Vec<&StoredEvent> = p.events.iter().filter(|e| e.calendar == cal.id).collect();
                let events: Vec<CalEvent> = stored.iter().map(|e| e.event.clone()).collect();
                // All-day events are dates, not times: widen the window by a day each way.
                for o in ical::occurrences(&events, from - 86_400, to + 86_400) {
                    let Some(e) = events.get(o.event) else { continue };
                    // //// Neoffice — by the offset of each date (Clock::zone: daylight saving): a day-long event from its
                    // days' own midnights; an occurrence its rule repeats at the hour of the first, the rule being kept in
                    // UTC (the offset's change between them made up).
                    let (start, end) = if e.all_day {
                        (self.clock.utc(o.start), self.clock.utc(o.end))
                    } else if e.rrule.is_some() && e.recurrence_id.is_none() {
                        let shift = i64::from(self.clock.offset_at(e.start)) - i64::from(self.clock.offset_at(o.start));
                        (o.start.saturating_add(shift), o.end.saturating_add(shift))
                    } else {
                        (o.start, o.end)
                    };
                    if start >= to || end.max(start + 1) <= from {
                        continue;
                    }
                    let mine: Vec<String> =
                        self.accounts.iter().flat_map(|a| [a.config.email.to_ascii_lowercase(), a.config.username.to_ascii_lowercase()]).collect();
                    let needs_reply = e.attendees.iter().any(|a| mine.contains(&a.email.to_ascii_lowercase()) && a.partstat == "NEEDS-ACTION");
                    out.push(AgendaItem {
                        account: account.clone(),
                        calendar: cal.id.clone(),
                        uid: e.uid.clone(),
                        occurrence: e.recurrence_id.unwrap_or(o.start),
                        recurring: e.rrule.is_some() || e.recurrence_id.is_some(),
                        summary: e.summary.clone(),
                        location: e.location.clone(),
                        start,
                        end,
                        all_day: e.all_day,
                        color: cal.color,
                        category: e.categories.first().map(|c| self.category_color_of(c)),
                        status: e.status.clone(),
                        needs_reply,
                        // //// Neoffice — see `AgendaItem::days`.
                        days: e.all_day.then(|| (o.start.div_euclid(86_400), o.end.div_euclid(86_400).max(o.start.div_euclid(86_400) + 1))),
                    });
                }
            }
        }
        out.sort_by(|a, b| (b.all_day, a.start, &a.summary).cmp(&(a.all_day, b.start, &b.summary)));
        out
    }

    /// A category's colour by its name: the one made with that name (mail and events share
    /// them), else one picked from the name (a category from the server MailCraft has no colour
    /// for still gets the same one every time).
    pub fn category_color_of(&self, name: &str) -> u8 {
        if let Some(c) = self.prefs.categories.iter().find(|c| c.name.eq_ignore_ascii_case(name.trim())) {
            return c.color;
        }
        let h = name.trim().to_lowercase().bytes().fold(5381u32, |h, b| h.wrapping_mul(33) ^ u32::from(b));
        u8::try_from(h % 8).unwrap_or(0)
    }

    /// The event (master, not an instance) with a uid, and its account.
    pub fn find_event(&self, uid: &str) -> Option<(&str, &StoredEvent)> {
        self.pim.iter().find_map(|(a, p)| p.events.iter().find(|e| e.event.uid == uid && e.event.recurrence_id.is_none()).map(|e| (a.as_str(), e)))
    }

    // //// Neoffice — the copy of an event the person can change: one in a calendar that isn't read-only, as another's
    // read-only calendar (a colleague's, shared) may hold the same event, listed first; else the first. And an occurrence
    // by when it starts (`at`, as the event's rule has it): a server's calendar read through JMAP holds the occurrences of
    // a series it expanded, and no event of the series.
    /// The event (master) with a uid the person can change, else the first, and its account.
    pub fn find_event_to_change(&self, uid: &str) -> Option<(&str, &StoredEvent)> {
        self.find_copy(uid, None).or_else(|| self.find_event(uid))
    }

    /// The occurrence starting `at` of the event with a uid: the copy the person can change, else the first.
    pub fn find_occurrence(&self, uid: &str, at: i64) -> Option<(&str, &StoredEvent)> {
        self.find_copy(uid, Some(at)).or_else(|| {
            self.pim
                .iter()
                .find_map(|(a, p)| p.events.iter().find(|e| e.event.uid == uid && e.event.recurrence_id == Some(at)).map(|e| (a.as_str(), e)))
        })
    }

    /// The copy of the event `uid` (`at`: the occurrence starting then; `None`: the event itself) in a calendar that can
    /// be changed.
    fn find_copy(&self, uid: &str, at: Option<i64>) -> Option<(&str, &StoredEvent)> {
        self.pim.iter().find_map(|(a, p)| {
            let writable = |e: &&StoredEvent| !p.calendars.iter().any(|c| c.id == e.calendar && c.read_only);
            p.events.iter().filter(|e| e.event.uid == uid && e.event.recurrence_id == at).find(writable).map(|e| (a.as_str(), e))
        })
    }

    /// Whether this is the browser build with a hosting page's mailboxes (nothing kept here: `keeps_local_pim` off), where
    /// the servers and the sign-in are the page's.
    pub fn hosted(&self) -> bool {
        !self.keeps_local_pim
    }

    /// Whether a calendar's server sends the meeting mail itself (JMAP's `sendSchedulingMessages`, calendars.rs in
    /// mailcraft-jmap): MailCraft doesn't send it too.
    pub fn server_schedules(&self, account: &str, calendar: &str) -> bool {
        calendar != LOCAL && self.pim.get(account).is_some_and(|p| p.caldav_home.as_deref() == Some("jmap"))
    }

    fn new_uid(&mut self) -> String {
        let n = self.fresh_id();
        format!("{:016x}-{n}@mailcraft", self.seed)
    }

    /// Add an event; returns its uid. Events in a server calendar are sent there.
    pub fn create_event(&mut self, account: Option<&str>, calendar: Option<&str>, mut e: CalEvent) -> CmdResult {
        let account = self.pim_account(account)?;
        check_event(&e)?;
        if e.uid.trim().is_empty() {
            e.uid = self.new_uid();
        }
        // //// Neoffice — none named: the first calendar this account's new events can go to (new_event_calendar), not the
        // one on this computer where it isn't kept; nor that one when named.
        let (keeps, first) = (self.keeps_local_pim, self.new_event_calendar(&account));
        let p = self.pim_mut(&account);
        let cal = match calendar {
            Some(c) => p
                .calendars
                .iter()
                .find(|x| x.id == c || x.name.eq_ignore_ascii_case(c))
                .ok_or_else(|| CmdError::Params(format!("no calendar `{c}`")))?,
            // //// Neoffice — see `first` above.
            None => first.and_then(|f| p.calendars.iter().find(|c| c.id == f)).ok_or_else(|| CmdError::Disabled("no calendar to add to".into()))?,
        };
        if cal.read_only {
            return Err(CmdError::Params(format!("{} can't be changed", cal.name)));
        }
        // //// Neoffice — see `keeps` above: it would be lost at the next reload.
        if !keeps && cal.id == LOCAL {
            return Err(CmdError::Disabled("the calendar on this computer isn't kept here".into()));
        }
        let id = cal.id.clone();
        let uid = e.uid.clone();
        p.events.retain(|x| x.event.uid != uid);
        p.events.push(StoredEvent { calendar: id.clone(), href: String::new(), etag: String::new(), event: e, dirty: id != LOCAL });
        self.after_pim_change(&account, id != LOCAL);
        Ok(serde_json::json!({"uid": uid, "account": account}))
    }

    /// Change an event (all of its occurrences).
    pub fn update_event(&mut self, uid: &str, change: impl FnOnce(&mut CalEvent)) -> CmdResult {
        // //// Neoffice — the copy the person can change (find_event_to_change), not one in a read-only calendar listed first.
        let (account, _) = self
            .find_event_to_change(uid)
            .map(|(a, e)| (a.to_string(), e.calendar.clone()))
            .ok_or_else(|| CmdError::Params(format!("no event `{uid}`")))?;
        let p = self.pim_mut(&account);
        // //// Neoffice — see above: that copy, else the first (refused below when it can't be changed either).
        let locked: Vec<String> = p.calendars.iter().filter(|c| c.read_only).map(|c| c.id.clone()).collect();
        let master = |e: &StoredEvent| e.event.uid == uid && e.event.recurrence_id.is_none();
        let at = p.events.iter().position(|e| master(e) && !locked.contains(&e.calendar)).or_else(|| p.events.iter().position(master));
        // //// Neoffice — its copy as read, before it changes (Pim::keep_as_read).
        if let Some(i) = at {
            p.keep_as_read(i);
        }
        let Some(stored) = at.and_then(|i| p.events.get_mut(i)) else {
            return Err(CmdError::Params(format!("no event `{uid}`")));
        };
        if p.calendars.iter().any(|c| c.id == stored.calendar && c.read_only) {
            return Err(CmdError::Params("this calendar can't be changed".into()));
        }
        let mut e = stored.event.clone();
        change(&mut e);
        check_event(&e)?;
        e.sequence = e.sequence.saturating_add(1);
        stored.event = e;
        let remote = stored.calendar != LOCAL;
        stored.dirty |= remote;
        self.after_pim_change(&account, remote);
        Ok(serde_json::json!({"uid": uid}))
    }

    // //// Neoffice — an occurrence a server expanded (JMAP: no event of the series here, find_occurrence) is changed alone,
    // by its own address there, or for the whole series (`series`), which it carries to the server (SERIES_CHANGE_LINE)
    // while the other occurrences show the change at once. An occurrence has no rule of its own: the rule `change` gives
    // (starting from the series', SERIES_RULE_LINE) is the series'.
    pub fn update_occurrence(&mut self, uid: &str, at: i64, series: bool, change: impl FnOnce(&mut CalEvent)) -> CmdResult {
        let account = self.find_occurrence(uid, at).map(|(a, _)| a.to_string()).ok_or_else(|| CmdError::Params(format!("no event `{uid}`")))?;
        let p = self.pim_mut(&account);
        let locked: Vec<String> = p.calendars.iter().filter(|c| c.read_only).map(|c| c.id.clone()).collect();
        let Some(i) = p.events.iter().position(|e| e.event.uid == uid && e.event.recurrence_id == Some(at) && !locked.contains(&e.calendar)) else {
            return Err(CmdError::Params("this calendar can't be changed".into()));
        };
        let Some((old, calendar, waiting)) = p.events.get(i).map(|x| (x.event.clone(), x.calendar.clone(), x.dirty)) else {
            return Err(CmdError::Params(format!("no event `{uid}`")));
        };
        // //// Neoffice — a change of this occurrence not sent yet goes with its scope (SERIES_CHANGE_LINE: the whole series,
        // or this occurrence alone): a change for the other scope waits until it has gone, else both would be sent for one
        // of them; a change of the series' rule waiting stays one.
        let pending = if waiting { series_line(&old, SERIES_CHANGE_LINE).map(str::to_string) } else { None };
        if waiting && pending.is_some() != series {
            return Err(CmdError::Params(CHANGE_NOT_SENT.into()));
        }
        let mut e = old.clone();
        e.rrule = series_line(&old, SERIES_RULE_LINE).map(str::to_string);
        change(&mut e);
        check_event(&e)?;
        // It stays that occurrence.
        (e.uid, e.recurrence_id) = (old.uid.clone(), old.recurrence_id);
        let rule = e.rrule.take();
        if series {
            let rule_changed = rule.as_deref() != series_line(&old, SERIES_RULE_LINE) || pending.as_deref() == Some(SERIES_CHANGE_RULE);
            if rule_changed {
                set_series_line(&mut e, SERIES_RULE_LINE, rule.as_deref());
            }
            set_series_line(&mut e, SERIES_CHANGE_LINE, Some(if rule_changed { SERIES_CHANGE_RULE } else { SERIES_CHANGE }));
            // The other occurrences show it at once; the next read brings them as the server has them.
            let base = series_line(&old, SERIES_LINE).map(str::to_string);
            let (moved, length) = (e.start - old.start, e.end - e.start);
            for x in p.events.iter_mut().filter(|x| {
                x.event.uid == uid
                    && x.calendar == calendar
                    && x.event.recurrence_id != Some(at)
                    && base.is_some()
                    && series_line(&x.event, SERIES_LINE) == base.as_deref()
            }) {
                x.event.summary.clone_from(&e.summary);
                x.event.location.clone_from(&e.location);
                x.event.description.clone_from(&e.description);
                x.event.categories.clone_from(&e.categories);
                x.event.attendees.clone_from(&e.attendees);
                x.event.all_day = e.all_day;
                x.event.start = x.event.start.saturating_add(moved);
                x.event.end = x.event.start.saturating_add(length);
            }
        } else {
            // //// Neoffice — this occurrence alone: no scope left from a change of the series already sent.
            set_series_line(&mut e, SERIES_CHANGE_LINE, None);
        }
        e.sequence = e.sequence.saturating_add(1);
        let remote = calendar != LOCAL;
        // Its copy as read, before it changes (Pim::keep_as_read): what the form showed, for the whole series too.
        p.keep_as_read(i);
        if let Some(x) = p.events.get_mut(i) {
            x.event = e;
            x.dirty |= remote;
        }
        self.after_pim_change(&account, remote);
        Ok(serde_json::json!({"uid": uid, "occurrence": at}))
    }

    // //// Neoffice — the occurrences a server expanded, with no event of the series here (find_occurrence): one deleted
    // by its own address there, the whole series by its base event's (SERIES_LINE), once; only the copies in calendars
    // that can be changed.
    fn delete_occurrences(&mut self, uid: &str, occurrence: Option<i64>) -> CmdResult {
        let Some(account) = self.pim.iter().find(|(_, p)| p.events.iter().any(|e| e.event.uid == uid)).map(|(a, _)| a.clone()) else {
            return Err(CmdError::Params(format!("no event `{uid}`")));
        };
        let p = self.pim_mut(&account);
        let locked: Vec<String> = p.calendars.iter().filter(|c| c.read_only).map(|c| c.id.clone()).collect();
        let going =
            |e: &StoredEvent| e.event.uid == uid && !locked.contains(&e.calendar) && occurrence.is_none_or(|at| e.event.recurrence_id == Some(at));
        let gone: Vec<StoredEvent> = p.events.iter().filter(|e| going(e)).cloned().collect();
        if gone.is_empty() {
            return Err(CmdError::Params("this calendar can't be changed".into()));
        }
        let mut remote = false;
        for g in &gone {
            let href = match (occurrence, series_line(&g.event, SERIES_LINE)) {
                (None, Some(base)) => base.to_string(),
                _ => g.href.clone(),
            };
            if href.is_empty() || p.deleted.iter().any(|t| t.href == href) {
                continue;
            }
            remote = true;
            p.deleted.push(Tombstone { href, etag: String::new(), calendar: g.calendar.clone() });
        }
        p.events.retain(|e| !going(e));
        self.after_pim_change(&account, remote);
        Ok(serde_json::json!({"deleted": uid}))
    }

    /// Delete an event, or one occurrence of a recurring one (`occurrence`: its start).
    pub fn delete_event(&mut self, uid: &str, occurrence: Option<i64>) -> CmdResult {
        // //// Neoffice — occurrences a server expanded, with no event of the series here.
        if self.find_event(uid).is_none() {
            return self.delete_occurrences(uid, occurrence);
        }
        let account = self.find_event(uid).map(|(a, _)| a.to_string()).ok_or_else(|| CmdError::Params(format!("no event `{uid}`")))?;
        let p = self.pim_mut(&account);
        // //// Neoffice — only its copies in calendars that can be changed: another's read-only calendar (a colleague's,
        // shared) may hold the same event, and it stays there as it is; with no other copy, refused.
        let locked: Vec<String> = p.calendars.iter().filter(|c| c.read_only).map(|c| c.id.clone()).collect();
        if !p.events.iter().any(|e| e.event.uid == uid && !locked.contains(&e.calendar)) {
            return Err(CmdError::Params("this calendar can't be changed".into()));
        }
        let mut remote = false;
        match occurrence {
            Some(at) if p.events.iter().any(|e| e.event.uid == uid && e.event.rrule.is_some()) => {
                // One occurrence: an exception on the rule, and any changed instance goes.
                // //// Neoffice — see `locked` above.
                p.events.retain(|e| !(e.event.uid == uid && e.event.recurrence_id == Some(at) && !locked.contains(&e.calendar)));
                let writable = |e: &StoredEvent| e.event.uid == uid && e.event.recurrence_id.is_none() && !locked.contains(&e.calendar);
                // //// Neoffice — its copy as read, before it changes (Pim::keep_as_read).
                let master = p.events.iter().position(writable);
                if let Some(i) = master {
                    p.keep_as_read(i);
                }
                if let Some(m) = master.and_then(|i| p.events.get_mut(i)) {
                    if !m.event.exdates.contains(&at) {
                        m.event.exdates.push(at);
                    }
                    m.event.sequence = m.event.sequence.saturating_add(1);
                    remote = m.calendar != LOCAL;
                    m.dirty |= remote;
                }
            }
            _ => {
                // //// Neoffice — see `locked` above.
                let gone: Vec<StoredEvent> = p.events.iter().filter(|e| e.event.uid == uid && !locked.contains(&e.calendar)).cloned().collect();
                for g in gone.iter().filter(|g| !g.href.is_empty()) {
                    remote = true;
                    p.deleted.push(Tombstone { href: g.href.clone(), etag: g.etag.clone(), calendar: g.calendar.clone() });
                }
                // //// Neoffice — see `locked` above.
                p.events.retain(|e| e.event.uid != uid || locked.contains(&e.calendar));
            }
        }
        self.after_pim_change(&account, remote);
        Ok(serde_json::json!({"deleted": uid}))
    }

    /// Save, and ask for a sync when the server has something to learn.
    pub(crate) fn after_pim_change(&mut self, account: &str, remote: bool) {
        self.save_pim(account);
        if remote && self.account(account).is_some_and(|a| !a.is_local()) {
            self.jobs.push(Job::SyncDav { account: account.to_string() });
        }
    }

    // ----- syncing with the server -----

    /// Merge what a CalDAV / CardDAV sync found. Changes made here since stay (and go to the
    /// server next time); everything else follows the server.
    pub fn apply_dav(&mut self, account: &str, sync: &mailcraft_model::pim::DavSync) {
        // //// Neoffice — the cards whose change made here had to go (their book now read-only), said once below.
        let mut undone: Vec<String> = Vec::new();
        let p = self.pim_mut(account);
        p.caldav_home = sync.caldav_home.clone().or_else(|| Some(String::new()));
        p.carddav_home = sync.carddav_home.clone().or_else(|| Some(String::new()));
        p.dav_checked = Some(sync.checked);
        p.deleted.retain(|t| !sync.deleted.contains(&t.href));
        // What was stored: its address and version, and no longer to send (unless it changed
        // again meanwhile).
        for x in &sync.pushed_events {
            for e in p.events.iter_mut().filter(|e| e.event.uid == x.event.uid && e.event.recurrence_id == x.event.recurrence_id) {
                e.href = x.href.clone();
                e.etag = x.etag.clone();
                // Stored in another calendar (a new event goes to the server's default one).
                e.calendar = x.calendar.clone();
                e.dirty &= e.event != x.event;
            }
            // //// Neoffice — what was sent is what the server has of the person's change (Pim::originals): changed again
            // meanwhile, the next change starts from it; else no change is left to start.
            if p.events.iter().any(|e| e.dirty && !x.href.is_empty() && e.href == x.href) {
                p.originals.insert(x.href.clone(), x.event.clone());
            } else {
                p.originals.remove(&x.href);
            }
        }
        // Notes stored on the server: their item there, and no longer to send.
        for x in &sync.pushed_notes {
            for n in p.notes.iter_mut().filter(|n| n.note.uid == x.note.uid) {
                n.href = x.href.clone();
                n.etag = x.etag.clone();
                n.dirty &= n.note != x.note;
            }
        }
        // The server's notes, by their item there; those changed here (and those never sent)
        // stay.
        if let Some(server) = &sync.notes {
            p.notes.retain(|n| n.dirty || n.href.is_empty());
            for x in server {
                if !p.notes.iter().any(|n| !n.href.is_empty() && n.href == x.href) {
                    p.notes.push(x.clone());
                }
            }
        }
        // //// Neoffice — changes the server refused (said in `errors`): an event never stored there goes, one stored there
        // follows the server again (the read brings it back as the server has it).
        // Changed again since it was sent, it is a new change, which goes; refused as it was, it shows as it was read before
        // the change (Pim::originals) until the next read brings the server's.
        for x in &sync.refused_events {
            p.events.retain(|e| {
                !(e.href.is_empty() && e.event.uid == x.event.uid && e.event.recurrence_id == x.event.recurrence_id && e.event == x.event)
            });
            let as_read = p.originals.get(&x.href).cloned();
            for e in p.events.iter_mut().filter(|e| !e.href.is_empty() && e.href == x.href) {
                e.dirty &= e.event != x.event;
                if !e.dirty
                    && let Some(read) = &as_read
                {
                    e.event = read.clone();
                }
            }
            if !p.events.iter().any(|e| e.dirty && !x.href.is_empty() && e.href == x.href) {
                p.originals.remove(&x.href);
            }
        }
        // //// Neoffice — cards whose change the server refused for good (said in `errors`), likewise: one never stored there
        // goes, one stored there follows the server again (the read brings it back as the server has it).
        for x in &sync.refused_cards {
            p.cards.retain(|c| !(c.href.is_empty() && c.card.uid == x.card.uid && c.card == x.card));
            for c in p.cards.iter_mut().filter(|c| !c.href.is_empty() && c.href == x.href) {
                c.dirty &= c.card != x.card;
            }
        }
        for x in &sync.pushed_cards {
            for c in p.cards.iter_mut().filter(|c| c.card.uid == x.card.uid) {
                c.href = x.href.clone();
                c.etag = x.etag.clone();
                c.book = x.book.clone();
                c.dirty &= c.card != x.card;
            }
        }
        if let Some(server) = &sync.calendars {
            // The calendar on this computer goes once the server has calendars, unless it holds
            // events (they stay there).
            // //// Neoffice — or none of the server's can be added to (none, or all read-only, as a JMAP server's for
            // NeoMail): new events, and the invitations answered, need somewhere to go.
            let keep_local = server.iter().all(|c| c.read_only) || p.events.iter().any(|e| e.calendar == LOCAL);
            let mut list: Vec<mailcraft_model::pim::CalendarInfo> = server.clone();
            if keep_local && let Some(l) = p.calendars.iter().find(|c| c.id == LOCAL) {
                list.push(l.clone());
            }
            p.events.retain(|e| e.calendar == LOCAL || list.iter().any(|c| c.id == e.calendar) || e.dirty);
            for cal in &sync.synced_calendars {
                p.events.retain(|e| &e.calendar != cal || e.dirty || e.href.is_empty() || sync.kept.contains(&e.href));
                for x in sync.events.iter().filter(|x| &x.calendar == cal) {
                    // //// Neoffice — what has a change here waiting is the server's event at its address (each occurrence a
                    // server expanded is one of its own, JMAP's: the others of the series are read as usual), or one never
                    // stored there with its uid (waiting_change).
                    if !p.events.iter().any(|e| waiting_change(e, x)) {
                        p.events.retain(|e| !(e.event.uid == x.event.uid && e.event.recurrence_id == x.event.recurrence_id && !e.dirty));
                        p.events.push(x.clone());
                    }
                }
            }
            // //// Neoffice — a calendar read only in part: what was read is added or updated (by uid and start, or by
            // its address on the server), and nothing of it goes; changes made here meanwhile stay to be sent.
            for cal in &sync.partial_calendars {
                for x in sync.events.iter().filter(|x| &x.calendar == cal) {
                    let same = |e: &StoredEvent| {
                        (e.event.uid == x.event.uid && e.event.recurrence_id == x.event.recurrence_id) || (!x.href.is_empty() && e.href == x.href)
                    };
                    // As for a calendar read in full: an event changed here keeps the change (waiting_change).
                    if !p.events.iter().any(|e| (e.dirty && same(e)) || waiting_change(e, x)) {
                        p.events.retain(|e| !same(e));
                        p.events.push(x.clone());
                    }
                }
                // What it held and the last reads in a row didn't bring goes after PARTIAL_MISSES of them, as it would
                // from a read in full; an occurrence wholly outside the span read can't come back, and goes now.
                let read: std::collections::HashSet<&str> = sync.events.iter().filter(|x| &x.calendar == cal).map(|x| x.href.as_str()).collect();
                let misses = &mut p.partial_misses;
                p.events.retain(|e| {
                    if &e.calendar != cal || e.dirty || e.href.is_empty() || read.contains(e.href.as_str()) {
                        return true;
                    }
                    let outside =
                        e.event.rrule.is_none() && sync.window.is_some_and(|(from, to)| e.event.end.max(e.event.start) < from || e.event.start > to);
                    let missed = misses.entry(e.href.clone()).or_insert(0);
                    *missed = missed.saturating_add(1);
                    !outside && *missed < mailcraft_model::pim::PARTIAL_MISSES
                });
            }
            p.calendars = list;
        }
        if let Some(server) = &sync.books {
            // //// Neoffice — likewise the address book here, while none of the server's can be added to: « New contact »
            // and « Add sender to contacts » need somewhere to go.
            let keep_local = server.iter().all(|b| b.read_only) || p.cards.iter().any(|c| c.book == LOCAL);
            let mut list: Vec<mailcraft_model::pim::BookInfo> = server.clone();
            if keep_local && let Some(l) = p.books.iter().find(|b| b.id == LOCAL) {
                list.push(l.clone());
            }
            p.cards.retain(|c| c.book == LOCAL || list.iter().any(|b| b.id == c.book) || c.dirty);
            // //// Neoffice — a card changed here in a book the server no longer lets the person write can't be sent: the
            // change goes (said once), and the server's card comes back with this read; one made here, never stored, goes.
            let locked: Vec<&str> = server.iter().filter(|b| b.read_only).map(|b| b.id.as_str()).collect();
            p.cards.retain_mut(|c| {
                if !c.dirty || !locked.contains(&c.book.as_str()) {
                    return true;
                }
                undone.push(c.card.display());
                c.dirty = false;
                !c.href.is_empty()
            });
            for book in &sync.synced_books {
                p.cards.retain(|c| &c.book != book || c.dirty || c.href.is_empty() || sync.kept.contains(&c.href));
                for x in sync.cards.iter().filter(|x| &x.book == book) {
                    if !p.cards.iter().any(|c| c.dirty && c.card.uid == x.card.uid) {
                        p.cards.retain(|c| c.card.uid != x.card.uid);
                        p.cards.push(x.clone());
                    }
                }
            }
            // //// Neoffice — a book read only in part: likewise, what was read is added or updated (by uid, or by its
            // address on the server: a card made here and stored there comes back under the server's uid), and nothing
            // of it goes.
            for book in &sync.partial_books {
                for x in sync.cards.iter().filter(|x| &x.book == book) {
                    let same = |c: &StoredCard| c.card.uid == x.card.uid || (!x.href.is_empty() && c.href == x.href);
                    if !p.cards.iter().any(|c| c.dirty && same(c)) {
                        p.cards.retain(|c| !same(c));
                        p.cards.push(x.clone());
                    }
                }
                // What it held and the last reads in a row didn't bring goes after PARTIAL_MISSES of them.
                let read: std::collections::HashSet<&str> = sync.cards.iter().filter(|x| &x.book == book).map(|x| x.href.as_str()).collect();
                let misses = &mut p.partial_misses;
                p.cards.retain(|c| {
                    if &c.book != book || c.dirty || c.href.is_empty() || read.contains(c.href.as_str()) {
                        return true;
                    }
                    let missed = misses.entry(c.href.clone()).or_insert(0);
                    *missed = missed.saturating_add(1);
                    *missed < mailcraft_model::pim::PARTIAL_MISSES
                });
            }
            p.books = list;
            // //// Neoffice — and whose cards may not be deleted, as this read says.
            p.no_delete_books.clone_from(&sync.no_delete_books);
        }
        // //// Neoffice — the misses of what this sync brought, or of what is gone, are let go of.
        let brought: std::collections::HashSet<&str> =
            sync.events.iter().map(|x| x.href.as_str()).chain(sync.cards.iter().map(|x| x.href.as_str())).collect();
        let alive: std::collections::HashSet<String> =
            p.events.iter().map(|e| e.href.clone()).chain(p.cards.iter().map(|c| c.href.clone())).collect();
        p.partial_misses.retain(|href, _| !brought.contains(href.as_str()) && alive.contains(href));
        // //// Neoffice — and the copies as read of what has no change left to send (Pim::originals).
        let waiting: std::collections::HashSet<&str> = p.events.iter().filter(|e| e.dirty).map(|e| e.href.as_str()).collect();
        p.originals.retain(|href, _| waiting.contains(href.as_str()));
        p.ensure_defaults();
        for e in sync.errors.iter().take(3) {
            self.notice(format!("calendars and contacts: {e}"), true);
        }
        // //// Neoffice — see `undone` above.
        for name in undone.iter().take(3) {
            self.notice(format!("{name}: its address book can't be changed any more, so the change made here was undone"), true);
        }
        self.save_pim(account);
    }

    // ----- people -----

    /// Everyone in the address books matching a search, by name.
    pub fn people(&self, query: &str) -> Vec<Person> {
        let mut out: Vec<Person> = self
            .pim
            .iter()
            .flat_map(|(a, p)| {
                p.cards.iter().filter(|c| c.card.matches(query)).map(|c| Person { account: a.clone(), book: c.book.clone(), card: c.card.clone() })
            })
            .collect();
        out.sort_by_key(|p| p.card.display().to_lowercase());
        out
    }

    pub fn find_person(&self, uid: &str) -> Option<Person> {
        self.pim.iter().find_map(|(a, p)| {
            p.cards.iter().find(|c| c.card.uid == uid).map(|c| Person { account: a.clone(), book: c.book.clone(), card: c.card.clone() })
        })
    }

    /// Add someone; returns their uid.
    pub fn create_card(&mut self, account: Option<&str>, mut card: Card) -> CmdResult {
        let account = self.pim_account(account)?;
        check_card(&card)?;
        if card.uid.trim().is_empty() {
            card.uid = self.new_uid();
        }
        // //// Neoffice — where it goes: the first book this account's new contacts can go to, and where the book here
        // isn't kept, else another account's (new_card_target); unchanged where it is.
        let (account, book) = self.new_card_target(Some(&account)).ok_or_else(|| CmdError::Disabled("no address book to add to".into()))?;
        let p = self.pim_mut(&account);
        let uid = card.uid.clone();
        p.cards.retain(|c| c.card.uid != uid);
        p.cards.push(StoredCard { book: book.clone(), href: String::new(), etag: String::new(), card, dirty: book != LOCAL });
        self.after_pim_change(&account, book != LOCAL);
        Ok(serde_json::json!({"uid": uid, "account": account}))
    }

    /// Whether a person is in an address book that can't be changed here (an organisation's
    /// shared contacts).
    pub fn person_read_only(&self, uid: &str) -> bool {
        self.pim.values().any(|p| p.cards.iter().any(|c| c.card.uid == uid && p.books.iter().any(|b| b.id == c.book && b.read_only)))
    }

    pub fn update_card(&mut self, uid: &str, change: impl FnOnce(&mut Card)) -> CmdResult {
        let account = self.find_person(uid).map(|p| p.account).ok_or_else(|| CmdError::Params(format!("no contact `{uid}`")))?;
        if self.person_read_only(uid) {
            return Err(CmdError::Disabled("this contact is in a shared address book that can't be changed here".into()));
        }
        let p = self.pim_mut(&account);
        let Some(c) = p.cards.iter_mut().find(|c| c.card.uid == uid) else { return Err(CmdError::Params(format!("no contact `{uid}`"))) };
        let mut card = c.card.clone();
        change(&mut card);
        check_card(&card)?;
        c.card = card;
        let remote = c.book != LOCAL;
        c.dirty |= remote;
        self.after_pim_change(&account, remote);
        Ok(serde_json::json!({"uid": uid}))
    }

    pub fn delete_card(&mut self, uid: &str) -> CmdResult {
        let account = self.find_person(uid).map(|p| p.account).ok_or_else(|| CmdError::Params(format!("no contact `{uid}`")))?;
        if self.person_read_only(uid) {
            return Err(CmdError::Disabled("this contact is in a shared address book that can't be changed here".into()));
        }
        let p = self.pim_mut(&account);
        // //// Neoffice — nor out of a book whose cards the person may not delete (a JMAP book's rights: Pim::no_delete_books).
        if p.cards.iter().any(|c| c.card.uid == uid && p.no_delete_books.contains(&c.book)) {
            return Err(CmdError::Disabled("your rights do not let you delete this contact".into()));
        }
        let mut remote = false;
        for c in p.cards.iter().filter(|c| c.card.uid == uid && !c.href.is_empty()) {
            remote = true;
            p.deleted.push(Tombstone { href: c.href.clone(), etag: c.etag.clone(), ..Default::default() });
        }
        p.cards.retain(|c| c.card.uid != uid);
        self.after_pim_change(&account, remote);
        Ok(serde_json::json!({"deleted": uid}))
    }
}

/// A note with the account it's in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoteRef {
    pub account: String,
    pub note: mailcraft_model::pim::Note,
}

impl Session {
    // ----- other people's calendars -----

    /// Open someone else's calendar from an Exchange account (Outlook's Open Calendar › From
    /// Address Book): read with the account's own, read-only, once the server shows it.
    pub fn open_shared_calendar(&mut self, account: Option<&str>, email: &str) -> CmdResult {
        let account = match account {
            Some(a) => self.pim_account(Some(a))?,
            None => self
                .accounts
                .iter()
                .find(|a| a.config.graph.is_some() || a.config.ews.is_some())
                .map(|a| a.config.id.clone())
                .ok_or_else(|| CmdError::Disabled("other people's calendars open from an Exchange account".into()))?,
        };
        if !self.account(&account).is_some_and(|a| a.config.graph.is_some() || a.config.ews.is_some()) {
            return Err(CmdError::Params("other people's calendars open from an Exchange account".into()));
        }
        let email = email.trim().to_ascii_lowercase();
        let ok =
            mailcraft_model::addr::parse_list(&email).is_ok_and(|l| l.len() == 1 && l.first().is_some_and(|a| a.email.eq_ignore_ascii_case(&email)));
        if !ok {
            return Err(CmdError::Params(format!("not an email address: {email}")));
        }
        let mine = self.account(&account).is_some_and(|a| a.config.email.eq_ignore_ascii_case(&email));
        if mine {
            return Err(CmdError::Params("that's this account's own calendar".into()));
        }
        let p = self.pim_mut(&account);
        if !p.shared.contains(&email) {
            if p.shared.len() >= 50 {
                return Err(CmdError::Params("at most 50 other calendars can be open".into()));
            }
            p.shared.push(email.clone());
        }
        self.after_pim_change(&account, true);
        Ok(serde_json::json!({"account": account, "calendar": mailcraft_model::pim::shared_calendar_id(&email)}))
    }

    /// Close someone else's calendar: it and its events go from this computer.
    pub fn close_shared_calendar(&mut self, email: &str) -> CmdResult {
        let email = email.trim().to_ascii_lowercase();
        let id = mailcraft_model::pim::shared_calendar_id(&email);
        let account = self.pim.iter().find(|(_, p)| p.shared.contains(&email) || p.calendars.iter().any(|c| c.id == id)).map(|(a, _)| a.clone());
        let Some(account) = account else { return Err(CmdError::Params(format!("no calendar of {email} is open"))) };
        let p = self.pim_mut(&account);
        p.shared.retain(|e| *e != email);
        p.calendars.retain(|c| c.id != id);
        p.events.retain(|e| e.calendar != id);
        self.after_pim_change(&account, false);
        Ok(serde_json::json!({"closed": email}))
    }

    // ----- notes -----

    /// Every note matching a search (in its text), the most recently changed first.
    pub fn notes(&self, query: &str) -> Vec<NoteRef> {
        let q = query.trim().to_lowercase();
        let mut out: Vec<NoteRef> = self
            .pim
            .iter()
            .flat_map(|(a, p)| {
                p.notes
                    .iter()
                    .filter(|n| q.is_empty() || n.note.text.to_lowercase().contains(&q))
                    .map(|n| NoteRef { account: a.clone(), note: n.note.clone() })
            })
            .collect();
        // A note's title is its first line, compared without case.
        let title = |n: &NoteRef| n.note.text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default().to_lowercase();
        match self.prefs.notes_order {
            super::NotesOrder::Modified => out.sort_by(|a, b| b.note.modified.cmp(&a.note.modified).then_with(|| a.note.uid.cmp(&b.note.uid))),
            super::NotesOrder::Title => out.sort_by_cached_key(|n| (title(n), n.note.uid.clone())),
            super::NotesOrder::Color => out.sort_by_cached_key(|n| (n.note.color, title(n), n.note.uid.clone())),
        }
        out
    }

    fn note_account(&self, uid: &str) -> Result<String, CmdError> {
        self.pim
            .iter()
            .find(|(_, p)| p.notes.iter().any(|n| n.note.uid == uid))
            .map(|(a, _)| a.clone())
            .ok_or_else(|| CmdError::Params(format!("no note `{uid}`")))
    }

    /// Notes go to the server only where it keeps them (Exchange's Notes folder).
    fn notes_sync(&self, account: &str) -> bool {
        self.account(account).is_some_and(|a| !a.is_local() && a.config.ews.is_some())
    }

    /// A new note; returns its uid.
    pub fn create_note(&mut self, account: Option<&str>, text: &str, color: Option<u8>) -> CmdResult {
        let account = self.pim_account(account)?;
        let note = mailcraft_model::pim::Note {
            uid: self.new_uid(),
            text: cap_note(text),
            color: color.unwrap_or(mailcraft_model::pim::NOTE_YELLOW).min(mailcraft_model::pim::NOTE_COLORS - 1),
            modified: self.clock.now,
        };
        let uid = note.uid.clone();
        let remote = self.notes_sync(&account);
        let p = self.pim_mut(&account);
        if p.notes.len() >= MAX_NOTES {
            return Err(CmdError::Params(format!("an account keeps at most {MAX_NOTES} notes")));
        }
        p.notes.push(mailcraft_model::pim::StoredNote { note, dirty: remote, ..Default::default() });
        self.after_pim_change(&account, remote);
        Ok(serde_json::json!({"uid": uid, "account": account}))
    }

    /// Change a note's text or colour.
    pub fn update_note(&mut self, uid: &str, text: Option<&str>, color: Option<u8>) -> CmdResult {
        let account = self.note_account(uid)?;
        let remote = self.notes_sync(&account);
        let now = self.clock.now;
        let p = self.pim_mut(&account);
        let Some(n) = p.notes.iter_mut().find(|n| n.note.uid == uid) else { return Err(CmdError::Params(format!("no note `{uid}`"))) };
        let before = n.note.clone();
        if let Some(t) = text {
            n.note.text = cap_note(t);
        }
        if let Some(c) = color {
            n.note.color = c.min(mailcraft_model::pim::NOTE_COLORS - 1);
        }
        if n.note == before {
            return Ok(serde_json::json!({"uid": uid, "changed": false}));
        }
        n.note.modified = now;
        n.dirty |= remote;
        self.after_pim_change(&account, remote);
        Ok(serde_json::json!({"uid": uid}))
    }

    pub fn delete_note(&mut self, uid: &str) -> CmdResult {
        let account = self.note_account(uid)?;
        let p = self.pim_mut(&account);
        let mut remote = false;
        for n in p.notes.iter().filter(|n| n.note.uid == uid && !n.href.is_empty()) {
            remote = true;
            p.deleted.push(Tombstone { href: n.href.clone(), etag: n.etag.clone(), ..Default::default() });
        }
        p.notes.retain(|n| n.note.uid != uid);
        self.after_pim_change(&account, remote);
        Ok(serde_json::json!({"deleted": uid}))
    }
}

/// Most notes an account keeps, and most characters in one.
const MAX_NOTES: usize = 5_000;
const MAX_NOTE_CHARS: usize = 100_000;

fn cap_note(text: &str) -> String {
    text.chars().take(MAX_NOTE_CHARS).collect()
}

fn check_event(e: &CalEvent) -> Result<(), CmdError> {
    if e.summary.trim().is_empty() {
        return Err(CmdError::Params("give the event a title".into()));
    }
    if e.end < e.start {
        return Err(CmdError::Params("the event ends before it starts".into()));
    }
    if e.end - e.start > 366 * 86_400 {
        return Err(CmdError::Params("an event can last a year at most".into()));
    }
    Ok(())
}

fn check_card(c: &Card) -> Result<(), CmdError> {
    if c.display().trim().is_empty() {
        return Err(CmdError::Params("give the contact a name or an address".into()));
    }
    for e in &c.emails {
        if !addr::is_valid_email(e.value.trim()) {
            return Err(CmdError::Params(format!("`{}` is not an email address", e.value)));
        }
    }
    Ok(())
}

/// A local date (`YYYY-MM-DD`, all day) or date and time (`YYYY-MM-DDTHH:MM`, or with a space),
/// or Unix seconds, as (Unix seconds, all day). `offset` is the local UTC offset; all-day dates
/// come back at UTC midnight (as events keep them).
pub fn parse_local(s: &str, offset: i64) -> Option<(i64, bool)> {
    let s = s.trim();
    if let Ok(n) = s.parse::<i64>() {
        return (n.abs() < 400_000_000_000).then_some((n, false));
    }
    let num = |a: usize, b: usize| s.get(a..b).filter(|x| x.bytes().all(|c| c.is_ascii_digit())).and_then(|x| x.parse::<i64>().ok());
    let (y, m, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    if s.get(4..5) != Some("-") || s.get(7..8) != Some("-") || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let day = days_from_civil(y, u32::try_from(m).ok()?, u32::try_from(d).ok()?);
    if s.len() == 10 {
        return Some((day * 86_400, true));
    }
    if !matches!(s.get(10..11), Some("T" | " ")) {
        return None;
    }
    let (h, mi) = (num(11, 13)?, num(14, 16)?);
    if h > 23 || mi > 59 || s.get(13..14) != Some(":") {
        return None;
    }
    Some((day * 86_400 + h * 3600 + mi * 60 - offset, false))
}

// //// Neoffice — see apply_dav (a calendar read in full or in part).
/// Whether `e`, here, has a change waiting to be sent that `x`, the server's, is not to undo: the same part of the same
/// event (by its address there, or by its uid: one event in two calendars, an occurrence a server numbered anew), the
/// event itself or the same occurrence of it, or an exception for a date the event here takes out; one never stored there
/// holds back all of its uid. The server's other parts come as it has them: the other occurrences of a series a server
/// expanded (JMAP's, each an event of its own), the other exceptions of a CalDAV event kept at one address with it, which
/// its next write keeps (upstream held back every part of the uid, and lost them there).
fn waiting_change(e: &StoredEvent, x: &StoredEvent) -> bool {
    if !e.dirty {
        return false;
    }
    if e.href.is_empty() {
        return e.event.uid == x.event.uid;
    }
    let taken_out = e.event.recurrence_id.is_none() && x.event.recurrence_id.is_some_and(|at| e.event.exdates.contains(&at));
    (e.href == x.href || e.event.uid == x.event.uid) && (e.event.recurrence_id == x.event.recurrence_id || taken_out)
}
