//! Calendar arithmetic for the message list: date groups (Today, Yesterday, Last week…) and the
//! short and long times shown for a message. Pure integer maths on Unix seconds plus the local UTC
//! offset, so it works the same everywhere, wasm included.

use serde::{Deserialize, Serialize};

/// "Now" and the local time zone, as the app reports them each frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Clock {
    /// Unix seconds, UTC.
    pub now: i64,
    /// Local offset from UTC in seconds (east positive).
    pub offset: i32,
    // //// Neoffice — the local zone's changes of offset around now (daylight saving), from the time-zone rules the app
    // has (the browser's, the system's), so that a time is shown, and a local time read, with the offset of its own date
    // rather than today's: a meeting at 10:00 shows 10:00 on both sides of a change. Without them, today's offset, as
    // before.
    #[serde(default)]
    pub zone: Zone,
}

// //// Neoffice — see `Clock::zone`.
/// The most changes of offset a [`Zone`] holds: the thirteen years the apps ask for (three back, ten ahead) of two a
/// year, with room.
pub const ZONE_CHANGES: usize = 32;

/// The local zone's offsets over a span of years (the apps ask for three years back and ten ahead): from each instant
/// (`at`, Unix seconds) the offset (seconds east of UTC) until the next. Outside the span, the offset of its nearest end:
/// one offset, without its changes. Built by [`Zone::from_offsets`] from the app's time-zone rules.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Zone {
    pub len: u8,
    pub at: [i64; ZONE_CHANGES],
    pub offset: [i32; ZONE_CHANGES],
}

impl Zone {
    /// The changes of `offset` (an instant's offset by the rules) from `from` to `to`, found a day at a time and then
    /// to the second; at most [`ZONE_CHANGES`].
    pub fn from_offsets(from: i64, to: i64, offset: impl Fn(i64) -> i32) -> Zone {
        let mut zone = Zone::default();
        let clamp = |o: i32| o.clamp(-14 * 3600, 14 * 3600);
        let push = |zone: &mut Zone, at: i64, o: i32| {
            let n = usize::from(zone.len);
            if let (Some(a), Some(off)) = (zone.at.get_mut(n), zone.offset.get_mut(n)) {
                (*a, *off) = (at, o);
                zone.len = zone.len.saturating_add(1);
            }
        };
        let (mut t, mut was) = (from, clamp(offset(from)));
        push(&mut zone, i64::MIN, was);
        while t < to && usize::from(zone.len) < ZONE_CHANGES {
            let next = t.saturating_add(86_400);
            let now = clamp(offset(next));
            if now != was {
                // The first second with the new offset.
                let (mut lo, mut hi) = (t, next);
                while hi - lo > 1 {
                    let mid = lo + (hi - lo) / 2;
                    if clamp(offset(mid)) == was {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                push(&mut zone, hi, now);
                was = now;
            }
            t = next;
        }
        zone
    }
}

impl Clock {
    pub fn new(now: i64, offset: i32) -> Clock {
        Clock { now, offset: offset.clamp(-14 * 3600, 14 * 3600), zone: Zone::default() }
    }

    // //// Neoffice — see `Clock::zone`.
    /// This clock with the zone's rules.
    pub fn with_zone(mut self, zone: Zone) -> Clock {
        self.zone = zone;
        self
    }

    /// The offset of an instant: by the zone's rules, else today's.
    pub fn offset_at(&self, t: i64) -> i32 {
        let n = usize::from(self.zone.len).min(ZONE_CHANGES);
        let mut found = None;
        for (at, offset) in self.zone.at.iter().zip(self.zone.offset.iter()).take(n) {
            if *at > t {
                break;
            }
            found = Some(*offset);
        }
        found.or_else(|| self.zone.offset.first().copied().filter(|_| n > 0)).unwrap_or(self.offset)
    }

    /// An instant as local time (seconds, as if UTC).
    pub fn local(&self, t: i64) -> i64 {
        t.saturating_add(i64::from(self.offset_at(t)))
    }

    /// The instant a local time (seconds, as if UTC) is, by the offset of its own date, whatever today's is: in the hour
    /// a change repeats, its first; in the hour a change skips, moved forward by the gap (02:30 becomes 03:30).
    pub fn utc(&self, local: i64) -> i64 {
        // The offsets on either side of that time (a change of offset is never within a day of another).
        let before = i64::from(self.offset_at(local.saturating_sub(86_400)));
        let after = i64::from(self.offset_at(local.saturating_add(86_400)));
        let is = |offset: i64| {
            let t = local.saturating_sub(offset);
            (self.local(t) == local).then_some(t)
        };
        match (is(before), is(after)) {
            (Some(a), Some(b)) => a.min(b),
            (Some(t), None) | (None, Some(t)) => t,
            // Skipped: the instant the offset before it would have given, which the offset after shows the gap later.
            (None, None) => local.saturating_sub(before),
        }
    }

    fn local_day(&self, t: i64) -> i64 {
        // //// Neoffice — the offset of its own date (`Clock::zone`).
        self.local(t).div_euclid(86_400)
    }
}

/// A date split into its parts (proleptic Gregorian).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Civil {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    /// 0 = Monday … 6 = Sunday.
    pub weekday: u32,
}

/// Days since 1970-01-01 → (year, month, day). Howard Hinnant's `civil_from_days`.
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z.saturating_add(719_468);
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// (year, month, day) → days since 1970-01-01.
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let m = i64::from(m.clamp(1, 12));
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(d.clamp(1, 31)) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Split a time into local calendar parts.
pub fn civil(t: i64, clock: &Clock) -> Civil {
    // //// Neoffice — with the offset of its own date (`Clock::zone`).
    let local = clock.local(t);
    let days = local.div_euclid(86_400);
    let secs = local.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    // 1970-01-01 was a Thursday (weekday 3 when Monday is 0).
    let weekday = (days + 3).rem_euclid(7) as u32;
    Civil { year, month, day, hour: (secs / 3600) as u32, minute: ((secs % 3600) / 60) as u32, weekday }
}

const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const WEEKDAYS_LONG: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

fn weekday_name(c: &Civil) -> &'static str {
    WEEKDAYS.get(c.weekday as usize).copied().unwrap_or("")
}

fn month_name(c: &Civil) -> &'static str {
    MONTHS.get(c.month.saturating_sub(1) as usize).copied().unwrap_or("")
}

/// The headings the message list groups by, newest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DateGroup {
    /// Dated in the future (a sender's clock is off): shown on top.
    Future,
    Today,
    Yesterday,
    /// Earlier this week (Monday-based); the weekday 0–6.
    ThisWeek(u32),
    LastWeek,
    TwoWeeksAgo,
    ThreeWeeksAgo,
    LastMonth,
    Older,
}

impl DateGroup {
    pub fn of(t: i64, clock: &Clock) -> DateGroup {
        let today = clock.local_day(clock.now);
        let day = clock.local_day(t);
        let diff = today - day;
        if diff < 0 {
            return DateGroup::Future;
        }
        if diff == 0 {
            return DateGroup::Today;
        }
        if diff == 1 {
            return DateGroup::Yesterday;
        }
        let today_wd = (today + 3).rem_euclid(7);
        let week_start = today - today_wd;
        if day >= week_start {
            return DateGroup::ThisWeek((day + 3).rem_euclid(7) as u32);
        }
        if day >= week_start - 7 {
            return DateGroup::LastWeek;
        }
        if day >= week_start - 14 {
            return DateGroup::TwoWeeksAgo;
        }
        if day >= week_start - 21 {
            return DateGroup::ThreeWeeksAgo;
        }
        let (ty, tm, _) = civil_from_days(today);
        let (y, m, _) = civil_from_days(day);
        let months_back = (ty * 12 + i64::from(tm)) - (y * 12 + i64::from(m));
        if months_back <= 1 { DateGroup::LastMonth } else { DateGroup::Older }
    }

    pub fn label(self) -> &'static str {
        match self {
            DateGroup::Future => "Upcoming",
            DateGroup::Today => "Today",
            DateGroup::Yesterday => "Yesterday",
            DateGroup::ThisWeek(wd) => WEEKDAYS_LONG.get(wd as usize).copied().unwrap_or("This week"),
            DateGroup::LastWeek => "Last week",
            DateGroup::TwoWeeksAgo => "Two weeks ago",
            DateGroup::ThreeWeeksAgo => "Three weeks ago",
            DateGroup::LastMonth => "Last month",
            DateGroup::Older => "Older",
        }
    }
}

/// The compact time in the message list: `14:05` today, `Mon 14:05` this week, `Oct 3` this
/// year, `2025-10-03` before that.
pub fn list_time(t: i64, clock: &Clock) -> String {
    let c = civil(t, clock);
    let now = civil(clock.now, clock);
    let days = clock.local_day(clock.now) - clock.local_day(t);
    if days == 0 {
        format!("{:02}:{:02}", c.hour, c.minute)
    } else if (1..7).contains(&days) {
        format!("{} {:02}:{:02}", weekday_name(&c), c.hour, c.minute)
    } else if c.year == now.year {
        format!("{} {}", month_name(&c), c.day)
    } else {
        format!("{:04}-{:02}-{:02}", c.year, c.month, c.day)
    }
}

/// The full date in the reading pane: `Wed 2026-10-14 14:05`.
pub fn full_time(t: i64, clock: &Clock) -> String {
    let c = civil(t, clock);
    format!("{} {:04}-{:02}-{:02} {:02}:{:02}", weekday_name(&c), c.year, c.month, c.day, c.hour, c.minute)
}

/// When a flagged message is due, as Outlook's Follow Up menu offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FollowUp {
    Today,
    Tomorrow,
    /// The last working day of this week (Friday), or today from Friday on.
    ThisWeek,
    /// Friday next week.
    NextWeek,
}

impl FollowUp {
    pub const ALL: [FollowUp; 4] = [FollowUp::Today, FollowUp::Tomorrow, FollowUp::ThisWeek, FollowUp::NextWeek];

    pub fn code(self) -> &'static str {
        match self {
            FollowUp::Today => "today",
            FollowUp::Tomorrow => "tomorrow",
            FollowUp::ThisWeek => "thisWeek",
            FollowUp::NextWeek => "nextWeek",
        }
    }

    pub fn from_code(s: &str) -> Option<FollowUp> {
        FollowUp::ALL.into_iter().find(|f| f.code().eq_ignore_ascii_case(s))
    }

    pub fn label(self) -> &'static str {
        match self {
            FollowUp::Today => "Today",
            FollowUp::Tomorrow => "Tomorrow",
            FollowUp::ThisWeek => "This week",
            FollowUp::NextWeek => "Next week",
        }
    }

    /// The local day it's due (days since 1970-01-01).
    pub fn due(self, clock: &Clock) -> i64 {
        let today = clock.local_day(clock.now);
        // 0 = Monday … 6 = Sunday; Friday is 4.
        let weekday = (today + 3).rem_euclid(7);
        match self {
            FollowUp::Today => today,
            FollowUp::Tomorrow => today + 1,
            FollowUp::ThisWeek => today + (4 - weekday).max(0),
            FollowUp::NextWeek => today + (7 - weekday) + 4,
        }
    }
}

/// Today, as a local day (days since 1970-01-01).
pub fn today(clock: &Clock) -> i64 {
    clock.local_day(clock.now)
}

/// A local day as `YYYY-MM-DD`.
pub fn day_string(day: i64) -> String {
    let (y, m, d) = civil_from_days(day);
    format!("{y:04}-{m:02}-{d:02}")
}

/// A local day as the reading pane shows it: `Fri 2026-10-16`.
pub fn day_label(day: i64) -> String {
    let (y, m, d) = civil_from_days(day);
    let weekday = WEEKDAYS.get((day + 3).rem_euclid(7) as usize).copied().unwrap_or("");
    format!("{weekday} {y:04}-{m:02}-{d:02}")
}

// //// Neoffice — see `Clock::zone`.
/// Europe/Zurich's rules, for the tests: CEST (+2) from the last Sunday of March, 01:00 UTC, to the last Sunday of
/// October, 01:00 UTC; CET (+1) otherwise.
#[cfg(test)]
pub(crate) fn zurich(t: i64) -> i32 {
    let (y, _, _) = civil_from_days(t.div_euclid(86_400));
    let last_sunday = |m: u32| {
        let end = days_from_civil(y, m, 31);
        (end - (end - 3).rem_euclid(7)) * 86_400 + 3600
    };
    if t >= last_sunday(3) && t < last_sunday(10) { 7200 } else { 3600 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follow_up_days() {
        // NOW is a Wednesday, 2026-10-14.
        let clock = Clock::new(NOW, 0);
        let day = |f: FollowUp| day_string(f.due(&clock));
        assert_eq!(day(FollowUp::Today), "2026-10-14");
        assert_eq!(day(FollowUp::Tomorrow), "2026-10-15");
        assert_eq!(day(FollowUp::ThisWeek), "2026-10-16");
        assert_eq!(day(FollowUp::NextWeek), "2026-10-23");
        // On a Saturday, "this week" is today.
        let sat = Clock::new(NOW + 3 * 86_400, 0);
        assert_eq!(day_string(FollowUp::ThisWeek.due(&sat)), "2026-10-17");
        assert_eq!(day_string(FollowUp::NextWeek.due(&sat)), "2026-10-23");
        assert_eq!(day_label(FollowUp::ThisWeek.due(&clock)), "Fri 2026-10-16");
        assert_eq!(FollowUp::from_code("nextweek"), Some(FollowUp::NextWeek));
    }

    // 2026-10-14 12:00:00 UTC, a Wednesday.
    const NOW: i64 = 1_791_979_200;

    #[test]
    fn civil_round_trip() {
        for d in [-800_000i64, -1, 0, 1, 365, 20_000, 2_000_000] {
            let (y, m, dd) = civil_from_days(d);
            assert_eq!(days_from_civil(y, m, dd), d);
        }
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        let c = civil(NOW, &Clock::new(NOW, 0));
        assert_eq!((c.year, c.month, c.day, c.hour, c.weekday), (2026, 10, 14, 12, 2));
    }

    #[test]
    fn groups() {
        let clock = Clock::new(NOW, 0);
        let day = 86_400;
        assert_eq!(DateGroup::of(NOW - 60, &clock), DateGroup::Today);
        assert_eq!(DateGroup::of(NOW + day, &clock), DateGroup::Future);
        assert_eq!(DateGroup::of(NOW - day, &clock), DateGroup::Yesterday);
        assert_eq!(DateGroup::of(NOW - 2 * day, &clock), DateGroup::ThisWeek(0));
        assert_eq!(DateGroup::of(NOW - 4 * day, &clock), DateGroup::LastWeek);
        assert_eq!(DateGroup::of(NOW - 12 * day, &clock), DateGroup::TwoWeeksAgo);
        assert_eq!(DateGroup::of(NOW - 30 * day, &clock), DateGroup::LastMonth);
        assert_eq!(DateGroup::of(NOW - 400 * day, &clock), DateGroup::Older);
    }

    #[test]
    fn offsets_move_the_day_boundary() {
        // 23:30 UTC on the 13th is already the 14th in UTC+2.
        let t = NOW - 12 * 3600 - 1800;
        assert_eq!(DateGroup::of(t, &Clock::new(NOW, 0)), DateGroup::Yesterday);
        assert_eq!(DateGroup::of(t, &Clock::new(NOW, 7200)), DateGroup::Today);
    }

    // //// Neoffice — a time is shown, and a local time read, with the offset of its own date (daylight saving), not today's.
    #[test]
    fn a_time_shows_with_the_offset_of_its_own_date() {
        let at = |y: i64, m: u32, d: u32, h: i64| days_from_civil(y, m, d) * 86_400 + h * 3600;
        let zone = Zone::from_offsets(NOW - 5 * 366 * 86_400, NOW + 10 * 366 * 86_400, super::zurich);
        // Today, 14 October 2026, is summer time (+2).
        let clock = Clock::new(NOW, 7200).with_zone(zone);
        // 10:00 in Zurich, before and after the change of 25 October 2026 (01:00 UTC), and of 29 March 2026.
        for (utc, day) in [(at(2026, 10, 20, 8), 20), (at(2026, 10, 27, 9), 27)] {
            let c = civil(utc, &clock);
            assert_eq!((c.month, c.day, c.hour, c.minute), (10, day, 10, 0), "October {day}");
            assert_eq!(clock.utc(clock.local(utc)), utc, "and back");
        }
        for (utc, day) in [(at(2026, 3, 28, 9), 28), (at(2026, 3, 30, 8), 30)] {
            let c = civil(utc, &clock);
            assert_eq!((c.month, c.day, c.hour), (3, day, 10), "March {day}");
        }
        // A local time read as the instant it is on its own date.
        assert_eq!(clock.utc(days_from_civil(2026, 10, 27) * 86_400 + 10 * 3600), at(2026, 10, 27, 9));
        assert_eq!(clock.utc(days_from_civil(2026, 10, 20) * 86_400 + 10 * 3600), at(2026, 10, 20, 8));
        // Exactly at the change.
        let change = at(2026, 10, 25, 1);
        assert_eq!((clock.offset_at(change - 1), clock.offset_at(change)), (7200, 3600));
        assert_eq!(list_time(at(2026, 10, 27, 9), &Clock::new(at(2026, 10, 27, 12), 3600).with_zone(zone)), "10:00");
        // Without the rules, today's offset for every date, as before.
        assert_eq!(civil(at(2026, 10, 27, 9), &Clock::new(NOW, 7200)).hour, 11);
        // A zone without changes, and hostile values.
        let fixed = Zone::from_offsets(0, 10 * 366 * 86_400, |_| 3600);
        assert_eq!(Clock::new(NOW, 3600).with_zone(fixed).offset_at(NOW), 3600);
        let _ = Clock::new(NOW, 0).with_zone(zone).utc(i64::MAX);
        let _ = civil(i64::MIN, &Clock::new(NOW, 0).with_zone(zone));
    }

    // //// Neoffice — the hour a change of offset skips, and the one it repeats, read the same whatever today's offset is.
    #[test]
    fn a_skipped_or_repeated_local_time_is_read_the_same_whatever_today_is() {
        let at = |y: i64, m: u32, d: u32, h: i64, mi: i64| days_from_civil(y, m, d) * 86_400 + h * 3600 + mi * 60;
        let zone = Zone::from_offsets(NOW - 2 * 366 * 86_400, NOW + 2 * 366 * 86_400, super::zurich);
        for today in [3600, 7200] {
            let clock = Clock::new(NOW, today).with_zone(zone);
            // 02:30 on 29 March 2026 doesn't exist (02:00 → 03:00): forward by the gap, to 03:30 summer time.
            let skipped = clock.utc(at(2026, 3, 29, 2, 30));
            assert_eq!((skipped, civil(skipped, &clock).hour, civil(skipped, &clock).minute), (at(2026, 3, 29, 1, 30), 3, 30), "today {today}");
            // 02:30 on 25 October 2026 happens twice (03:00 → 02:00): the first, summer time.
            assert_eq!(clock.utc(at(2026, 10, 25, 2, 30)), at(2026, 10, 25, 0, 30), "today {today}");
            // Elsewhere, the one instant there is.
            assert_eq!(clock.utc(at(2026, 10, 27, 10, 0)), at(2026, 10, 27, 9, 0));
        }
        // A zone that changes at midnight (00:00 → 01:00): a day's midnight is on that day, never the evening before.
        let change = days_from_civil(2026, 9, 6) * 86_400 - 3 * 3600;
        let midnight = Zone::from_offsets(change - 30 * 86_400, change + 30 * 86_400, |t| if t < change { 3 * 3600 } else { 4 * 3600 });
        let clock = Clock::new(change, 3 * 3600).with_zone(midnight);
        let day = days_from_civil(2026, 9, 6);
        assert_eq!(clock.local(clock.utc(day * 86_400)).div_euclid(86_400), day);
    }

    #[test]
    fn formats() {
        let clock = Clock::new(NOW, 0);
        assert_eq!(list_time(NOW - 3600, &clock), "11:00");
        assert_eq!(list_time(NOW - 2 * 86_400, &clock), "Mon 12:00");
        assert_eq!(list_time(NOW - 30 * 86_400, &clock), "Sep 14");
        assert_eq!(list_time(NOW - 400 * 86_400, &clock), "2025-09-09");
        assert_eq!(full_time(NOW, &clock), "Wed 2026-10-14 12:00");
        // Hostile values never panic.
        let _ = full_time(i64::MAX, &clock);
        let _ = list_time(i64::MIN, &clock);
        let _ = DateGroup::of(i64::MIN, &Clock::new(i64::MAX, i32::MAX));
    }
}
