// //// Neoffice — added file (no upstream equivalent): JMAP for Calendars (draft-ietf-jmap-calendars), read by the same
// client as the mail.
//
// A server that offers `urn:ietf:params:jmap:calendars` (Neoffice's mail facade does, for the person's calendars) gives
// its calendars and the events of a window around today, each occurrence of a repeating event apart, with its start
// and end in UTC (`utcStart`, `utcEnd`, which the server works out): MailCraft has no time-zone database, and the
// browser build has no CalDAV. They come back as a [`DavSync`], which the engine merges as a CalDAV sync's, so the
// calendar, the day's agenda and the meeting answers read them unchanged. Read only for now.

use mailcraft_model::dates::civil_from_days;
use mailcraft_model::ical::{CalEvent, duration};
use mailcraft_model::pim::{CalendarInfo, DavSync, StoredEvent};
use serde_json::{Value, json};

use crate::{CORE, Client, Http, Result, list, utc_date};

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
        let window = json!({"after": utc_string(now - DAYS_BACK * 86_400), "before": utc_string(now + DAYS_AHEAD * 86_400)});
        let r = self
            .call(
                &[CORE, CALENDARS],
                vec![
                    ("Calendar/get", json!({"accountId": account, "ids": null})),
                    ("CalendarEvent/query", json!({"accountId": account, "filter": window, "expandRecurrences": true, "limit": MAX_EVENTS})),
                ],
            )
            .await?;
        let calendars = list(r.first(), "list").to_vec();
        let ids: Vec<&str> = list(r.get(1), "ids").iter().filter_map(Value::as_str).take(MAX_EVENTS).collect();
        let mut events = Vec::with_capacity(ids.len());
        for chunk in ids.chunks(self.max_get.max(1)) {
            let got = self
                .call(&[CORE, CALENDARS], vec![("CalendarEvent/get", json!({"accountId": account, "ids": chunk, "properties": PROPERTIES}))])
                .await?;
            events.extend(list(got.first(), "list").iter().cloned());
        }
        Ok(Some(calendars_sync(&calendars, &events, now)))
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
        for calendar in v.get("calendarIds").and_then(Value::as_object).into_iter().flat_map(|m| m.keys()) {
            if calendars.iter().any(|c| &c.id == calendar) {
                stored.push(StoredEvent {
                    calendar: calendar.clone(),
                    href: format!("{HREF_PREFIX}{id}"),
                    etag: String::new(),
                    event: event.clone(),
                    dirty: false,
                });
            }
        }
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
        ..Default::default()
    })
}

/// One sync of what the server offers: its contacts and its calendars together, so that neither of the two homes of
/// the account is said to be gone when only the other was read.
pub fn pim_sync(contacts: Option<DavSync>, calendars: Option<DavSync>, now: i64) -> Option<DavSync> {
    if contacts.is_none() && calendars.is_none() {
        return None;
    }
    let mut out = DavSync { checked: now, ..Default::default() };
    if let Some(c) = contacts {
        out.carddav_home = c.carddav_home;
        out.books = c.books;
        out.cards = c.cards;
        out.synced_books = c.synced_books;
    }
    if let Some(c) = calendars {
        out.caldav_home = c.caldav_home;
        out.calendars = c.calendars;
        out.events = c.events;
        out.synced_calendars = c.synced_calendars;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meeting() -> Value {
        json!({
            "@type": "Event", "id": "E1", "uid": "f00d", "calendarIds": {"K1": true},
            "title": "Chantier, point hebdo", "description": "Avec le client.",
            "locations": {"l1": {"@type": "Location", "name": "Bureau"}},
            "start": "2026-10-19T09:00:00", "timeZone": "Europe/Zurich", "duration": "PT1H30M",
            "utcStart": "2026-10-19T07:00:00Z", "utcEnd": "2026-10-19T08:30:00Z",
            "recurrenceId": "2026-10-19T09:00:00", "status": "confirmed"
        })
    }

    #[test]
    fn an_occurrence_reads_with_its_times_in_utc() {
        let e = event_of(&meeting()).unwrap();
        assert_eq!((e.uid.as_str(), e.summary.as_str(), e.location.as_str()), ("f00d", "Chantier, point hebdo", "Bureau"));
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
    fn a_day_long_event_keeps_its_days() {
        let day = json!({"id": "E2", "uid": "d1", "calendarIds": {"K1": true}, "title": "Fermeture", "start": "2026-12-24T00:00:00", "duration": "P2D", "showWithoutTime": true});
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
        let sync = calendars_sync(&calendars, &[meeting(), elsewhere], 5);
        let listed: Vec<(&str, &str, bool)> = sync.calendars.iter().flatten().map(|c| (c.id.as_str(), c.name.as_str(), c.read_only)).collect();
        assert_eq!(listed, vec![("K1", "Agenda", true), ("K2", "Calendar", true)]);
        assert_eq!(sync.synced_calendars, vec!["K1", "K2"]);
        let placed: Vec<(&str, &str)> = sync.events.iter().map(|e| (e.calendar.as_str(), e.href.as_str())).collect();
        assert_eq!(placed, vec![("K1", "jmap:E1")], "an event of an unknown calendar is left out");
        assert_eq!((sync.caldav_home.as_deref(), sync.carddav_home.as_deref()), (Some("jmap"), None));
        assert_eq!(utc_string(utc_date("2026-10-17T09:30:05Z").unwrap()), "2026-10-17T09:30:05Z");
    }

    #[test]
    fn contacts_and_calendars_make_one_sync() {
        let contacts = DavSync { carddav_home: Some("jmap".into()), books: Some(Vec::new()), ..Default::default() };
        let calendars = DavSync { caldav_home: Some("jmap".into()), calendars: Some(Vec::new()), ..Default::default() };
        let both = pim_sync(Some(contacts.clone()), Some(calendars), 3).unwrap();
        assert_eq!((both.carddav_home.as_deref(), both.caldav_home.as_deref(), both.checked), (Some("jmap"), Some("jmap"), 3));
        assert!(both.books.is_some() && both.calendars.is_some());
        let only = pim_sync(Some(contacts), None, 3).unwrap();
        assert!(only.calendars.is_none(), "calendars not read are left as they are");
        assert_eq!(pim_sync(None, None, 3), None);
    }
}
