// //// Neoffice — added file (no upstream equivalent): JMAP for Calendars (draft-ietf-jmap-calendars), read by the same
// client as the mail.
//
// A server that offers `urn:ietf:params:jmap:calendars` (Neoffice's mail facade does, for the person's calendars) gives
// its calendars and the events of a window around today, page after page, each occurrence of a repeating event apart,
// with its start and end in UTC (`utcStart`, `utcEnd`, which the server works out): MailCraft has no time-zone
// database, and the browser build has no CalDAV. They come back as a [`DavSync`], which the engine merges as a CalDAV
// sync's, so the calendar, the day's agenda and the meeting answers read them unchanged. Read only for now.

use mailcraft_model::dates::civil_from_days;
use mailcraft_model::ical::{CalEvent, duration};
use mailcraft_model::pim::{CalendarInfo, DavSync, StoredEvent};
use serde_json::{Value, json};

use crate::{CORE, Client, Http, Result, list, query_page, utc_date};

/// The capability, and the type of the account that holds the calendars.
pub const CALENDARS: &str = "urn:ietf:params:jmap:calendars";
/// The window read: this many days back, and ahead.
const DAYS_BACK: i64 = 90;
const DAYS_AHEAD: i64 = 366;
/// The most occurrences read in one sync.
const MAX_EVENTS: usize = 5000;
/// What an event is read with: what [`CalEvent`] shows.
const PROPERTIES: [&str; 13] = [
    "id",
    "uid",
    "calendarIds",
    "title",
    "description",
    "locations",
    "start",
    "duration",
    "showWithoutTime",
    "utcStart",
    "utcEnd",
    "recurrenceId",
    "status",
];
/// An event's address on this account (its id on the server), apart from those of CalDAV.
pub const HREF_PREFIX: &str = "jmap:";

impl<H: Http> Client<H> {
    /// Whether the session offers calendars.
    pub fn offers_calendars(&self) -> bool {
        self.calendars.is_some()
    }

    /// The calendars and the occurrences of their events in the window around `now`, as a sync to merge (`None` when
    /// the session offers none).
    pub async fn sync_calendars(&self, now: i64) -> Result<Option<DavSync>> {
        let Some(account) = self.calendars.as_deref() else { return Ok(None) };
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
                .call(&[CORE, CALENDARS], vec![("CalendarEvent/get", json!({"accountId": account, "ids": chunk, "properties": PROPERTIES}))])
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
        Ok(Some(sync))
    }
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

/// The sync the engine merges, from the server's calendars and events.
pub fn calendars_sync(calendars: &[Value], events: &[Value], now: i64) -> DavSync {
    let calendars: Vec<CalendarInfo> = calendars
        .iter()
        .enumerate()
        .filter_map(|(n, c)| {
            let id = c.get("id")?.as_str()?.to_string();
            let name = c.get("name").and_then(Value::as_str).filter(|n| !n.trim().is_empty()).unwrap_or("Calendar").to_string();
            Some(CalendarInfo { id, name, color: u8::try_from(n % 8).unwrap_or(0), read_only: true, ctag: String::new() })
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

/// A JSCalendar event (one occurrence, its times in UTC) as MailCraft shows it; `None` without a start.
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
    Some(CalEvent {
        uid,
        summary: text("title"),
        description: text("description"),
        location,
        start,
        end,
        all_day,
        recurrence_id: occurrence.then_some(start),
        status: text("status").to_ascii_uppercase(),
        categories,
        ..Default::default()
    })
}

/// One sync of what the server offers: its contacts and its calendars together, so that neither of the two homes of
/// the account is said to be gone when only the other was read (`offered`: whether the session offers contacts, and
/// calendars). What each part stored, let go of and could not send comes along.
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
        out.deleted.extend(c.deleted);
        out.errors.extend(c.errors);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let sync = crate::block_on(c.sync_calendars(utc_date("2026-10-17T09:00:00Z").unwrap())).unwrap().unwrap();
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
        let sync = crate::block_on(c.sync_calendars(utc_date("2026-10-17T09:00:00Z").unwrap())).unwrap().unwrap();
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
        let sync = crate::block_on(c.sync_calendars(now)).unwrap().unwrap();
        let asked = c.http.1.borrow();
        let filter = &asked[0]["methodCalls"][1][1]["filter"];
        let span = utc_date(filter["before"].as_str().unwrap()).unwrap() - utc_date(filter["after"].as_str().unwrap()).unwrap();
        assert_eq!(span, 100 * 86_400);
        assert_eq!(sync.window, Some(window(now, Some(100 * 86_400))), "handed over with the sync");
    }
}
