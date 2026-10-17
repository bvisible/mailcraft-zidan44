// //// Neoffice — added file (no upstream equivalent): JMAP for Calendars (draft-ietf-jmap-calendars), read and written by
// the same client as the mail.
//
// A server that offers `urn:ietf:params:jmap:calendars` (Neoffice's mail facade does, for the person's calendars) gives
// its calendars and the events of a window around today, page after page, each occurrence of a repeating event apart,
// with its start and end in UTC (`utcStart`, `utcEnd`, which the server works out): MailCraft has no time-zone
// database, and the browser build has no CalDAV. They come back as a [`DavSync`], which the engine merges as a CalDAV
// sync's, so the calendar, the day's agenda and the meeting answers read them unchanged.
//
// What was made, changed or deleted here in a calendar the person may write (`myRights.mayWriteAll` or `mayWriteOwn`)
// goes back first, with `CalendarEvent/set`, as many at a time as `maxObjectsInSet` allows and at most 50 (Neoffice's
// facade answers `rateLimit` beyond): a new event whole, with its own uid; a change as a patch of what the person changed
// since the copy it started from (as read, `Pim::originals`), made to the server's copy read just before, so that what
// the person didn't change stays as the server has it, what someone else changed meanwhile among it; a map (the
// locations, the guests) whole, the person's change made to the server's (Neoffice's facade takes no patch of a part of
// a property); a deletion as a destroy. What the server could not take for the time being (`rateLimit`, `serverFail`…)
// stays to be sent at the next sync; what it refused goes, said. A time made or changed here is written in the browser's
// zone (`start` and `timeZone`, the browser's time-zone rules giving the local time; its length as days on the wall clock
// there, then the time that passes), an occurrence's in its own zone (the server keeps a series' zone), a day-long event
// as its days. An occurrence the server expanded is changed or deleted by its own id (« this occurrence ») or by its
// series' base event (« the whole series »). The server sends the meeting mail (`sendSchedulingMessages`) where the person
// organizes an event with guests, as MailCraft sends it itself elsewhere (the person's answer to another's meeting is
// mailed by MailCraft, as elsewhere); the organizer, the draft state
// and a series' overrides are never sent (the server keeps them), nor the keywords (MailCraft's categories) to a server
// that doesn't keep them (Neoffice's).

use std::collections::{HashMap, HashSet};

use mailcraft_model::dates::{civil_from_days, days_from_civil};
use mailcraft_model::ical::{Attendee, CalEvent, duration};
use mailcraft_model::pim::{
    CalendarInfo, DavSync, LOCAL, Pim, SERIES_CHANGE_LINE, SERIES_CHANGE_RULE, SERIES_LINE, SERIES_RULE_LINE, StoredEvent, series_line,
};
use serde_json::{Map, Value, json};

use crate::contacts::{FAILED_FOR_NOW, Refusal, refusal_words};
use crate::{CORE, Client, Error, Http, Result, list, query_page, utc_date};

/// The capability, and the type of the account that holds the calendars.
pub const CALENDARS: &str = "urn:ietf:params:jmap:calendars";
/// The window read: this many days back, and ahead.
const DAYS_BACK: i64 = 90;
const DAYS_AHEAD: i64 = 366;
/// The most occurrences read in one sync.
const MAX_EVENTS: usize = 5000;
/// The most events one `CalendarEvent/set` changes (Neoffice's facade answers `rateLimit` beyond).
const MAX_SET: usize = 50;
/// What an event is read with: what [`CalEvent`] shows, and what writing it back needs (its zone, its series, who
/// organizes it, whether it is a draft); its keywords too, where the server keeps them ([`Writing::keywords`]).
const PROPERTIES: [&str; 19] = [
    "id",
    "uid",
    "calendarIds",
    "title",
    "description",
    "locations",
    "start",
    "timeZone",
    "duration",
    "showWithoutTime",
    "utcStart",
    "utcEnd",
    "recurrenceId",
    "baseEventId",
    "recurrenceRule",
    "status",
    "participants",
    "organizerCalendarAddress",
    "isDraft",
];
/// An event's address on this account (its id on the server), apart from those of CalDAV.
pub const HREF_PREFIX: &str = "jmap:";

/// How the calendars are read and written: who the person is, whether the server keeps keywords, and the browser's zone
/// and its rules.
pub struct Writing<'a> {
    /// The person's addresses: who organizes an event, and who else takes part.
    pub me: Vec<String>,
    /// The server keeps the events' keywords (MailCraft's categories): not Neoffice's, which refuses a set that has them.
    pub keywords: bool,
    /// The browser's zone (its IANA name), which new and changed times are written in.
    pub zone: String,
    /// The local time (seconds, as if UTC) of an instant in a zone, the browser's for `None`: the browser works it out
    /// with its time-zone rules. `None` for a zone it doesn't know.
    pub local: &'a dyn Fn(Option<&str>, i64) -> Option<i64>,
}

impl Writing<'_> {
    fn properties(&self) -> Vec<&'static str> {
        let mut p = PROPERTIES.to_vec();
        if self.keywords {
            p.push("keywords");
        }
        p
    }

    fn mine(&self, email: &str) -> bool {
        self.me.iter().any(|m| m.trim().eq_ignore_ascii_case(email.trim()))
    }
}

impl<H: Http> Client<H> {
    /// Whether the session offers calendars.
    pub fn offers_calendars(&self) -> bool {
        self.calendars.is_some()
    }

    /// What was made, changed or deleted here first (`pim`: the account's calendars as the engine keeps them), then the
    /// calendars and the occurrences of their events in the window around `now`, as a sync to merge (`None` when the
    /// session offers none). What the server refused is said once in the sync's errors and handed over as refused (the
    /// change goes); what could not be sent stays to be sent, said where a call failed or the server failed for the time
    /// being, not where it asked to wait (`rateLimit`). When the calendars can't be read, what was sent is handed over all the same, the calendars
    /// unread (`None`). A token the server refuses ([`Error::Auth`]) stops the sync, for the mailbox to sign in again:
    /// what it sent before goes again then, which the server recognises (an event's uid, a deletion done, a change made).
    pub async fn sync_calendars(&self, pim: &Pim, w: &Writing<'_>, now: i64) -> Result<Option<DavSync>> {
        let Some(account) = self.calendars.as_deref() else { return Ok(None) };
        let pushed = self.push_events(account, pim, w).await?;
        let mut sync = match self.read_calendars(account, w, now).await {
            Ok(sync) => sync,
            Err(e @ Error::Auth(_)) => return Err(e),
            Err(e) => {
                log::warn!("the calendars were not read: {e}");
                DavSync { caldav_home: Some("jmap".into()), checked: now, ..Default::default() }
            }
        };
        sync.pushed_events = pushed.events;
        sync.refused_events = pushed.refused;
        sync.deleted = pushed.deleted;
        sync.refused_deletions = pushed.refused_deletions;
        sync.errors = pushed.errors;
        Ok(Some(sync))
    }

    /// What was made, changed or deleted here, sent, without reading the calendars again: a sync of what is left to send
    /// (the calendars stay as the engine has them, `None`).
    pub async fn send_calendars(&self, pim: &Pim, w: &Writing<'_>, now: i64) -> Result<Option<DavSync>> {
        let Some(account) = self.calendars.as_deref() else { return Ok(None) };
        let pushed = self.push_events(account, pim, w).await?;
        Ok(Some(DavSync {
            caldav_home: Some("jmap".into()),
            checked: now,
            pushed_events: pushed.events,
            refused_events: pushed.refused,
            deleted: pushed.deleted,
            refused_deletions: pushed.refused_deletions,
            errors: pushed.errors,
            ..Default::default()
        }))
    }

    /// The calendars and the occurrences of their events in the window around `now`.
    async fn read_calendars(&self, account: &str, w: &Writing<'_>, now: i64) -> Result<DavSync> {
        let (from, to) = window(now, self.calendars_span);
        let window = json!({"after": utc_string(from), "before": utc_string(to)});
        let query = json!({"accountId": account, "filter": window, "expandRecurrences": true});
        let r = self
            .call(
                &[CORE, CALENDARS],
                vec![("Calendar/get", json!({"accountId": account, "ids": null})), ("CalendarEvent/query", query_page(&query, 0, MAX_EVENTS))],
            )
            .await?;
        let calendars = list(r.first(), "list").to_vec();
        let first = r.get(1).cloned().unwrap_or(Value::Null);
        let (ids, whole) = self.query_rest(&[CORE, CALENDARS], "CalendarEvent/query", &query, first, MAX_EVENTS).await?;
        let mut events = Vec::with_capacity(ids.len());
        for chunk in ids.chunks(self.max_get.max(1)) {
            let got = self
                .call(&[CORE, CALENDARS], vec![("CalendarEvent/get", json!({"accountId": account, "ids": chunk, "properties": w.properties()}))])
                .await?;
            events.extend(list(got.first(), "list").iter().cloned());
        }
        let mut sync = calendars_sync(&calendars, &events, now);
        sync.window = Some((from, to));
        if !whole {
            // One query covers every calendar: none was read whole, so each is handed over as read in part, and the
            // engine adds and updates the occurrences read without losing those past the cap.
            log::warn!("the calendars were not read whole (more than {MAX_EVENTS} occurrences, or too many pages): merged without removing any");
            sync.partial_calendars = std::mem::take(&mut sync.synced_calendars);
        }
        Ok(sync)
    }

    /// The server's copies of these events, by id, and the ids it no longer has.
    async fn events_by_id(&self, account: &str, ids: &[String], w: &Writing<'_>) -> Result<(HashMap<String, Value>, HashSet<String>)> {
        let (mut found, mut missing) = (HashMap::new(), HashSet::new());
        for chunk in ids.chunks(self.max_get.max(1)) {
            let r = self
                .call(&[CORE, CALENDARS], vec![("CalendarEvent/get", json!({"accountId": account, "ids": chunk, "properties": w.properties()}))])
                .await?;
            for v in list(r.first(), "list") {
                if let Some(id) = v.get("id").and_then(Value::as_str) {
                    found.insert(id.to_string(), v.clone());
                }
            }
            missing.extend(list(r.first(), "notFound").iter().filter_map(Value::as_str).map(str::to_string));
        }
        Ok((found, missing))
    }

    /// Sends what was made, changed or deleted here in the server's calendars: the events it stored (with their ids), the
    /// changes it refused (said once: the change goes), the deletions it is done with (made, already made, or refused and
    /// said: the event comes back at the next read), and why the others were not sent (a call that failed: they stay to
    /// be sent; not when the server asked to wait).
    async fn push_events(&self, account: &str, pim: &Pim, w: &Writing<'_>) -> Result<Pushed> {
        let mut out = Pushed::default();
        if pim.caldav_home.as_deref() != Some("jmap") {
            return Ok(out);
        }
        let changes = changes(pim);
        if changes.is_empty() {
            return Ok(out);
        }
        // The server's copies of what is changed or deleted (and, for a whole series, of the occurrence changed).
        let mut ids: Vec<String> = Vec::new();
        for c in &changes {
            for id in c.reads() {
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
        }
        let (server, missing, unread) = match self.events_by_id(account, &ids, w).await {
            Ok((server, missing)) => (server, missing, None),
            Err(e @ Error::Auth(_)) => return Err(e),
            Err(e) => (HashMap::new(), HashSet::new(), Some(e)),
        };
        let mut ops: Vec<Op> = Vec::new();
        for c in &changes {
            match c {
                Change::Create(e) => {
                    let (body, schedule) = create_json(e, w);
                    ops.push(Op { schedule, kind: Kind::Create(e, body) });
                }
                // Not read: tried again at the next sync (said, unless the server asked to wait).
                Change::Update { e, .. } if unread.is_some() => {
                    if let Some(err) = unread.as_ref().filter(|err| !rate_limited(err)) {
                        out.errors.push(format!("an event was not saved: {} — {err}", title(e)));
                    }
                }
                Change::Destroy { .. } if unread.is_some() => {
                    if let Some(err) = unread.as_ref().filter(|err| !rate_limited(err)) {
                        out.errors.push(format!("an event was not deleted: {err}"));
                    }
                }
                Change::Update { e, id, how, was } => {
                    let read = |id: &str| server.get(id);
                    let target = read(id);
                    let occurrence = match how {
                        How::Series { occurrence, .. } => read(occurrence),
                        _ => target,
                    };
                    let (Some(s), Some(o)) = (target, occurrence) else {
                        // No longer there (deleted on the server meanwhile): the change can't be made, and goes.
                        out.errors.push(format!("an event was not saved: {} — not found", title(e)));
                        out.refused.push((*e).clone());
                        continue;
                    };
                    // What the person changed: since the copy the change started from; without it (a change made before
                    // it was kept), since the server's copy (of the occurrence, for a whole series).
                    let Some(was) = was.cloned().or_else(|| event_of(o)) else {
                        out.errors.push(format!("an event was not saved: {} — not found", title(e)));
                        out.refused.push((*e).clone());
                        continue;
                    };
                    let (patch, schedule) = match how {
                        How::Event => patch(&e.event, &was, s, s, w, Time::Browser, Some((e.event.rrule.as_deref(), was.rrule.as_deref()))),
                        How::Occurrence => patch(&e.event, &was, s, s, w, Time::Own, None),
                        How::Series { rule, .. } => {
                            let rule = rule.then(|| (series_line(&e.event, SERIES_RULE_LINE), series_line(&was, SERIES_RULE_LINE)));
                            patch(&e.event, &was, s, o, w, Time::Series, rule)
                        }
                    };
                    if patch.is_empty() {
                        // Nothing that differs: done.
                        out.events.push(StoredEvent { dirty: false, ..(*e).clone() });
                        continue;
                    }
                    ops.push(Op { schedule, kind: Kind::Update(e, id.clone(), patch) });
                }
                Change::Destroy { href, id } => {
                    match server.get(id) {
                        Some(s) => ops.push(Op { schedule: scheduled(s, w), kind: Kind::Destroy(href, id.clone()) }),
                        // Already gone there: done.
                        None if missing.contains(id) => out.deleted.push((*href).to_string()),
                        None => {}
                    }
                }
            }
        }
        let per_call = self.max_set.clamp(1, MAX_SET);
        for schedule in [false, true] {
            let these: Vec<&Op> = ops.iter().filter(|o| o.schedule == schedule).collect();
            for chunk in these.chunks(per_call) {
                let (mut create, mut update, mut destroy) = (Map::new(), Map::new(), Vec::new());
                for (n, op) in chunk.iter().enumerate() {
                    match &op.kind {
                        Kind::Create(_, body) => {
                            create.insert(format!("k{n}"), body.clone());
                        }
                        Kind::Update(_, id, patch) => {
                            update.insert(id.clone(), Value::Object(patch.clone()));
                        }
                        Kind::Destroy(_, id) => destroy.push(id.clone()),
                    }
                }
                let mut args = json!({"accountId": account, "create": create, "update": update, "destroy": destroy});
                if schedule {
                    args["sendSchedulingMessages"] = json!(true);
                }
                let answer = match self.call(&[CORE, CALENDARS], vec![("CalendarEvent/set", args)]).await {
                    Ok(r) => r.into_iter().next().unwrap_or(Value::Null),
                    Err(e @ Error::Auth(_)) => return Err(e),
                    // Tried again at the next sync, nothing said.
                    Err(e) if rate_limited(&e) => continue,
                    Err(e) => {
                        for op in chunk {
                            out.errors.push(match &op.kind {
                                Kind::Create(s, _) | Kind::Update(s, _, _) => format!("an event was not saved: {} — {e}", title(s)),
                                Kind::Destroy(..) => format!("an event was not deleted: {e}"),
                            });
                        }
                        continue;
                    }
                };
                for (n, op) in chunk.iter().enumerate() {
                    match &op.kind {
                        Kind::Create(e, _) => {
                            let key = format!("k{n}");
                            let refusal = answer.get("notCreated").and_then(|m| m.get(&key));
                            // Made before (an answer lost on the way, a sync run twice): the server says which it is
                            // (`alreadyExists` and its `existingId`, RFC 8620 §5.3), and that one it is.
                            let existing =
                                refusal.filter(|r| r.get("type").and_then(Value::as_str) == Some("alreadyExists")).and_then(|r| r.get("existingId"));
                            let id = answer.get("created").and_then(|m| m.get(&key)).and_then(|v| v.get("id")).or(existing).and_then(Value::as_str);
                            match id {
                                Some(id) => out.events.push(StoredEvent { href: format!("{HREF_PREFIX}{id}"), dirty: false, ..(*e).clone() }),
                                None => out.refuse(e, refusal),
                            }
                        }
                        Kind::Update(e, id, _) => {
                            if answer.get("updated").and_then(|u| u.get(id)).is_some() {
                                out.events.push(StoredEvent { dirty: false, ..(*e).clone() });
                            } else {
                                out.refuse(e, answer.get("notUpdated").and_then(|m| m.get(id)));
                            }
                        }
                        Kind::Destroy(href, id) => {
                            let done = answer.get("destroyed").and_then(Value::as_array).is_some_and(|d| d.iter().any(|x| x.as_str() == Some(id)));
                            let refusal = answer.get("notDestroyed").and_then(|m| m.get(id));
                            let gone = refusal.and_then(|r| r.get("type")).and_then(Value::as_str) == Some("notFound");
                            match refusal.map(Refusal::of) {
                                // Already gone there: done.
                                _ if done || gone => out.deleted.push((*href).to_string()),
                                // Tried again at the next sync: nothing said where the server asked to wait, said where it
                                // failed for the time being.
                                Some(Refusal::Wait) => {}
                                Some(Refusal::Failed) => out.errors.push(format!("an event was not deleted: {FAILED_FOR_NOW}")),
                                // Said once; the event is the server's still, and comes back at the next read.
                                Some(Refusal::Refused) => {
                                    out.errors.push(format!("an event was not deleted: {}", refusal_words(refusal)));
                                    out.deleted.push((*href).to_string());
                                    out.refused_deletions.push((*href).to_string());
                                }
                                None => {}
                            }
                        }
                    }
                }
            }
        }
        Ok(out)
    }
}

/// What a push did: the events stored, the changes refused (said), the deletions done with, what was said.
#[derive(Default)]
struct Pushed {
    events: Vec<StoredEvent>,
    refused: Vec<StoredEvent>,
    deleted: Vec<String>,
    /// The deletions the server refused for good (in `deleted` too).
    refused_deletions: Vec<String>,
    errors: Vec<String>,
}

impl Pushed {
    /// The server didn't make `e`'s change (`refusal`): refused, said once, and the change goes; it asked to wait
    /// (`rateLimit`), or didn't answer for it, nothing said; it failed for the time being (`serverFail`…), said: these two
    /// stay, sent again at the next sync.
    fn refuse(&mut self, e: &StoredEvent, refusal: Option<&Value>) {
        let Some(r) = refusal else { return };
        match Refusal::of(r) {
            Refusal::Wait => {}
            Refusal::Failed => self.errors.push(format!("an event was not saved: {} — {FAILED_FOR_NOW}", title(e))),
            Refusal::Refused => {
                self.errors.push(format!("an event was not saved: {} — {}", title(e), refusal_words(refusal)));
                self.refused.push(e.clone());
            }
        }
    }
}

fn title(e: &StoredEvent) -> &str {
    let t = e.event.summary.trim();
    if t.is_empty() { "(no title)" } else { t }
}

/// Whether the server asked to wait (`rateLimit`, or too many requests): tried again at the next sync, nothing said.
fn rate_limited(e: &Error) -> bool {
    match e {
        Error::No(m) => m.starts_with("rateLimit"),
        Error::Closed(m) => m == "HTTP 429",
        _ => false,
    }
}

/// One change made here to send.
enum Change<'a> {
    /// Made here: no address there yet.
    Create(&'a StoredEvent),
    /// Changed here: an event or one occurrence (`id`, its own), or a whole series (`id`, its base event's); `was`, the
    /// copy the change started from, as read (Pim::originals), where it is kept.
    Update { e: &'a StoredEvent, id: String, how: How<'a>, was: Option<&'a CalEvent> },
    /// Deleted here: its address there (`jmap:<id>`).
    Destroy { href: &'a str, id: String },
}

/// What an update changes.
enum How<'a> {
    /// An event (not an occurrence of a series the server expanded): its time in the browser's zone, its rule too.
    Event,
    /// One occurrence: its time in its own zone.
    Occurrence,
    /// The whole series, from one of its occurrences (`occurrence`, its id); its rule too where `rule`.
    Series { occurrence: &'a str, rule: bool },
}

impl Change<'_> {
    /// The ids whose server copies the change needs.
    fn reads(&self) -> Vec<String> {
        match self {
            Change::Create(_) => Vec::new(),
            Change::Update { id, how: How::Series { occurrence, .. }, .. } => vec![id.clone(), (*occurrence).to_string()],
            Change::Update { id, .. } | Change::Destroy { id, .. } => vec![id.clone()],
        }
    }
}

/// What was made, changed or deleted here in the server's calendars: new events (no address yet) and the server's own
/// (`jmap:<id>`), and their deletions (an event's tombstone names its calendar, a card's doesn't). A whole series is
/// changed once, from the first of its occurrences that says so.
fn changes(pim: &Pim) -> Vec<Change<'_>> {
    let mut out: Vec<Change> = Vec::new();
    let mut series: HashSet<&str> = HashSet::new();
    for e in pim.events.iter().filter(|e| e.dirty && e.calendar != LOCAL) {
        if e.href.is_empty() {
            out.push(Change::Create(e));
            continue;
        }
        let Some(own) = e.href.strip_prefix(HREF_PREFIX) else { continue };
        let base = series_line(&e.event, SERIES_LINE).and_then(|b| b.strip_prefix(HREF_PREFIX));
        let how = match (series_line(&e.event, SERIES_CHANGE_LINE), base) {
            (Some(change), Some(base)) => {
                if !series.insert(base) {
                    continue;
                }
                let how = How::Series { occurrence: own, rule: change == SERIES_CHANGE_RULE };
                out.push(Change::Update { e, id: base.to_string(), how, was: pim.originals.get(&e.href) });
                continue;
            }
            (None, Some(_)) => How::Occurrence,
            _ => How::Event,
        };
        out.push(Change::Update { e, id: own.to_string(), how, was: pim.originals.get(&e.href) });
    }
    for t in pim.deleted.iter().filter(|t| !t.calendar.is_empty()) {
        if let Some(id) = t.href.strip_prefix(HREF_PREFIX) {
            out.push(Change::Destroy { href: &t.href, id: id.to_string() });
        }
    }
    out.truncate(MAX_EVENTS);
    out
}

/// Whether something made, changed or deleted here waits to be sent to the server's calendars (`changes`).
pub(crate) fn waiting(pim: &Pim) -> bool {
    pim.caldav_home.as_deref() == Some("jmap") && !changes(pim).is_empty()
}

/// One `/set` operation, and whether the server is to send the meeting mail for it.
struct Op<'a> {
    schedule: bool,
    kind: Kind<'a>,
}

enum Kind<'a> {
    Create(&'a StoredEvent, Value),
    Update(&'a StoredEvent, String, Map<String, Value>),
    Destroy(&'a str, String),
}

/// How a changed time is written.
#[derive(Clone, Copy)]
enum Time {
    /// In the browser's zone (an event made or changed here).
    Browser,
    /// In the event's own zone (an occurrence: the server keeps a series' zone).
    Own,
    /// A whole series, moved as its occurrence was.
    Series,
}

/// A new event, whole: its uid (a server that stored it before, its answer lost, knows it again), its calendar, what it
/// says, its time in the browser's zone, its rule, its keywords where the server keeps them, and its guests where the
/// person organizes it; and whether the server is to invite them.
fn create_json(stored: &StoredEvent, w: &Writing) -> (Value, bool) {
    let e = &stored.event;
    let mut v = Map::new();
    v.insert("@type".into(), json!("Event"));
    v.insert("uid".into(), json!(e.uid));
    v.insert("calendarIds".into(), json!({ stored.calendar.clone(): true }));
    v.insert("title".into(), json!(e.summary.trim()));
    if !e.description.trim().is_empty() {
        v.insert("description".into(), json!(e.description.trim()));
    }
    if !e.location.trim().is_empty() {
        v.insert("locations".into(), json!({"l1": {"@type": "Location", "name": e.location.trim()}}));
    }
    browser_time(e, w, &mut v);
    if let Some(rule) = e.rrule.as_deref().and_then(|r| rule_json(r, &|t| (w.local)(None, t))) {
        v.insert("recurrenceRule".into(), rule);
    }
    if w.keywords && !e.categories.is_empty() {
        v.insert("keywords".into(), keywords(&e.categories));
    }
    // The guests of an event the person organizes (one taken from an invitation keeps its organizer's: none are sent).
    let organized = e.organizer.as_ref().is_none_or(|o| w.mine(&o.email));
    let guests: Vec<&Attendee> = e.attendees.iter().filter(|a| !w.mine(&a.email)).collect();
    let schedule = organized && !guests.is_empty();
    if schedule {
        let list: Map<String, Value> = guests.iter().enumerate().map(|(n, a)| (format!("p{}", n + 1), participant_json(a))).collect();
        v.insert("participants".into(), Value::Object(list));
    }
    (Value::Object(v), schedule)
}

/// What the person changed of `e` since `was` (the copy the change started from, as read), as a patch of the server's
/// copy `s` as it is now (`o`: the occurrence changed, for a whole series; else `s` itself): of the properties MailCraft
/// edits, those the person changed, as they now are, the others as the server has them (what someone else changed
/// meanwhile, in the desk or on another device, among them); a map (the locations, the guests) whole, with the person's
/// change made to the server's, as the server takes no patch of a part of a property (Neoffice's facade refuses
/// `locations/l1/name`); and whether the server is to send the meeting mail. A changed time is written as `time` says;
/// the rule where `rule` gives it with the one the change started from (`None`: no repeats), and it changed.
fn patch(
    e: &CalEvent,
    was: &CalEvent,
    s: &Value,
    o: &Value,
    w: &Writing,
    time: Time,
    rule: Option<(Option<&str>, Option<&str>)>,
) -> (Map<String, Value>, bool) {
    let mut p = Map::new();
    let now = event_of(s).unwrap_or_default();
    // Changed by the person, and not so on the server already.
    let changed = |ours: &str, before: &str, theirs: &str| ours.trim() != before.trim() && ours.trim() != theirs.trim();
    if changed(&e.summary, &was.summary, &now.summary) {
        p.insert("title".into(), json!(e.summary.trim()));
    }
    if changed(&e.description, &was.description, &now.description) {
        p.insert("description".into(), json!(e.description.trim()));
    }
    // The location MailCraft shows is the first named one: the person's change of it is made to the server's locations,
    // the others staying.
    let location = e.location.trim();
    if location != was.location.trim() {
        let server = s.get("locations").and_then(Value::as_object).cloned().unwrap_or_default();
        let mut map = server.clone();
        let named = |l: &Value| l.get("name").and_then(Value::as_str).is_some_and(|n| !n.trim().is_empty());
        match map.iter().find(|(_, l)| named(l)).map(|(k, _)| k.clone()) {
            Some(key) if location.is_empty() => {
                map.remove(&key);
            }
            Some(key) => {
                if let Some(l) = map.get_mut(&key).and_then(Value::as_object_mut) {
                    l.insert("name".into(), json!(location));
                }
            }
            None if !location.is_empty() => {
                map.insert(free_key(&map, "l"), json!({"@type": "Location", "name": location}));
            }
            None => {}
        }
        if map != server {
            p.insert("locations".into(), if map.is_empty() { Value::Null } else { Value::Object(map) });
        }
    }
    if w.keywords {
        // The categories the person added and took out, made to the server's.
        let out: Vec<&String> = was.categories.iter().filter(|c| !e.categories.contains(c)).collect();
        let mut kept: Vec<String> = now.categories.iter().filter(|c| !out.contains(c)).cloned().collect();
        for c in e.categories.iter().filter(|c| !was.categories.contains(c)) {
            if !kept.contains(c) {
                kept.push(c.clone());
            }
        }
        let (mut ours, mut theirs) = (kept.clone(), now.categories.clone());
        ours.sort();
        theirs.sort();
        if ours != theirs {
            p.insert("keywords".into(), if kept.is_empty() { Value::Null } else { keywords(&kept) });
        }
    }
    // The time, where the person moved it (to the minute: the form shows minutes) and it is not there already.
    let then = event_of(o).unwrap_or_default();
    let moved = |from: &CalEvent| from.all_day != e.all_day || (from.start - e.start).abs() >= 60 || (from.end - e.end).abs() >= 60;
    if moved(was) && moved(&then) {
        match time {
            Time::Browser => browser_time(e, w, &mut p),
            Time::Own => own_time(e, s, w, &mut p),
            Time::Series => series_time(e, was, s, o, w, &mut p),
        }
    }
    if let Some((ours, before)) = rule {
        let zone = s.get("timeZone").and_then(Value::as_str);
        let json_of = |r: Option<&str>| r.and_then(|r| rule_json(r, &|t| (w.local)(zone, t)));
        let text_of = |r: Option<&str>| json_of(r).as_ref().and_then(rule_text);
        let theirs = s.get("recurrenceRule").filter(|r| r.is_object()).and_then(rule_text);
        let wanted = text_of(ours);
        if wanted != text_of(before) && wanted != theirs {
            p.insert("recurrenceRule".into(), json_of(ours).unwrap_or(Value::Null));
        }
    }
    let schedule = participants(e, was, s, w, &mut p);
    (p, schedule)
}

/// A key of `map` no entry has: `l1`, `l2`… (`prefix` `l`), one of its length and one more.
fn free_key(map: &Map<String, Value>, prefix: &str) -> String {
    (1..=map.len() + 1).map(|n| format!("{prefix}{n}")).find(|k| !map.contains_key(k)).unwrap_or_default()
}

/// A time in the browser's zone: `start` local there with `timeZone`, and `duration`; a day-long event's days.
fn browser_time(e: &CalEvent, w: &Writing, p: &mut Map<String, Value>) {
    if e.all_day {
        all_day_time(e, p);
        p.insert("timeZone".into(), Value::Null);
        return;
    }
    // The browser's zone, unless the browser couldn't say (UTC then).
    let (start, zone, duration) = match (w.local)(None, e.start) {
        Some(local) if !w.zone.trim().is_empty() => (local, w.zone.trim().to_string(), wall_duration(e.start, e.end, &|t| (w.local)(None, t))),
        _ => (e.start, "Etc/UTC".to_string(), wall_duration(e.start, e.end, &|t| Some(t))),
    };
    p.insert("start".into(), json!(local_string(start)));
    p.insert("timeZone".into(), json!(zone));
    p.insert("duration".into(), json!(duration));
    p.insert("showWithoutTime".into(), json!(false));
}

/// A day-long event's days: from its first day, at midnight, for as many days.
fn all_day_time(e: &CalEvent, p: &mut Map<String, Value>) {
    p.insert("start".into(), json!(local_string(e.start - e.start.rem_euclid(86_400))));
    p.insert("duration".into(), json!(format!("P{}D", ((e.end - e.start) / 86_400).max(1))));
    p.insert("showWithoutTime".into(), json!(true));
}

/// An occurrence's time in its own zone `s` has (the server refuses another zone for one occurrence of a series).
fn own_time(e: &CalEvent, s: &Value, w: &Writing, p: &mut Map<String, Value>) {
    let was_all_day = s.get("showWithoutTime").and_then(Value::as_bool).unwrap_or(false);
    if e.all_day {
        all_day_time(e, p);
        if was_all_day {
            p.remove("showWithoutTime");
        }
        return;
    }
    let zone = s.get("timeZone").and_then(Value::as_str);
    // The browser's rules for that zone; else the server's local start, moved as the instant moved.
    let moved = || {
        let local = s.get("start").and_then(Value::as_str).and_then(utc_date)?;
        let utc = s.get("utcStart").and_then(Value::as_str).and_then(utc_date)?;
        Some(local + (e.start - utc))
    };
    let start = (w.local)(zone, e.start).or_else(moved).unwrap_or(e.start);
    p.insert("start".into(), json!(local_string(start)));
    p.insert("duration".into(), json!(wall_duration(e.start, e.end, &|t| (w.local)(zone, t))));
    if was_all_day {
        p.insert("showWithoutTime".into(), json!(false));
    }
}

/// A whole series moved as the person moved its occurrence (from `was`, as the form showed it, to `e`; `o`, the server's
/// copy of it now): as many days earlier or later, at the occurrence's new time of day in the series' zone (`b`, the base
/// event, keeps it), for the occurrence's new length.
fn series_time(e: &CalEvent, was: &CalEvent, b: &Value, o: &Value, w: &Writing, p: &mut Map<String, Value>) {
    let local_of = |v: &Value| v.get("start").and_then(Value::as_str).and_then(utc_date);
    let Some(base) = local_of(b) else { return };
    let zone = b.get("timeZone").and_then(Value::as_str);
    // On the series' wall clock: by the zone's rules, else by the offset the server's copy of the occurrence has.
    let offset = local_of(o).zip(event_of(o)).map(|(l, then)| l - then.start);
    let local = |t: i64, all_day: bool| if all_day { Some(t) } else { (w.local)(zone, t).or_else(|| offset.map(|d| t + d)) };
    let (Some(old), Some(new)) = (local(was.start, was.all_day), local(e.start, e.all_day)) else { return };
    let day = |t: i64| t.div_euclid(86_400);
    let start = (day(base) + day(new) - day(old)) * 86_400 + if e.all_day { 0 } else { new.rem_euclid(86_400) };
    p.insert("start".into(), json!(local_string(start)));
    let was_all_day = b.get("showWithoutTime").and_then(Value::as_bool).unwrap_or(false);
    if e.all_day {
        p.insert("duration".into(), json!(format!("P{}D", ((e.end - e.start) / 86_400).max(1))));
    } else {
        p.insert("duration".into(), json!(wall_duration(e.start, e.end, &|t| (w.local)(zone, t))));
    }
    if e.all_day != was_all_day {
        p.insert("showWithoutTime".into(), json!(e.all_day));
        if e.all_day {
            p.insert("timeZone".into(), Value::Null);
        } else if zone.is_none() {
            p.insert("timeZone".into(), json!(w.zone));
        }
    }
}

/// The guests a person organizing `e` added or took out since `was` (the copy the change started from), and the person's
/// own answer where it changed since, made to the server's participants of `s`: the whole map, where it differs from the
/// server's (the organizer and everyone else as the server has them, those someone else added meanwhile among them); and
/// whether the server is to send the meeting mail: where the person organizes it with others taking part (the
/// invitations and their changes, as MailCraft mails them elsewhere), not for a draft. The person's answer to someone
/// else's meeting goes to its organizer by MailCraft's mail, as elsewhere.
fn participants(e: &CalEvent, was: &CalEvent, s: &Value, w: &Writing, p: &mut Map<String, Value>) -> bool {
    let server: Map<String, Value> = s.get("participants").and_then(Value::as_object).cloned().unwrap_or_default();
    let organizer = s
        .get("organizerCalendarAddress")
        .and_then(Value::as_str)
        .map(mailto)
        .or_else(|| server.values().find(|v| role(v, "owner")).and_then(participant_email));
    let organized = organizer.as_deref().map_or(server.is_empty(), |o| w.mine(o));
    let lists = |list: &[Attendee], email: &str| list.iter().any(|a| a.email.eq_ignore_ascii_case(email));
    let mut map = server.clone();
    if organized {
        // Taken out by the person: a guest's entry goes (never the organizer's, nor the person's own).
        map.retain(|_, v| {
            let Some(email) = participant_email(v) else { return true };
            let guest = attends(v) && !role(v, "owner") && !w.mine(&email);
            !(guest && lists(&was.attendees, &email) && !lists(&e.attendees, &email))
        });
        // Added by the person, unless the server has them already.
        for a in e.attendees.iter().filter(|a| !w.mine(&a.email) && !lists(&was.attendees, &a.email)) {
            if !map.values().any(|v| participant_email(v).is_some_and(|x| x.eq_ignore_ascii_case(&a.email))) {
                map.insert(free_key(&map, "p"), participant_json(a));
            }
        }
    }
    // The person's own answer, where it changed.
    let answer = |list: &[Attendee]| list.iter().find(|a| w.mine(&a.email)).map(|a| a.partstat.trim().to_ascii_lowercase()).filter(|x| !x.is_empty());
    if let Some(ours) = answer(&e.attendees).filter(|ours| answer(&was.attendees).as_ref() != Some(ours)) {
        for v in map.values_mut().filter(|v| participant_email(v).is_some_and(|x| w.mine(&x))) {
            if let Some(entry) = v.as_object_mut() {
                entry.insert("participationStatus".into(), json!(ours));
            }
        }
    }
    // A guest taken out is told too, the last one among them.
    let guests = |list: &[Attendee]| list.iter().any(|a| !w.mine(&a.email));
    let regrouped = map != server;
    if regrouped {
        p.insert("participants".into(), Value::Object(map));
    }
    let draft = s.get("isDraft").and_then(Value::as_bool).unwrap_or(false);
    organized && !draft && (guests(&e.attendees) || (regrouped && guests(&was.attendees)))
}

/// Whether deleting the server's event `s` is to tell its guests: one the person organizes, with others, not a draft.
fn scheduled(s: &Value, w: &Writing) -> bool {
    let draft = s.get("isDraft").and_then(Value::as_bool).unwrap_or(false);
    let (organizer, attendees) = people(s);
    !draft && organizer.is_some_and(|o| w.mine(&o.email)) && attendees.iter().any(|a| !w.mine(&a.email))
}

/// A guest invited here: to reply, by mail (`imip`).
fn participant_json(a: &Attendee) -> Value {
    let mut v = json!({
        "@type": "Participant",
        "email": a.email.trim(),
        "sendTo": {"imip": format!("mailto:{}", a.email.trim())},
        "roles": {"attendee": true},
        "participationStatus": if a.partstat.trim().is_empty() { "needs-action".to_string() } else { a.partstat.to_ascii_lowercase() },
        "expectReply": true,
    });
    if !a.name.trim().is_empty() {
        v["name"] = json!(a.name.trim());
    }
    v
}

fn keywords(categories: &[String]) -> Value {
    Value::Object(categories.iter().map(|c| (c.clone(), json!(true))).collect())
}

/// An address without its `mailto:`.
fn mailto(uri: &str) -> String {
    let uri = uri.trim();
    match uri.get(..7) {
        Some(scheme) if scheme.eq_ignore_ascii_case("mailto:") => uri.get(7..).unwrap_or_default().trim().to_string(),
        _ => uri.to_string(),
    }
}

/// A participant's address, as the server reads it: its calendar address (`calendarAddress`, which the Suite keeps for
/// each, with no `sendTo` for one it asks no answer of, the organizer among them), else its `email`, else where it is
/// sent to by mail, else its `scheduleId`.
fn participant_email(p: &Value) -> Option<String> {
    let at = |v: Option<&Value>| v.and_then(Value::as_str).map(mailto).filter(|e| e.contains('@'));
    at(p.get("calendarAddress"))
        .or_else(|| at(p.get("email")))
        .or_else(|| at(p.get("sendTo").and_then(|s| s.get("imip"))))
        .or_else(|| at(p.get("scheduleId")))
}

fn role(p: &Value, name: &str) -> bool {
    p.get("roles").and_then(|r| r.get(name)).and_then(Value::as_bool) == Some(true)
}

/// Whether a participant attends (its roles say so, or it has none).
fn attends(p: &Value) -> bool {
    p.get("roles").and_then(Value::as_object).is_none_or(|r| r.get("attendee").and_then(Value::as_bool) == Some(true))
}

/// Who organizes the event and who attends it.
fn people(v: &Value) -> (Option<Attendee>, Vec<Attendee>) {
    let address = v.get("organizerCalendarAddress").and_then(Value::as_str).map(mailto).filter(|a| a.contains('@'));
    let (mut organizer, mut attendees) = (None, Vec::new());
    for p in v.get("participants").and_then(Value::as_object).into_iter().flat_map(|m| m.values()).take(200) {
        let Some(email) = participant_email(p) else { continue };
        let a = Attendee {
            name: p.get("name").and_then(Value::as_str).map(str::trim).unwrap_or_default().to_string(),
            partstat: p.get("participationStatus").and_then(Value::as_str).unwrap_or("needs-action").to_ascii_uppercase(),
            rsvp: p.get("expectReply").and_then(Value::as_bool).unwrap_or(false),
            email,
        };
        let owner = role(p, "owner") || address.as_deref().is_some_and(|o| o.eq_ignore_ascii_case(&a.email));
        if owner && organizer.is_none() {
            organizer = Some(a.clone());
        }
        if attends(p) {
            attendees.push(a);
        }
    }
    (organizer.or_else(|| address.map(|email| Attendee { email, ..Default::default() })), attendees)
}

/// `2026-10-17T09:30:00`: a JSCalendar LocalDateTime, from seconds as if UTC.
fn local_string(t: i64) -> String {
    utc_string(t).trim_end_matches('Z').to_string()
}

/// A JSCalendar Duration from `start` to `end` (instants) in a zone (`local`: the local time there of an instant, as
/// [`Writing::local`] gives it; `None` where its rules are unknown): its whole days on the wall clock (`D`, which the
/// server reads as days of the calendar, to the same time the next day: 23 or 25 hours across a change of offset), then
/// the time that passes from there (`T`, read as such): `P1D`, `P1DT1H`, `PT22H30M`. Where the zone's rules are unknown,
/// all of it as the time that passes, which is right whatever the zone.
fn wall_duration(start: i64, end: i64, local: &dyn Fn(i64) -> Option<i64>) -> String {
    let end = end.max(start);
    let (Some(from), Some(to)) = (local(start), local(end)) else { return duration_parts(0, end - start) };
    // When the wall clock shows `l` (seconds, as if UTC), by its offsets at the start and at the end: the first time where
    // it shows it twice, and as if the offset before went on where it never shows it, as the server reads a local time.
    let (before, after) = (from - start, to - end);
    let instant = |l: i64| {
        let shows = |t: i64| local(t) == Some(l);
        match (shows(l - before), shows(l - after)) {
            (true, true) => (l - before).min(l - after),
            (false, true) => l - after,
            _ => l - before,
        }
    };
    let mut days = (to - from).div_euclid(86_400).max(0);
    while days > 0 && instant(from + days * 86_400) > end {
        days -= 1;
    }
    duration_parts(days, (end - instant(from + days * 86_400)).max(0))
}

/// A JSCalendar Duration of `days` days and `secs` seconds, the hours past a day as they are: `P1D`, `PT25H`, `P1DT2H30M`,
/// `PT0S`.
fn duration_parts(days: i64, secs: i64) -> String {
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    let mut out = String::from("P");
    if days > 0 {
        out.push_str(&format!("{days}D"));
    }
    if secs > 0 || days == 0 {
        out.push('T');
        if h > 0 {
            out.push_str(&format!("{h}H"));
        }
        if m > 0 {
            out.push_str(&format!("{m}M"));
        }
        if s > 0 || secs == 0 {
            out.push_str(&format!("{s}S"));
        }
    }
    out
}

const WEEKDAYS: [&str; 7] = ["mo", "tu", "we", "th", "fr", "sa", "su"];

/// An iCalendar rule (`FREQ=WEEKLY;BYDAY=MO,WE;UNTIL=…`) as a JSCalendar `recurrenceRule` (RFC 8984 §4.3.3), its end
/// (`UNTIL`, an instant) local in the event's zone (`local`, as [`Writing::local`]); `None` without a frequency.
pub fn rule_json(rrule: &str, local: &dyn Fn(i64) -> Option<i64>) -> Option<Value> {
    let mut m = Map::new();
    m.insert("@type".into(), json!("RecurrenceRule"));
    let ints = |v: &str| -> Vec<i64> { v.split(',').filter_map(|x| x.trim().parse().ok()).take(366).collect() };
    for part in rrule.split(';') {
        let Some((k, v)) = part.split_once('=') else { continue };
        let v = v.trim();
        let key = match k.trim().to_ascii_uppercase().as_str() {
            "FREQ" => {
                m.insert("frequency".into(), json!(v.to_ascii_lowercase()));
                continue;
            }
            "INTERVAL" => {
                if let Some(n) = v.parse::<u64>().ok().filter(|n| *n > 1) {
                    m.insert("interval".into(), json!(n));
                }
                continue;
            }
            "COUNT" => {
                if let Ok(n) = v.parse::<u64>() {
                    m.insert("count".into(), json!(n));
                }
                continue;
            }
            "UNTIL" => {
                if let Some(u) = until(v, local) {
                    m.insert("until".into(), json!(u));
                }
                continue;
            }
            "BYDAY" => {
                let days: Vec<Value> = v
                    .split(',')
                    .filter_map(|d| {
                        let d = d.trim();
                        let split = d.len().checked_sub(2)?;
                        let (n, day) = (d.get(..split)?, d.get(split..)?.to_ascii_lowercase());
                        WEEKDAYS.contains(&day.as_str()).then_some(())?;
                        let mut nday = json!({"@type": "NDay", "day": day});
                        if let Some(nth) = n.trim_start_matches('+').parse::<i64>().ok().filter(|n| *n != 0) {
                            nday["nthOfPeriod"] = json!(nth);
                        }
                        Some(nday)
                    })
                    .take(31)
                    .collect();
                m.insert("byDay".into(), json!(days));
                continue;
            }
            "BYMONTH" => {
                let months: Vec<String> = v.split(',').map(str::trim).filter(|x| !x.is_empty()).take(12).map(str::to_string).collect();
                m.insert("byMonth".into(), json!(months));
                continue;
            }
            "WKST" => {
                let day = v.to_ascii_lowercase();
                if WEEKDAYS.contains(&day.as_str()) {
                    m.insert("firstDayOfWeek".into(), json!(day));
                }
                continue;
            }
            "BYMONTHDAY" => "byMonthDay",
            "BYYEARDAY" => "byYearDay",
            "BYWEEKNO" => "byWeekNo",
            "BYHOUR" => "byHour",
            "BYMINUTE" => "byMinute",
            "BYSECOND" => "bySecond",
            "BYSETPOS" => "bySetPosition",
            _ => continue,
        };
        m.insert(key.into(), json!(ints(v)));
    }
    m.contains_key("frequency").then_some(Value::Object(m))
}

/// An RRULE's `UNTIL` as a JSCalendar LocalDateTime: a date's whole last day; an instant (`…Z`) local in the event's
/// zone; a floating time as it is.
fn until(v: &str, local: &dyn Fn(i64) -> Option<i64>) -> Option<String> {
    let num = |a: usize, b: usize| v.get(a..b).and_then(|x| x.parse::<u32>().ok());
    let day = days_from_civil(i64::from(num(0, 4)?), num(4, 6)?, num(6, 8)?) * 86_400;
    if v.len() == 8 {
        return Some(local_string(day + 86_399));
    }
    let t = day + i64::from(num(9, 11)?) * 3600 + i64::from(num(11, 13)?) * 60 + i64::from(num(13, 15)?);
    Some(local_string(if v.ends_with('Z') { local(t).unwrap_or(t) } else { t }))
}

/// A JSCalendar `recurrenceRule` as an iCalendar rule (`FREQ=WEEKLY;BYDAY=MO`), the defaults left out, so that the same
/// rule reads the same.
pub fn rule_text(v: &Value) -> Option<String> {
    let freq = v.get("frequency")?.as_str()?.trim().to_ascii_uppercase();
    if freq.is_empty() {
        return None;
    }
    let mut parts = vec![format!("FREQ={freq}")];
    if let Some(n) = v.get("interval").and_then(Value::as_u64).filter(|n| *n > 1) {
        parts.push(format!("INTERVAL={n}"));
    }
    if let Some(n) = v.get("count").and_then(Value::as_u64) {
        parts.push(format!("COUNT={n}"));
    }
    if let Some(u) = v.get("until").and_then(Value::as_str).and_then(utc_date) {
        let (y, m, d) = civil_from_days(u.div_euclid(86_400));
        let s = u.rem_euclid(86_400);
        parts.push(format!("UNTIL={y:04}{m:02}{d:02}T{:02}{:02}{:02}", s / 3600, s / 60 % 60, s % 60));
    }
    let days: Vec<String> = v
        .get("byDay")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|d| {
            let day = d.get("day")?.as_str()?.to_ascii_uppercase();
            Some(match d.get("nthOfPeriod").and_then(Value::as_i64) {
                Some(n) => format!("{n}{day}"),
                None => day,
            })
        })
        .collect();
    if !days.is_empty() {
        parts.push(format!("BYDAY={}", days.join(",")));
    }
    for (key, name) in [
        ("byMonthDay", "BYMONTHDAY"),
        ("byMonth", "BYMONTH"),
        ("byYearDay", "BYYEARDAY"),
        ("byWeekNo", "BYWEEKNO"),
        ("byHour", "BYHOUR"),
        ("byMinute", "BYMINUTE"),
        ("bySecond", "BYSECOND"),
        ("bySetPosition", "BYSETPOS"),
    ] {
        let values: Vec<String> = v
            .get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|x| x.as_i64().map(|n| n.to_string()).or_else(|| x.as_str().map(str::to_string)))
            .collect();
        if !values.is_empty() {
            parts.push(format!("{name}={}", values.join(",")));
        }
    }
    if let Some(day) = v.get("firstDayOfWeek").and_then(Value::as_str).filter(|d| !d.eq_ignore_ascii_case("mo")) {
        parts.push(format!("WKST={}", day.to_ascii_uppercase()));
    }
    Some(parts.join(";"))
}

/// The span read around `now`: [`DAYS_BACK`] days back and [`DAYS_AHEAD`] ahead, or as much of them as a server that
/// expands repeating events over a shorter span (`span`, seconds) allows, back and ahead in the same proportion.
pub fn window(now: i64, span: Option<i64>) -> (i64, i64) {
    let (back, ahead) = (DAYS_BACK * 86_400, DAYS_AHEAD * 86_400);
    match span.filter(|s| *s < back + ahead) {
        Some(span) => {
            let back = span.saturating_mul(DAYS_BACK) / (DAYS_BACK + DAYS_AHEAD);
            (now - back, now - back + span)
        }
        None => (now - back, now + ahead),
    }
}

/// `2026-10-17T09:30:00Z`: a JMAP UTCDate.
pub fn utc_string(t: i64) -> String {
    let (y, m, d) = civil_from_days(t.div_euclid(86_400));
    let s = t.rem_euclid(86_400);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", s / 3600, s / 60 % 60, s % 60)
}

/// The sync the engine merges, from the server's calendars and events. A calendar is writable where the server lets the
/// person write its events (`myRights.mayWriteAll`, or `mayWriteOwn` for the ones they organize).
pub fn calendars_sync(calendars: &[Value], events: &[Value], now: i64) -> DavSync {
    let calendars: Vec<CalendarInfo> = calendars
        .iter()
        .enumerate()
        .filter_map(|(n, c)| {
            let id = c.get("id")?.as_str()?.to_string();
            let name = c.get("name").and_then(Value::as_str).filter(|n| !n.trim().is_empty()).unwrap_or("Calendar").to_string();
            let right = |key: &str| c.get("myRights").and_then(|r| r.get(key)).and_then(Value::as_bool) == Some(true);
            let read_only = !(right("mayWriteAll") || right("mayWriteOwn"));
            Some(CalendarInfo { id, name, color: u8::try_from(n % 8).unwrap_or(0), read_only, ctag: String::new() })
        })
        .collect();
    let mut stored = Vec::new();
    for v in events {
        let (Some(id), Some(event)) = (v.get("id").and_then(Value::as_str), event_of(v)) else { continue };
        // Once, as the engine keeps an occurrence (by its uid and start), in the first of its calendars the server lists
        // that the person may write, else the first.
        let holds = |c: &&CalendarInfo| v.get("calendarIds").and_then(|m| m.get(&c.id)).is_some_and(|on| on.as_bool() != Some(false));
        let Some(calendar) = calendars.iter().filter(holds).find(|c| !c.read_only).or_else(|| calendars.iter().find(holds)) else { continue };
        stored.push(StoredEvent { calendar: calendar.id.clone(), href: format!("{HREF_PREFIX}{id}"), etag: String::new(), event, dirty: false });
    }
    DavSync {
        caldav_home: Some("jmap".into()),
        checked: now,
        synced_calendars: calendars.iter().map(|c| c.id.clone()).collect(),
        calendars: Some(calendars),
        events: stored,
        ..Default::default()
    }
}

/// A JSCalendar event (one occurrence, its times in UTC) as MailCraft shows it; `None` without a start. An occurrence of
/// a series the server expanded names its series (its base event's address there) and the series' rule when the server
/// gives it; an event the server didn't expand keeps its rule, for the engine to expand.
pub fn event_of(v: &Value) -> Option<CalEvent> {
    let text = |key: &str| v.get(key).and_then(Value::as_str).map(str::trim).unwrap_or_default().to_string();
    let all_day = v.get("showWithoutTime").and_then(Value::as_bool).unwrap_or(false);
    let length = v.get("duration").and_then(Value::as_str).and_then(duration).unwrap_or(if all_day { 86_400 } else { 0 }).max(0);
    // A day-long event is its days, from midnight UTC as MailCraft keeps them; a timed one, the server's UTC times.
    let start = if all_day {
        v.get("start").and_then(Value::as_str).and_then(utc_date)
    } else {
        v.get("utcStart").and_then(Value::as_str).and_then(utc_date)
    }?;
    let end = if all_day { None } else { v.get("utcEnd").and_then(Value::as_str).and_then(utc_date) }.unwrap_or(start + length).max(start);
    let location = v
        .get("locations")
        .and_then(Value::as_object)
        .and_then(|m| m.values().find_map(|l| l.get("name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty())))
        .unwrap_or_default()
        .to_string();
    let uid = [text("uid"), text("id")].into_iter().find(|u| !u.is_empty())?;
    // An occurrence of a repeating event stands apart from the others by when it starts.
    let occurrence = v.get("recurrenceId").is_some_and(|r| !r.is_null());
    // Its keywords (JSCalendar's tags, where iCalendar's CATEGORIES go) are its categories, which colour it in the
    // calendar; as many as an iCalendar event keeps (model/src/ical.rs).
    let mut categories: Vec<String> = Vec::new();
    for (k, on) in v.get("keywords").and_then(Value::as_object).into_iter().flatten() {
        let k: String = k.trim().chars().take(100).collect();
        if on.as_bool() == Some(true) && !k.is_empty() && !categories.contains(&k) && categories.len() < 20 {
            categories.push(k);
        }
    }
    let rule = v.get("recurrenceRule").filter(|r| r.is_object()).and_then(rule_text);
    let mut extra = Vec::new();
    if occurrence {
        if let Some(base) = v.get("baseEventId").and_then(Value::as_str).map(str::trim).filter(|b| !b.is_empty()) {
            extra.push(format!("{SERIES_LINE}{HREF_PREFIX}{base}"));
        }
        if let Some(rule) = &rule {
            extra.push(format!("{SERIES_RULE_LINE}{rule}"));
        }
    }
    let (organizer, attendees) = people(v);
    Some(CalEvent {
        uid,
        summary: text("title"),
        description: text("description"),
        location,
        start,
        end,
        all_day,
        rrule: if occurrence { None } else { rule },
        recurrence_id: occurrence.then_some(start),
        organizer,
        attendees,
        status: text("status").to_ascii_uppercase(),
        categories,
        extra,
        ..Default::default()
    })
}

/// One sync of what the server offers: its contacts and its calendars together, so that neither of the two homes of
/// the account is said to be gone when only the other was read (`offered`: whether the session offers contacts, and
/// calendars). What each part stored, let go of, refused and could not send comes along.
pub fn pim_sync(contacts: Option<DavSync>, calendars: Option<DavSync>, offered: (bool, bool), now: i64) -> Option<DavSync> {
    if contacts.is_none() && calendars.is_none() {
        return None;
    }
    let mut out = DavSync { checked: now, ..Default::default() };
    // A part offered but not read this time (it failed) keeps its home, and its collections stay as they are.
    if offered.0 {
        out.carddav_home = Some("jmap".into());
    }
    if offered.1 {
        out.caldav_home = Some("jmap".into());
    }
    if let Some(c) = contacts {
        out.carddav_home = c.carddav_home;
        out.books = c.books;
        out.cards = c.cards;
        out.synced_books = c.synced_books;
        out.partial_books = c.partial_books;
        out.no_delete_books = c.no_delete_books;
        out.pushed_cards = c.pushed_cards;
        out.refused_cards = c.refused_cards;
        out.refused_deletions.extend(c.refused_deletions);
        out.deleted.extend(c.deleted);
        out.errors.extend(c.errors);
    }
    if let Some(c) = calendars {
        out.caldav_home = c.caldav_home;
        out.calendars = c.calendars;
        out.events = c.events;
        out.synced_calendars = c.synced_calendars;
        out.partial_calendars = c.partial_calendars;
        out.window = c.window;
        out.pushed_events = c.pushed_events;
        out.refused_events = c.refused_events;
        out.refused_deletions.extend(c.refused_deletions);
        out.deleted.extend(c.deleted);
        out.errors.extend(c.errors);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use mailcraft_model::pim::{SERIES_CHANGE, Tombstone};

    use super::*;

    /// Europe/Zurich's offset at an instant: CEST (+2) from the last Sunday of March, 01:00 UTC, to the last Sunday of
    /// October, 01:00 UTC; CET (+1) otherwise.
    fn zurich(t: i64) -> i64 {
        let (y, _, _) = civil_from_days(t.div_euclid(86_400));
        let last_sunday = |m: u32| {
            let end = days_from_civil(y, m, 31);
            (end - (end - 3).rem_euclid(7)) * 86_400 + 3600
        };
        if t >= last_sunday(3) && t < last_sunday(10) { 7200 } else { 3600 }
    }

    /// The browser's rules, for the tests: Zurich's (summer and winter time), New York's in summer, and a zone it doesn't
    /// know.
    fn zones(zone: Option<&str>, t: i64) -> Option<i64> {
        match zone {
            None | Some("Europe/Zurich") => Some(t + zurich(t)),
            Some("America/New_York") => Some(t - 4 * 3600),
            _ => None,
        }
    }

    fn writing(keywords: bool) -> Writing<'static> {
        Writing { me: vec!["me@example.ch".into()], keywords, zone: "Europe/Zurich".into(), local: &zones }
    }

    /// The account's calendars as the engine keeps them: the server's K1, writable.
    fn jmap_pim(events: Vec<StoredEvent>, deleted: Vec<Tombstone>) -> Pim {
        Pim {
            calendars: vec![CalendarInfo { id: "K1".into(), name: "Agenda".into(), read_only: false, ..Default::default() }],
            events,
            deleted,
            caldav_home: Some("jmap".into()),
            ..Default::default()
        }
    }

    fn at(s: &str) -> i64 {
        utc_date(s).unwrap()
    }

    /// An event as the engine keeps it, changed here (`dirty`).
    fn here(href: &str, summary: &str, start: &str, minutes: i64) -> StoredEvent {
        let start = at(start);
        let event = CalEvent { uid: format!("u-{summary}"), summary: summary.into(), start, end: start + minutes * 60, ..Default::default() };
        StoredEvent { calendar: "K1".into(), href: href.into(), etag: String::new(), event, dirty: true }
    }

    /// A server that answers each method call with `answer(name, args)` (`{"error": …}` a method error, `{"transport": …}`
    /// a failure of the whole request, `{"status": 401}` that HTTP status), and keeps the calls in order.
    struct Server<F: Fn(&str, &Value) -> Value> {
        session: Value,
        answer: F,
        asked: RefCell<Vec<(String, Value)>>,
    }

    impl<F: Fn(&str, &Value) -> Value> crate::Http for Server<F> {
        async fn send(&self, req: crate::Request) -> crate::Result<crate::Response> {
            if req.method == "GET" {
                return Ok(crate::Response { status: 200, body: self.session.to_string().into_bytes() });
            }
            let body: Value = serde_json::from_slice(&req.body).map_err(|e| crate::Error::Protocol(e.to_string()))?;
            let mut responses = Vec::new();
            for call in body["methodCalls"].as_array().cloned().unwrap_or_default() {
                let name = call[0].as_str().unwrap_or_default().to_string();
                self.asked.borrow_mut().push((name.clone(), call[1].clone()));
                let r = (self.answer)(&name, &call[1]);
                if let Some(why) = r.get("transport") {
                    return Err(crate::Error::Io(why.to_string()));
                }
                if let Some(status) = r.get("status").and_then(Value::as_u64).and_then(|s| u16::try_from(s).ok()) {
                    return Ok(crate::Response { status, body: b"{}".to_vec() });
                }
                responses.push(match r.get("error") {
                    Some(e) => json!(["error", e, call[2]]),
                    None => json!([name, r, call[2]]),
                });
            }
            Ok(crate::Response { status: 200, body: json!({"methodResponses": responses}).to_string().into_bytes() })
        }
    }

    fn serve<F: Fn(&str, &Value) -> Value>(set_max: u64, answer: F) -> Client<Server<F>> {
        let mut session = session();
        session["capabilities"]["urn:ietf:params:jmap:core"]["maxObjectsInSet"] = json!(set_max);
        let server = Server { session, answer, asked: RefCell::new(Vec::new()) };
        crate::block_on(Client::connect(server, "jmap.example.com", "me", "pw", true)).unwrap()
    }

    fn asked<F: Fn(&str, &Value) -> Value>(c: &Client<Server<F>>, method: &str) -> Vec<Value> {
        c.http.asked.borrow().iter().filter(|(n, _)| n == method).map(|(_, a)| a.clone()).collect()
    }

    /// The server's events by id (`CalendarEvent/get`); a set stores what it is given (`N1`, `N2`… for new ones); the
    /// calendar K1, empty, read after.
    fn keeping(events: Vec<Value>) -> impl Fn(&str, &Value) -> Value {
        move |name: &str, args: &Value| match name {
            "Calendar/get" => json!({"list": [{"id": "K1", "name": "Agenda", "myRights": {"mayWriteAll": true, "mayWriteOwn": true}}]}),
            "CalendarEvent/query" => json!({"ids": [], "total": 0}),
            "CalendarEvent/get" => {
                let ids: Vec<&str> = args["ids"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
                let found: Vec<Value> = events.iter().filter(|e| ids.contains(&e["id"].as_str().unwrap_or_default())).cloned().collect();
                let missing: Vec<&str> = ids.iter().copied().filter(|id| !events.iter().any(|e| e["id"] == json!(id))).collect();
                json!({"list": found, "notFound": missing})
            }
            "CalendarEvent/set" => {
                let created: Map<String, Value> = args["create"]
                    .as_object()
                    .map(|m| m.keys().enumerate().map(|(n, k)| (k.clone(), json!({"id": format!("N{}", n + 1)}))).collect())
                    .unwrap_or_default();
                let updated: Map<String, Value> =
                    args["update"].as_object().map(|m| m.keys().map(|k| (k.clone(), Value::Null)).collect()).unwrap_or_default();
                json!({"created": created, "updated": updated, "destroyed": args.get("destroy").cloned().unwrap_or(json!([]))})
            }
            _ => json!({}),
        }
    }

    /// The server's copy of an event: at 07:00Z for an hour, from Zurich, with no guests.
    fn theirs(id: &str, title: &str) -> Value {
        json!({"id": id, "uid": format!("u-{title}"), "calendarIds": {"K1": true}, "title": title, "start": "2026-10-19T09:00:00",
               "timeZone": "Europe/Zurich", "duration": "PT1H", "utcStart": "2026-10-19T07:00:00Z", "utcEnd": "2026-10-19T08:00:00Z"})
    }

    #[test]
    fn a_new_event_goes_whole_in_the_browser_s_zone() {
        let mut new = here("", "Site visit", "2026-10-19T07:00:00Z", 90);
        new.event.location = "Office".into();
        new.event.rrule = Some("FREQ=WEEKLY;BYDAY=MO;UNTIL=20261231T230000Z".into());
        new.event.categories = vec!["Work".into()];
        let pim = jmap_pim(vec![new], Vec::new());
        let c = serve(500, keeping(Vec::new()));
        let sync = crate::block_on(c.sync_calendars(&pim, &writing(false), 1)).unwrap().unwrap();
        let set = &asked(&c, "CalendarEvent/set")[0];
        let body = &set["create"]["k0"];
        assert_eq!((&body["uid"], &body["calendarIds"], &body["title"]), (&json!("u-Site visit"), &json!({"K1": true}), &json!("Site visit")));
        // 07:00Z is 09:00 in the browser's zone, which it is written in.
        assert_eq!(
            (&body["start"], &body["timeZone"], &body["duration"]),
            (&json!("2026-10-19T09:00:00"), &json!("Europe/Zurich"), &json!("PT1H30M"))
        );
        assert_eq!(body["locations"]["l1"]["name"], "Office");
        assert_eq!(
            body["recurrenceRule"],
            json!({"@type": "RecurrenceRule", "frequency": "weekly", "byDay": [{"@type": "NDay", "day": "mo"}], "until": "2027-01-01T00:00:00"})
        );
        for never in ["keywords", "organizerCalendarAddress", "isDraft", "recurrenceOverrides", "participants"] {
            assert!(body.get(never).is_none(), "{never}: {body}");
        }
        assert!(set.get("sendSchedulingMessages").is_none(), "no guests: no meeting mail");
        assert_eq!(sync.pushed_events.iter().map(|e| (e.href.as_str(), e.dirty)).collect::<Vec<_>>(), vec![("jmap:N1", false)], "stored there");
        // Where the server keeps keywords, the categories go too; a meeting the person organizes invites its guests.
        let mut meeting = here("", "Review", "2026-10-20T12:00:00Z", 60);
        meeting.event.categories = vec!["Work".into()];
        meeting.event.organizer = Some(Attendee { email: "me@example.ch".into(), ..Default::default() });
        meeting.event.attendees = vec![
            Attendee { email: "me@example.ch".into(), partstat: "ACCEPTED".into(), ..Default::default() },
            Attendee { email: "ana@example.ch".into(), name: "Ana Lima".into(), partstat: "NEEDS-ACTION".into(), rsvp: true },
        ];
        let c = serve(500, keeping(Vec::new()));
        crate::block_on(c.sync_calendars(&jmap_pim(vec![meeting], Vec::new()), &writing(true), 1)).unwrap();
        let set = &asked(&c, "CalendarEvent/set")[0];
        assert_eq!(set["create"]["k0"]["keywords"], json!({"Work": true}));
        assert_eq!(
            set["create"]["k0"]["participants"],
            json!({"p1": {"@type": "Participant", "email": "ana@example.ch", "name": "Ana Lima",
            "sendTo": {"imip": "mailto:ana@example.ch"}, "roles": {"attendee": true}, "participationStatus": "needs-action", "expectReply": true}}),
            "the guests, not the person (the server keeps the organizer)"
        );
        assert_eq!(set["sendSchedulingMessages"], true, "the server invites them");
    }

    #[test]
    fn a_day_long_event_is_its_days() {
        let mut day = here("", "Closed", "2026-12-24T00:00:00Z", 2 * 24 * 60);
        day.event.all_day = true;
        let c = serve(500, keeping(Vec::new()));
        crate::block_on(c.sync_calendars(&jmap_pim(vec![day], Vec::new()), &writing(false), 1)).unwrap();
        let body = &asked(&c, "CalendarEvent/set")[0]["create"]["k0"];
        assert_eq!((&body["start"], &body["duration"], &body["showWithoutTime"]), (&json!("2026-12-24T00:00:00"), &json!("P2D"), &json!(true)));
    }

    #[test]
    fn a_change_is_a_patch_of_what_differs_from_the_server_s_copy() {
        // The title changed here: only the title goes, the server keeps the rest (its zone included).
        let title = here("jmap:E1", "Site visit, moved", "2026-10-19T07:00:00Z", 60);
        let c = serve(500, keeping(vec![theirs("E1", "Site visit")]));
        let sync = crate::block_on(c.sync_calendars(&jmap_pim(vec![title], Vec::new()), &writing(false), 1)).unwrap().unwrap();
        assert_eq!(asked(&c, "CalendarEvent/get")[0]["ids"], json!(["E1"]), "the server's copy read first");
        assert_eq!(asked(&c, "CalendarEvent/set")[0]["update"], json!({"E1": {"title": "Site visit, moved"}}));
        assert_eq!(sync.pushed_events.iter().map(|e| (e.href.as_str(), e.dirty)).collect::<Vec<_>>(), vec![("jmap:E1", false)]);
        // Its time changed: written in the browser's zone, for its length.
        let moved = here("jmap:E1", "Site visit", "2026-10-19T08:00:00Z", 30);
        let c = serve(500, keeping(vec![theirs("E1", "Site visit")]));
        crate::block_on(c.sync_calendars(&jmap_pim(vec![moved], Vec::new()), &writing(false), 1)).unwrap();
        assert_eq!(
            asked(&c, "CalendarEvent/set")[0]["update"]["E1"],
            json!({"start": "2026-10-19T10:00:00", "timeZone": "Europe/Zurich", "duration": "PT30M", "showWithoutTime": false})
        );
        // Nothing that differs: nothing sent, and done.
        let same = here("jmap:E1", "Site visit", "2026-10-19T07:00:00Z", 60);
        let c = serve(500, keeping(vec![theirs("E1", "Site visit")]));
        let sync = crate::block_on(c.sync_calendars(&jmap_pim(vec![same], Vec::new()), &writing(false), 1)).unwrap().unwrap();
        assert!(asked(&c, "CalendarEvent/set").is_empty());
        assert_eq!(sync.pushed_events.len(), 1);
    }

    #[test]
    fn an_occurrence_keeps_its_zone_and_a_whole_series_goes_through_its_base_event() {
        let series = |id: &str, start: &str, utc: &str| {
            let mut v = theirs(id, "Weekly");
            v["start"] = json!(start);
            v["utcStart"] = json!(utc);
            v["utcEnd"] = json!(utc_string(at(utc) + 3600));
            v["timeZone"] = json!("America/New_York");
            v["recurrenceId"] = json!(start);
            v["baseEventId"] = json!("B1");
            v
        };
        let mut base = theirs("B1", "Weekly");
        (base["start"], base["timeZone"], base["recurrenceRule"]) =
            (json!("2026-10-05T09:00:00"), json!("America/New_York"), json!({"@type": "RecurrenceRule", "frequency": "weekly"}));
        let occurrence = series("E2", "2026-10-19T09:00:00", "2026-10-19T13:00:00Z");
        let read = event_of(&occurrence).unwrap();
        assert_eq!(series_line(&read, SERIES_LINE), Some("jmap:B1"), "an occurrence names its series");
        // This occurrence, an hour later: its own id, in its own zone (New York), no zone sent.
        let mut later = StoredEvent { calendar: "K1".into(), href: "jmap:E2".into(), etag: String::new(), event: read.clone(), dirty: true };
        later.event.start += 3600;
        later.event.end += 3600;
        let c = serve(500, keeping(vec![base.clone(), occurrence.clone()]));
        crate::block_on(c.sync_calendars(&jmap_pim(vec![later.clone()], Vec::new()), &writing(false), 1)).unwrap();
        assert_eq!(asked(&c, "CalendarEvent/set")[0]["update"], json!({"E2": {"start": "2026-10-19T10:00:00", "duration": "PT1H"}}));
        // The whole series: its base event, moved as the occurrence was, the rule as it is.
        let mut whole = later.clone();
        mailcraft_model::pim::set_series_line(&mut whole.event, SERIES_CHANGE_LINE, Some(SERIES_CHANGE));
        whole.event.summary = "Weekly, later".into();
        let c = serve(500, keeping(vec![base.clone(), occurrence.clone()]));
        let sync = crate::block_on(c.sync_calendars(&jmap_pim(vec![whole.clone()], Vec::new()), &writing(false), 1)).unwrap().unwrap();
        let mut got = asked(&c, "CalendarEvent/get")[0]["ids"].as_array().cloned().unwrap();
        got.sort_by_key(|v| v.to_string());
        assert_eq!(got, vec![json!("B1"), json!("E2")], "the base, and the occurrence it moves from");
        assert_eq!(
            asked(&c, "CalendarEvent/set")[0]["update"],
            json!({"B1": {"title": "Weekly, later", "start": "2026-10-05T10:00:00", "duration": "PT1H"}})
        );
        assert_eq!(
            sync.pushed_events.iter().map(|e| e.href.as_str()).collect::<Vec<_>>(),
            vec!["jmap:E2"],
            "the occurrence it was made from is done"
        );
        // Its rule changed too: it goes.
        mailcraft_model::pim::set_series_line(&mut whole.event, SERIES_CHANGE_LINE, Some(SERIES_CHANGE_RULE));
        mailcraft_model::pim::set_series_line(&mut whole.event, SERIES_RULE_LINE, Some("FREQ=DAILY"));
        let c = serve(500, keeping(vec![base, occurrence]));
        crate::block_on(c.sync_calendars(&jmap_pim(vec![whole], Vec::new()), &writing(false), 1)).unwrap();
        assert_eq!(asked(&c, "CalendarEvent/set")[0]["update"]["B1"]["recurrenceRule"], json!({"@type": "RecurrenceRule", "frequency": "daily"}));
    }

    #[test]
    fn deletions_go_and_tell_the_guests_of_the_person_s_meetings() {
        let mut meeting = theirs("E1", "Review");
        meeting["organizerCalendarAddress"] = json!("mailto:me@example.ch");
        meeting["participants"] = json!({
            "o": {"@type": "Participant", "sendTo": {"imip": "mailto:me@example.ch"}, "roles": {"owner": true, "attendee": true}},
            "a": {"@type": "Participant", "sendTo": {"imip": "mailto:ana@example.ch"}, "roles": {"attendee": true}}
        });
        let mut draft = meeting.clone();
        (draft["id"], draft["isDraft"]) = (json!("E2"), json!(true));
        let gone = |href: &str| Tombstone { href: href.into(), etag: String::new(), calendar: "K1".into() };
        let pim = jmap_pim(Vec::new(), vec![gone("jmap:E1"), gone("jmap:E2"), gone("jmap:E3"), gone("jmap:E9")]);
        let refusing = |name: &str, args: &Value| {
            let mut answer = keeping(vec![meeting.clone(), draft.clone(), theirs("E3", "Plain")])(name, args);
            if name == "CalendarEvent/set" && args["destroy"].as_array().is_some_and(|d| d.contains(&json!("E3"))) {
                answer["destroyed"] = json!(args["destroy"].as_array().unwrap().iter().filter(|d| **d != json!("E3")).collect::<Vec<_>>());
                answer["notDestroyed"] = json!({"E3": {"type": "forbidden", "description": "Linked to a project"}});
            }
            answer
        };
        let c = serve(500, refusing);
        let sync = crate::block_on(c.sync_calendars(&pim, &writing(false), 1)).unwrap().unwrap();
        let sets = asked(&c, "CalendarEvent/set");
        // Not told: a draft, an event with no guests; told: the person's meeting with a guest.
        assert_eq!((sets.len(), &sets[0]["destroy"], sets[0].get("sendSchedulingMessages")), (2, &json!(["E2", "E3"]), None));
        assert_eq!((&sets[1]["destroy"], &sets[1]["sendSchedulingMessages"]), (&json!(["E1"]), &json!(true)));
        let mut done = sync.deleted.clone();
        done.sort();
        assert_eq!(done, vec!["jmap:E1", "jmap:E2", "jmap:E3", "jmap:E9"], "made, refused and said, or already gone (E9)");
        assert_eq!(sync.errors, vec!["an event was not deleted: not allowed: Linked to a project"], "the refusal said once");
        assert_eq!(sync.refused_deletions, vec!["jmap:E3"], "refused for good, not the one already gone (E9)");
    }

    #[test]
    fn the_person_s_answer_goes_and_mailcraft_tells_the_organizer() {
        let mut invited = theirs("E1", "Board");
        invited["organizerCalendarAddress"] = json!("mailto:boss@example.ch");
        invited["participants"] = json!({
            "b": {"@type": "Participant", "sendTo": {"imip": "mailto:boss@example.ch"}, "roles": {"owner": true}},
            "m": {"@type": "Participant", "sendTo": {"imip": "mailto:me@example.ch"}, "roles": {"attendee": true}, "participationStatus": "needs-action"}
        });
        let mut answered =
            StoredEvent { calendar: "K1".into(), href: "jmap:E1".into(), etag: String::new(), event: event_of(&invited).unwrap(), dirty: true };
        assert_eq!(answered.event.organizer.as_ref().map(|o| o.email.as_str()), Some("boss@example.ch"), "who organizes it, read");
        for a in &mut answered.event.attendees {
            a.partstat = "ACCEPTED".into();
        }
        let c = serve(500, keeping(vec![invited]));
        crate::block_on(c.sync_calendars(&jmap_pim(vec![answered], Vec::new()), &writing(false), 1)).unwrap();
        let set = &asked(&c, "CalendarEvent/set")[0];
        assert_eq!(
            set["update"],
            json!({"E1": {"participants": {
                "b": {"@type": "Participant", "sendTo": {"imip": "mailto:boss@example.ch"}, "roles": {"owner": true}},
                "m": {"@type": "Participant", "sendTo": {"imip": "mailto:me@example.ch"}, "roles": {"attendee": true}, "participationStatus": "accepted"}
            }}}),
            "the answer only, in the whole map (the server takes no patch of a part): the guests are the organizer's"
        );
        assert!(set.get("sendSchedulingMessages").is_none(), "MailCraft mails the reply itself, as for any other calendar");
    }

    /// The person's copy, changed here, of the server's event `v` (as `change` makes it).
    fn changed(v: &Value, change: impl FnOnce(&mut CalEvent)) -> StoredEvent {
        let mut event = event_of(v).unwrap();
        change(&mut event);
        let href = format!("{HREF_PREFIX}{}", v["id"].as_str().unwrap());
        StoredEvent { calendar: "K1".into(), href, etag: String::new(), event, dirty: true }
    }

    /// What one sync of `events` (the copies here, as read before they were changed: `originals`) sends of them to a server
    /// that has `server`.
    fn sent(events: Vec<StoredEvent>, originals: Vec<(&str, Value)>, server: Vec<Value>) -> Value {
        let mut pim = jmap_pim(events, Vec::new());
        pim.originals = originals.into_iter().map(|(href, v)| (href.to_string(), event_of(&v).unwrap())).collect();
        let c = serve(500, keeping(server));
        crate::block_on(c.sync_calendars(&pim, &writing(false), 1)).unwrap();
        asked(&c, "CalendarEvent/set").into_iter().next().unwrap_or(Value::Null)
    }

    #[test]
    fn a_location_and_the_guests_go_as_whole_maps() {
        // The server takes no patch of a part of a property (Neoffice's facade: no `locations/a/name`): the whole map goes,
        // with the person's change made to it, and the rest of it as the server has it.
        let mut two = theirs("E1", "Site visit");
        two["locations"] = json!({"a": {"@type": "Location", "name": "Room 1"}, "b": {"@type": "Location", "name": "Car park"}});
        assert_eq!(event_of(&two).unwrap().location, "Room 1", "the first named one shown");
        let set = sent(vec![changed(&two, |e| e.location = "Room 2".into())], Vec::new(), vec![two.clone()]);
        assert_eq!(
            set["update"],
            json!({"E1": {"locations": {"a": {"@type": "Location", "name": "Room 2"}, "b": {"@type": "Location", "name": "Car park"}}}})
        );
        // Taken out: the other stays; with none left, none.
        let set = sent(vec![changed(&two, |e| e.location.clear())], Vec::new(), vec![two.clone()]);
        assert_eq!(set["update"], json!({"E1": {"locations": {"b": {"@type": "Location", "name": "Car park"}}}}));
        let mut one = theirs("E2", "Lone");
        one["locations"] = json!({"a": {"@type": "Location", "name": "Room 1"}});
        let set = sent(vec![changed(&one, |e| e.location.clear())], Vec::new(), vec![one.clone()]);
        assert_eq!(set["update"], json!({"E2": {"locations": null}}));
        // None there yet: one, under a key of its own.
        let bare = theirs("E3", "Bare");
        let set = sent(vec![changed(&bare, |e| e.location = "Hall".into())], Vec::new(), vec![bare.clone()]);
        assert_eq!(set["update"], json!({"E3": {"locations": {"l1": {"@type": "Location", "name": "Hall"}}}}));
        // The guests of the person's meeting: one added, one taken out; the organizer and the others as the server has them.
        let mut meeting = theirs("E4", "Review");
        meeting["organizerCalendarAddress"] = json!("mailto:me@example.ch");
        meeting["participants"] = json!({
            "o": {"@type": "Participant", "sendTo": {"imip": "mailto:me@example.ch"}, "roles": {"owner": true, "attendee": true}, "participationStatus": "accepted"},
            "a": {"@type": "Participant", "sendTo": {"imip": "mailto:ana@example.ch"}, "roles": {"attendee": true}, "participationStatus": "accepted"},
            "r": {"@type": "Participant", "sendTo": {"imip": "mailto:room@example.ch"}, "roles": {"attendee": true}, "kind": "location"}
        });
        let regrouped = changed(&meeting, |e| {
            e.attendees.retain(|a| a.email != "ana@example.ch");
            e.attendees.push(Attendee { email: "bob@example.ch".into(), name: "Bob Keller".into(), partstat: "NEEDS-ACTION".into(), rsvp: true });
        });
        let set = sent(vec![regrouped], Vec::new(), vec![meeting.clone()]);
        assert_eq!(
            set["update"],
            json!({"E4": {"participants": {
                "o": meeting["participants"]["o"],
                "r": meeting["participants"]["r"],
                "p1": {"@type": "Participant", "email": "bob@example.ch", "name": "Bob Keller", "sendTo": {"imip": "mailto:bob@example.ch"},
                       "roles": {"attendee": true}, "participationStatus": "needs-action", "expectReply": true}
            }}})
        );
        assert_eq!(set["sendSchedulingMessages"], true, "the server tells them");
        // The last guest taken out: the server tells them too.
        let mut duo = theirs("E5", "One to one");
        duo["organizerCalendarAddress"] = json!("mailto:me@example.ch");
        duo["participants"] = json!({
            "o": {"@type": "Participant", "sendTo": {"imip": "mailto:me@example.ch"}, "roles": {"owner": true, "attendee": true}},
            "a": {"@type": "Participant", "sendTo": {"imip": "mailto:ana@example.ch"}, "roles": {"attendee": true}}
        });
        let alone = changed(&duo, |e| e.attendees.retain(|a| a.email != "ana@example.ch"));
        let set = sent(vec![alone], Vec::new(), vec![duo.clone()]);
        assert_eq!(set["update"]["E5"]["participants"], json!({"o": duo["participants"]["o"]}));
        assert_eq!(set["sendSchedulingMessages"], true, "the guest taken out is told");
    }

    #[test]
    fn participants_are_known_by_their_calendar_address_as_the_server_keeps_them() {
        // The Suite keeps each participant by its calendar address, and one asked for no answer (the organizer, a guest
        // whose answer it doesn't expect) without `sendTo` (suite's calendar/jmap_events.py, `participants_map`).
        let mut meeting = theirs("E1", "Review");
        meeting["organizerCalendarAddress"] = json!("mailto:boss@example.ch");
        meeting["participants"] = json!({
            "b": {"@type": "Participant", "calendarAddress": "mailto:boss@example.ch", "sendTo": null, "roles": {"owner": true}, "expectReply": false},
            "m": {"@type": "Participant", "calendarAddress": "mailto:me@example.ch", "sendTo": {"imip": "mailto:me@example.ch"},
                  "roles": {"attendee": true}, "participationStatus": "needs-action", "expectReply": true},
            "a": {"@type": "Participant", "calendarAddress": "mailto:ana@example.ch", "sendTo": null, "roles": {"attendee": true}, "expectReply": false}
        });
        let read = event_of(&meeting).unwrap();
        let mut seen: Vec<&str> = read.attendees.iter().map(|a| a.email.as_str()).collect();
        seen.sort_unstable();
        assert_eq!(seen, ["ana@example.ch", "me@example.ch"], "every guest, by its calendar address");
        // The person's answer, on their own entry; nobody added nor lost.
        let answered = changed(&meeting, |e| {
            for a in e.attendees.iter_mut().filter(|a| a.email == "me@example.ch") {
                a.partstat = "ACCEPTED".into();
            }
        });
        let set = sent(vec![answered], Vec::new(), vec![meeting.clone()]);
        let mut want = meeting["participants"].clone();
        want["m"]["participationStatus"] = json!("accepted");
        assert_eq!(set["update"]["E1"]["participants"], want);
        // The person's own meeting, a guest known by its calendar address only: not invited twice when another is added.
        let mut own = meeting.clone();
        own["organizerCalendarAddress"] = json!("mailto:me@example.ch");
        own["participants"]["b"]["roles"] = json!({"attendee": true});
        own["participants"]["m"]["roles"] = json!({"owner": true, "attendee": true});
        let more = changed(&own, |e| e.attendees.push(Attendee { email: "bob@example.ch".into(), ..Default::default() }));
        let set = sent(vec![more], Vec::new(), vec![own.clone()]);
        let keys: Vec<&String> = set["update"]["E1"]["participants"].as_object().unwrap().keys().collect();
        assert_eq!(keys, ["a", "b", "m", "p1"], "Bob added, nobody twice");
    }

    #[test]
    fn what_goes_is_what_the_person_changed_over_what_someone_else_changed_since() {
        // Read at 09:00 (Zurich); moved to 11:00 in the desk since; the person then fixes its title here: only the title
        // goes, the time stays as the desk moved it (and the guests are not told of the old one again).
        let read = theirs("E1", "Site visit");
        let mut moved = read.clone();
        (moved["start"], moved["utcStart"], moved["utcEnd"]) =
            (json!("2026-10-19T11:00:00"), json!("2026-10-19T09:00:00Z"), json!("2026-10-19T10:00:00Z"));
        let fixed = changed(&read, |e| e.summary = "Site visit, fixed".into());
        let set = sent(vec![fixed], vec![("jmap:E1", read)], vec![moved]);
        assert_eq!(set["update"], json!({"E1": {"title": "Site visit, fixed"}}));
        // A guest added in the desk since it was read; the person then gives it a place here: the guest stays.
        let mut read = theirs("E2", "Review");
        read["organizerCalendarAddress"] = json!("mailto:me@example.ch");
        read["participants"] = json!({
            "o": {"@type": "Participant", "sendTo": {"imip": "mailto:me@example.ch"}, "roles": {"owner": true, "attendee": true}},
            "a": {"@type": "Participant", "sendTo": {"imip": "mailto:ana@example.ch"}, "roles": {"attendee": true}}
        });
        let mut added = read.clone();
        added["participants"]["b"] = json!({"@type": "Participant", "sendTo": {"imip": "mailto:bob@example.ch"}, "roles": {"attendee": true}});
        let placed = changed(&read, |e| e.location = "Room 2".into());
        let set = sent(vec![placed], vec![("jmap:E2", read)], vec![added]);
        assert_eq!(set["update"], json!({"E2": {"locations": {"l1": {"@type": "Location", "name": "Room 2"}}}}), "the guests as the server has them");
        assert_eq!(set["sendSchedulingMessages"], true, "they hear of the new place");
        // The whole series, changed from one of its occurrences: compared with that occurrence as the form showed it, not
        // as it is now (moved alone in the desk since), so that only the title goes, to the series.
        let mut base = theirs("B1", "Weekly");
        (base["start"], base["recurrenceRule"]) = (json!("2026-10-05T09:00:00"), json!({"@type": "RecurrenceRule", "frequency": "weekly"}));
        let mut read = theirs("E3", "Weekly");
        (read["recurrenceId"], read["baseEventId"]) = (json!("2026-10-19T09:00:00"), json!("B1"));
        let mut alone = read.clone();
        (alone["start"], alone["utcStart"], alone["utcEnd"]) =
            (json!("2026-10-19T10:00:00"), json!("2026-10-19T08:00:00Z"), json!("2026-10-19T09:00:00Z"));
        let renamed = changed(&read, |e| {
            e.summary = "Weekly, renamed".into();
            mailcraft_model::pim::set_series_line(e, SERIES_CHANGE_LINE, Some(SERIES_CHANGE));
        });
        let set = sent(vec![renamed], vec![("jmap:E3", read)], vec![base, alone]);
        assert_eq!(set["update"], json!({"B1": {"title": "Weekly, renamed"}}));
        // Changed back to what it was read as: nothing goes.
        let same = theirs("E4", "Same");
        let mut retitled = same.clone();
        retitled["title"] = json!("Retitled in the desk");
        let set = sent(vec![changed(&same, |_| {})], vec![("jmap:E4", same)], vec![retitled]);
        assert_eq!(set, Value::Null, "nothing the person changed");
    }

    #[test]
    fn a_day_or_more_across_a_change_of_offset_counts_its_days_on_the_wall_clock() {
        // From Saturday 10:00 in Zurich, over the nights the clocks go back (25 October 2026: a day of 25 hours) and forward
        // (29 March 2026: 23 hours): its days on the wall clock (`D`, as the server reads it: to the same time the next
        // day), the rest as the time that passes.
        for (start, end, want) in [
            ("2026-10-24T08:00:00Z", "2026-10-25T09:00:00Z", "P1D"),
            ("2026-10-24T08:00:00Z", "2026-10-25T10:00:00Z", "P1DT1H"),
            ("2026-10-24T08:00:00Z", "2026-10-25T08:00:00Z", "PT24H"),
            ("2026-03-28T09:00:00Z", "2026-03-29T08:00:00Z", "P1D"),
            ("2026-03-28T09:00:00Z", "2026-03-29T07:30:00Z", "PT22H30M"),
            ("2026-10-19T07:00:00Z", "2026-10-21T08:30:00Z", "P2DT1H30M"),
        ] {
            let mut new = here("", "Long", start, 0);
            new.event.end = at(end);
            let c = serve(500, keeping(Vec::new()));
            crate::block_on(c.sync_calendars(&jmap_pim(vec![new], Vec::new()), &writing(false), 1)).unwrap();
            assert_eq!(asked(&c, "CalendarEvent/set")[0]["create"]["k0"]["duration"], want, "{start} to {end}");
        }
        // Moved over the change, likewise; and an occurrence, in its own zone.
        let one = theirs("E1", "Site visit");
        let longer = changed(&one, |e| (e.start, e.end) = (at("2026-10-24T08:00:00Z"), at("2026-10-25T09:00:00Z")));
        assert_eq!(sent(vec![longer], Vec::new(), vec![one])["update"]["E1"]["duration"], "P1D");
        let mut occurrence = theirs("E2", "Weekly");
        (occurrence["recurrenceId"], occurrence["baseEventId"]) = (json!("2026-10-19T09:00:00"), json!("B1"));
        let longer = changed(&occurrence, |e| (e.start, e.end) = (at("2026-03-28T09:00:00Z"), at("2026-03-29T08:00:00Z")));
        assert_eq!(sent(vec![longer], Vec::new(), vec![occurrence])["update"]["E2"]["duration"], "P1D");
    }

    #[test]
    fn a_send_alone_reads_no_calendar() {
        let events = vec![here("jmap:E1", "Changed", "2026-10-19T07:00:00Z", 60)];
        let c = serve(500, keeping(vec![theirs("E1", "Before")]));
        let sync = crate::block_on(c.send_calendars(&jmap_pim(events, Vec::new()), &writing(false), 1)).unwrap().unwrap();
        assert_eq!(sync.pushed_events.len(), 1, "sent");
        assert!(sync.calendars.is_none() && sync.events.is_empty() && sync.synced_calendars.is_empty(), "the calendars stay as the engine has them");
        assert!(asked(&c, "CalendarEvent/query").is_empty() && asked(&c, "Calendar/get").is_empty(), "nothing read but the copy changed");
    }

    #[test]
    fn what_the_server_could_not_take_for_now_stays_to_be_sent() {
        // A server that fails for the time being on some events (`serverFail`, `serverUnavailable`: Neoffice's when the
        // mail server does not answer) keeps none of their changes: said, they stay to be sent at the next sync; a
        // definite refusal still goes, said in its words.
        let events = vec![
            here("", "New", "2026-10-19T07:00:00Z", 60),
            here("jmap:E1", "Changed", "2026-10-19T07:00:00Z", 60),
            here("jmap:E2", "Refused", "2026-10-19T07:00:00Z", 60),
        ];
        let gone = vec![Tombstone { href: "jmap:E3".into(), etag: String::new(), calendar: "K1".into() }];
        let failing = |name: &str, args: &Value| match name {
            "CalendarEvent/set" => json!({
                "notCreated": {"k0": {"type": "serverFail"}},
                "notUpdated": {"E1": {"type": "serverUnavailable"}, "E2": {"type": "invalidProperties", "description": "No such calendar"}},
                "notDestroyed": {"E3": {"type": "serverFail", "description": "The mail server did not answer"}}
            }),
            _ => keeping(vec![theirs("E1", "Before"), theirs("E2", "Before"), theirs("E3", "Old")])(name, args),
        };
        let c = serve(500, failing);
        let sync = crate::block_on(c.sync_calendars(&jmap_pim(events, gone), &writing(false), 1)).unwrap().unwrap();
        assert!(sync.pushed_events.is_empty(), "none stored");
        assert_eq!(sync.refused_events.iter().map(|e| e.event.summary.as_str()).collect::<Vec<_>>(), vec!["Refused"], "only the refusal goes");
        assert!(sync.deleted.is_empty(), "the deletion stays to be sent");
        assert_eq!(
            sync.errors,
            vec![
                "an event was not saved: New — the server failed; tried again at the next sync",
                "an event was not saved: Changed — the server failed; tried again at the next sync",
                "an event was not saved: Refused — refused by the server: No such calendar",
                "an event was not deleted: the server failed; tried again at the next sync",
            ]
        );
    }

    #[test]
    fn at_most_fifty_events_a_call_or_what_the_server_takes() {
        let many: Vec<StoredEvent> = (0..120).map(|n| here("", &format!("E{n}"), "2026-10-19T07:00:00Z", 60)).collect();
        let c = serve(500, keeping(Vec::new()));
        let sync = crate::block_on(c.sync_calendars(&jmap_pim(many.clone(), Vec::new()), &writing(false), 1)).unwrap().unwrap();
        let sizes: Vec<usize> = asked(&c, "CalendarEvent/set").iter().map(|s| s["create"].as_object().map_or(0, Map::len)).collect();
        assert_eq!(sizes, vec![50, 50, 20]);
        assert_eq!(sync.pushed_events.len(), 120);
        let c = serve(30, keeping(Vec::new()));
        crate::block_on(c.sync_calendars(&jmap_pim(many, Vec::new()), &writing(false), 1)).unwrap();
        assert_eq!(asked(&c, "CalendarEvent/set").len(), 4, "the server's own smaller limit");
    }

    #[test]
    fn a_refusal_is_said_once_and_the_change_goes_and_a_wait_is_quiet() {
        let events = vec![
            here("", "Refused", "2026-10-19T07:00:00Z", 60),
            here("", "Later", "2026-10-19T07:00:00Z", 60),
            here("", "Again", "2026-10-19T07:00:00Z", 60),
        ];
        let answering = |name: &str, args: &Value| match name {
            "CalendarEvent/set" => json!({
                "notCreated": {
                    "k0": {"type": "invalidProperties", "description": "No such calendar"},
                    "k1": {"type": "rateLimit"},
                    "k2": {"type": "alreadyExists", "existingId": "E7"}
                }
            }),
            _ => keeping(Vec::new())(name, args),
        };
        let c = serve(500, answering);
        let sync = crate::block_on(c.sync_calendars(&jmap_pim(events.clone(), Vec::new()), &writing(false), 1)).unwrap().unwrap();
        assert_eq!(sync.errors, vec!["an event was not saved: Refused — refused by the server: No such calendar"]);
        assert_eq!(sync.refused_events.iter().map(|e| e.event.summary.as_str()).collect::<Vec<_>>(), vec!["Refused"], "its change goes");
        assert_eq!(
            sync.pushed_events.iter().map(|e| (e.event.summary.as_str(), e.href.as_str())).collect::<Vec<_>>(),
            vec![("Again", "jmap:E7")],
            "made before: that one"
        );
        // The server asks to wait for the whole call, or the call fails: nothing refused; said only for the failure.
        let waiting = |name: &str, args: &Value| {
            if name == "CalendarEvent/set" { json!({"error": {"type": "rateLimit"}}) } else { keeping(Vec::new())(name, args) }
        };
        let c = serve(500, waiting);
        let sync = crate::block_on(c.sync_calendars(&jmap_pim(events.clone(), Vec::new()), &writing(false), 1)).unwrap().unwrap();
        assert!(sync.errors.is_empty() && sync.refused_events.is_empty() && sync.pushed_events.is_empty());
        // And a failed push doesn't stop the read.
        let failing =
            |name: &str, args: &Value| if name == "CalendarEvent/set" { json!({"transport": "reset"}) } else { keeping(Vec::new())(name, args) };
        let c = serve(500, failing);
        let sync = crate::block_on(c.sync_calendars(&jmap_pim(events, Vec::new()), &writing(false), 1)).unwrap().unwrap();
        assert_eq!((sync.errors.len(), sync.refused_events.len()), (3, 0), "{:?}", sync.errors);
        assert_eq!(sync.calendars.as_ref().map(Vec::len), Some(1), "read all the same");
    }

    #[test]
    fn a_token_refused_during_the_calendars_sync_is_said_as_such() {
        let refusing =
            |name: &str, args: &Value| if name == "CalendarEvent/query" { json!({"status": 401}) } else { keeping(Vec::new())(name, args) };
        let c = serve(500, refusing);
        let r = crate::block_on(c.sync_calendars(&Pim::default(), &writing(false), 1));
        assert!(matches!(r, Err(crate::Error::Auth(_))), "{r:?}");
        let refusing = |name: &str, args: &Value| if name == "CalendarEvent/set" { json!({"status": 401}) } else { keeping(Vec::new())(name, args) };
        let c = serve(500, refusing);
        let r = crate::block_on(c.sync_calendars(&jmap_pim(vec![here("", "New", "2026-10-19T07:00:00Z", 60)], Vec::new()), &writing(false), 1));
        assert!(matches!(r, Err(crate::Error::Auth(_))), "{r:?}");
    }

    #[test]
    fn what_writing_back_needs_is_read_and_keywords_only_where_kept() {
        // The calendars the person may write events in, and what an event is read with (Neoffice keeps no keywords).
        let sync = calendars_sync(
            &[
                json!({"id": "K1", "myRights": {"mayWriteAll": true}}),
                json!({"id": "K2", "myRights": {"mayWriteOwn": true}}),
                json!({"id": "K3", "myRights": {"mayReadItems": true}}),
            ],
            &[],
            1,
        );
        let writable: Vec<bool> = sync.calendars.iter().flatten().map(|c| !c.read_only).collect();
        assert_eq!(writable, vec![true, true, false]);
        let probe = |name: &str, args: &Value| match name {
            "CalendarEvent/query" => json!({"ids": ["E1"], "total": 1}),
            _ => keeping(vec![theirs("E1", "One")])(name, args),
        };
        for kept in [false, true] {
            let c = serve(500, probe);
            crate::block_on(c.sync_calendars(&Pim::default(), &writing(kept), 1)).unwrap();
            let props = asked(&c, "CalendarEvent/get")[0]["properties"].clone();
            for needed in ["baseEventId", "timeZone", "recurrenceRule", "organizerCalendarAddress", "isDraft", "participants", "recurrenceId"] {
                assert!(props.as_array().unwrap().contains(&json!(needed)), "{needed}");
            }
            assert_eq!(props.as_array().unwrap().contains(&json!("keywords")), kept, "keywords only where the server keeps them");
        }
    }

    #[test]
    fn rules_read_and_written_as_icalendar_s() {
        let utc = |t: i64| Some(t);
        let r = rule_json("FREQ=MONTHLY;INTERVAL=2;BYDAY=-1FR;COUNT=6;WKST=SU", &utc).unwrap();
        assert_eq!(
            r,
            json!({"@type": "RecurrenceRule", "frequency": "monthly", "interval": 2, "byDay": [{"@type": "NDay", "day": "fr", "nthOfPeriod": -1}], "count": 6, "firstDayOfWeek": "su"})
        );
        assert_eq!(rule_text(&r).as_deref(), Some("FREQ=MONTHLY;INTERVAL=2;COUNT=6;BYDAY=-1FR;WKST=SU"));
        assert_eq!(
            rule_text(&json!({"frequency": "weekly", "interval": 1, "firstDayOfWeek": "mo"})).as_deref(),
            Some("FREQ=WEEKLY"),
            "the defaults left out"
        );
        assert_eq!(rule_json("FREQ=DAILY;UNTIL=20261231", &utc).unwrap()["until"], "2026-12-31T23:59:59", "a date's whole day");
        assert_eq!(rule_json("BYDAY=MO", &utc), None, "no frequency, no rule");
        let utc = |t: i64| Some(t);
        assert_eq!(wall_duration(0, 5400, &utc), "PT1H30M");
        assert_eq!(wall_duration(0, 26 * 3600, &utc), "P1DT2H");
        assert_eq!(wall_duration(0, 0, &utc), "PT0S");
        assert_eq!(wall_duration(0, 25 * 3600, &|_| None), "PT25H", "a zone it doesn't know: the time that passes");
    }

    fn meeting() -> Value {
        json!({
            "@type": "Event", "id": "E1", "uid": "f00d", "calendarIds": {"K1": true},
            "title": "Site meeting, weekly", "description": "With the customer.",
            "locations": {"l1": {"@type": "Location", "name": "Office"}},
            "start": "2026-10-19T09:00:00", "timeZone": "Europe/Zurich", "duration": "PT1H30M",
            "utcStart": "2026-10-19T07:00:00Z", "utcEnd": "2026-10-19T08:30:00Z",
            "recurrenceId": "2026-10-19T09:00:00", "status": "confirmed"
        })
    }

    #[test]
    fn an_occurrence_reads_with_its_times_in_utc() {
        let e = event_of(&meeting()).unwrap();
        assert_eq!((e.uid.as_str(), e.summary.as_str(), e.location.as_str()), ("f00d", "Site meeting, weekly", "Office"));
        assert_eq!((e.start, e.end), (utc_date("2026-10-19T07:00:00Z").unwrap(), utc_date("2026-10-19T08:30:00Z").unwrap()));
        assert_eq!(e.recurrence_id, Some(e.start), "an occurrence stands apart by its start");
        assert_eq!(e.status, "CONFIRMED");
        // Without utcEnd, the duration gives the end; a one-off has no recurrence id.
        let mut one = meeting();
        one.as_object_mut().unwrap().remove("utcEnd");
        one["recurrenceId"] = Value::Null;
        let e = event_of(&one).unwrap();
        assert_eq!((e.end - e.start, e.recurrence_id), (5400, None));
    }

    #[test]
    fn its_keywords_are_its_categories() {
        let mut e = meeting();
        assert!(event_of(&e).unwrap().categories.is_empty(), "none given");
        e["keywords"] = json!({"Work": true, " Clients ": true, "": true, "Off": false, "Clients": true});
        let mut got = event_of(&e).unwrap().categories;
        got.sort();
        assert_eq!(got, ["Clients", "Work"], "the ones set, trimmed, once each");
        e["keywords"] = json!((0..30).map(|i| (format!("k{i}"), json!(true))).collect::<serde_json::Map<_, _>>());
        assert_eq!(event_of(&e).unwrap().categories.len(), 20, "as many as an iCalendar event keeps");
        e["keywords"] = json!(["Work"]);
        assert!(event_of(&e).unwrap().categories.is_empty(), "not a map: none");
    }

    #[test]
    fn a_day_long_event_keeps_its_days() {
        let day = json!({"id": "E2", "uid": "d1", "calendarIds": {"K1": true}, "title": "Closed", "start": "2026-12-24T00:00:00", "duration": "P2D", "showWithoutTime": true});
        let e = event_of(&day).unwrap();
        assert!(e.all_day);
        assert_eq!((e.start, e.end), (utc_date("2026-12-24T00:00:00Z").unwrap(), utc_date("2026-12-26T00:00:00Z").unwrap()));
        assert_eq!(event_of(&json!({"id": "E3", "title": "No start"})), None);
    }

    #[test]
    fn the_calendars_and_their_events_come_back_as_a_sync_to_merge() {
        let calendars = vec![json!({"id": "K1", "name": "Agenda"}), json!({"id": "K2", "name": ""}), json!({"name": "no id"})];
        let mut elsewhere = meeting();
        elsewhere["id"] = json!("E9");
        elsewhere["calendarIds"] = json!({"K9": true});
        let mut in_two = meeting();
        in_two["id"] = json!("E5");
        in_two["calendarIds"] = json!({"K2": true, "K1": true});
        let sync = calendars_sync(&calendars, &[meeting(), elsewhere, in_two], 5);
        let listed: Vec<(&str, &str, bool)> = sync.calendars.iter().flatten().map(|c| (c.id.as_str(), c.name.as_str(), c.read_only)).collect();
        assert_eq!(listed, vec![("K1", "Agenda", true), ("K2", "Calendar", true)]);
        assert_eq!(sync.synced_calendars, vec!["K1", "K2"]);
        let placed: Vec<(&str, &str)> = sync.events.iter().map(|e| (e.calendar.as_str(), e.href.as_str())).collect();
        assert_eq!(
            placed,
            vec![("K1", "jmap:E1"), ("K1", "jmap:E5")],
            "an event of an unknown calendar is left out; one in two calendars is placed once, in the first the server lists"
        );
        assert_eq!((sync.caldav_home.as_deref(), sync.carddav_home.as_deref()), (Some("jmap"), None));
        assert_eq!(utc_string(utc_date("2026-10-17T09:30:05Z").unwrap()), "2026-10-17T09:30:05Z");
    }

    #[test]
    fn contacts_and_calendars_make_one_sync() {
        let contacts = DavSync { carddav_home: Some("jmap".into()), books: Some(Vec::new()), ..Default::default() };
        let calendars = DavSync { caldav_home: Some("jmap".into()), calendars: Some(Vec::new()), ..Default::default() };
        let both = pim_sync(Some(contacts.clone()), Some(calendars.clone()), (true, true), 3).unwrap();
        assert_eq!((both.carddav_home.as_deref(), both.caldav_home.as_deref(), both.checked), (Some("jmap"), Some("jmap"), 3));
        assert!(both.books.is_some() && both.calendars.is_some());
        let only = pim_sync(Some(contacts.clone()), None, (true, true), 3).unwrap();
        assert!(only.calendars.is_none(), "calendars not read are left as they are");
        assert_eq!(only.caldav_home.as_deref(), Some("jmap"), "offered, only not read this time: still there");
        assert_eq!(pim_sync(Some(contacts.clone()), None, (true, false), 3).unwrap().caldav_home, None, "not offered: none");
        let unread = pim_sync(None, Some(calendars), (true, true), 3).unwrap();
        assert_eq!((unread.carddav_home.as_deref(), unread.books.is_none(), unread.synced_books.is_empty()), (Some("jmap"), true, true));
        assert_eq!(pim_sync(None, None, (true, true), 3), None);
        // What the contacts' part stored, let go of and could not send comes along.
        let pushed = mailcraft_model::pim::StoredCard { book: "B1".into(), href: "jmap:N1".into(), ..Default::default() };
        let written = DavSync {
            pushed_cards: vec![pushed],
            deleted: vec!["jmap:C9".into()],
            errors: vec!["Jeanne: refused".into()],
            partial_books: vec!["B1".into()],
            no_delete_books: vec!["B1".into()],
            ..contacts
        };
        let merged = pim_sync(Some(written), None, (true, false), 3).unwrap();
        assert_eq!(merged.partial_books, vec!["B1"], "a book read in part stays so");
        assert_eq!(merged.no_delete_books, vec!["B1"], "and whose cards may not be deleted");
        let read = DavSync { caldav_home: Some("jmap".into()), calendars: Some(Vec::new()), window: Some((1, 2)), ..Default::default() };
        assert_eq!(pim_sync(None, Some(read), (false, true), 3).unwrap().window, Some((1, 2)), "the span the calendars were read for");
        assert_eq!(
            (merged.pushed_cards.len(), merged.deleted.as_slice(), merged.errors.as_slice()),
            (1, &["jmap:C9".to_string()][..], &["Jeanne: refused".to_string()][..])
        );
    }

    /// A canned server: answers in order, and keeps what was asked.
    struct Canned(std::cell::RefCell<Vec<Value>>, std::cell::RefCell<Vec<Value>>);

    impl crate::Http for Canned {
        async fn send(&self, req: crate::Request) -> crate::Result<crate::Response> {
            if let Ok(asked) = serde_json::from_slice::<Value>(&req.body) {
                self.1.borrow_mut().push(asked);
            }
            let mut answers = self.0.borrow_mut();
            if answers.is_empty() {
                return Err(crate::Error::Io("no more answers".into()));
            }
            Ok(crate::Response { status: 200, body: answers.remove(0).to_string().into_bytes() })
        }
    }

    fn session() -> Value {
        json!({
            "capabilities": {"urn:ietf:params:jmap:core": {"maxObjectsInGet": 1000}, "urn:ietf:params:jmap:mail": {}, CALENDARS: {}},
            "primaryAccounts": {"urn:ietf:params:jmap:mail": "A1"},
            "apiUrl": "/api/", "downloadUrl": "/d/{accountId}/{blobId}", "uploadUrl": "/u/{accountId}/",
        })
    }

    fn client(answers: Vec<Value>) -> Client<Canned> {
        crate::block_on(Client::connect(Canned(answers.into(), Vec::new().into()), "jmap.example.com", "me", "pw", true)).unwrap()
    }

    #[test]
    fn the_events_are_read_page_after_page() {
        let mut next = meeting();
        next["id"] = json!("E2");
        next["recurrenceId"] = json!("2026-10-26T09:00:00");
        next["utcStart"] = json!("2026-10-26T08:00:00Z");
        next["utcEnd"] = json!("2026-10-26T09:30:00Z");
        let c = client(vec![
            session(),
            json!({"methodResponses": [
                ["Calendar/get", {"list": [{"id": "K1", "name": "Agenda"}]}, "c0"],
                ["CalendarEvent/query", {"ids": ["E1"], "position": 0, "limit": 1, "total": 2}, "c1"]
            ]}),
            json!({"methodResponses": [["CalendarEvent/query", {"ids": ["E2"], "position": 1, "total": 2}, "c0"]]}),
            json!({"methodResponses": [["CalendarEvent/get", {"list": [meeting(), next]}, "c0"]]}),
        ]);
        let sync = crate::block_on(c.sync_calendars(&Pim::default(), &writing(false), utc_date("2026-10-17T09:00:00Z").unwrap())).unwrap().unwrap();
        assert_eq!(sync.events.len(), 2, "both pages' occurrences");
        assert_eq!(sync.synced_calendars, vec!["K1"]);
        let asked = c.http.1.borrow();
        let first = &asked[0]["methodCalls"][1][1];
        assert_eq!((&first["position"], &first["limit"], &first["calculateTotal"]), (&json!(0), &json!(MAX_EVENTS), &json!(true)));
        let page = &asked[1]["methodCalls"][0][1];
        assert_eq!((&page["position"], &page["limit"]), (&json!(1), &json!(1)), "on from the first page, by the server's limit");
        assert_eq!((&page["filter"], &page["expandRecurrences"]), (&first["filter"], &json!(true)), "the same window, each occurrence apart");
    }

    #[test]
    fn calendars_not_read_whole_are_handed_over_as_read_in_part() {
        // More occurrences than one sync reads (a server that gives more than it was asked for): the calendars stay
        // listed, as read in part, which the engine merges without removing anything.
        let many: Vec<String> = (0..=MAX_EVENTS).map(|n| format!("E{n}")).collect();
        let mut answers = vec![
            session(),
            json!({"methodResponses": [["Calendar/get", {"list": [{"id": "K1", "name": "Agenda"}]}, "c0"], ["CalendarEvent/query", {"ids": many}, "c1"]]}),
        ];
        answers.extend((0..MAX_EVENTS / 1000).map(|_| json!({"methodResponses": [["CalendarEvent/get", {"list": []}, "c0"]]})));
        let c = client(answers);
        let sync = crate::block_on(c.sync_calendars(&Pim::default(), &writing(false), utc_date("2026-10-17T09:00:00Z").unwrap())).unwrap().unwrap();
        assert!(sync.synced_calendars.is_empty());
        assert_eq!(sync.partial_calendars, vec!["K1"]);
        assert_eq!(sync.calendars.as_ref().map(Vec::len), Some(1));
        assert_eq!(c.http.0.borrow().len(), 0, "the cap's occurrences read, no more");
    }

    #[test]
    fn the_window_fits_the_span_the_server_expands() {
        let now = utc_date("2026-10-17T09:00:00Z").unwrap();
        assert_eq!(window(now, None), (now - DAYS_BACK * 86_400, now + DAYS_AHEAD * 86_400));
        assert_eq!(window(now, Some(500 * 86_400)), window(now, None), "a server that allows more: the usual window");
        let (from, to) = window(now, Some(100 * 86_400));
        assert_eq!(to - from, 100 * 86_400, "within what the server expands");
        assert!(from < now && to > now && now - from < to - now, "more of it ahead than back, as usual");
        // Read from the calendars' account capabilities.
        let mut narrow = session();
        narrow["accounts"] = json!({"A1": {"accountCapabilities": {CALENDARS: {"maxExpandedQueryDuration": "P100D"}}}});
        let c = client(vec![
            narrow,
            json!({"methodResponses": [["Calendar/get", {"list": [{"id": "K1", "name": "Agenda"}]}, "c0"], ["CalendarEvent/query", {"ids": [], "total": 0}, "c1"]]}),
        ]);
        let sync = crate::block_on(c.sync_calendars(&Pim::default(), &writing(false), now)).unwrap().unwrap();
        let asked = c.http.1.borrow();
        let filter = &asked[0]["methodCalls"][1][1]["filter"];
        let span = utc_date(filter["before"].as_str().unwrap()).unwrap() - utc_date(filter["after"].as_str().unwrap()).unwrap();
        assert_eq!(span, 100 * 86_400);
        assert_eq!(sync.window, Some(window(now, Some(100 * 86_400))), "handed over with the sync");
    }
}
