//! Meeting invitations by mail (iTIP, RFC 5546, over iMIP, RFC 6047): answering one, taking
//! in the answers and cancellations organizers and guests send, and inviting people to the
//! user's own events. Invitations travel as `text/calendar` parts with a `METHOD`.

use mailcraft_model::ical::{self, Attendee, CalEvent, ICalendar};
use mailcraft_model::pim::{LOCAL, StoredEvent, Tombstone};
// //// Neoffice — the occurrences of a series a server expanded, answered on the series (respond_to_invite).
use mailcraft_model::pim::{Pim, SERIES_CHANGE, SERIES_CHANGE_LINE, SERIES_CHANGE_RULE, SERIES_LINE, series_line, set_series_line};

use super::*;

/// An invitation (or an answer to one, or a cancellation) in the open message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invite {
    /// `REQUEST`, `REPLY` or `CANCEL`.
    pub method: String,
    pub event: CalEvent,
    /// The user's answer so far (`NEEDS-ACTION`, `ACCEPTED`…), from their calendar when it's there.
    pub answer: String,
    /// The event is in one of the user's calendars.
    pub in_calendar: bool,
    /// For a reply: who answered, and how.
    pub replier: Option<Attendee>,
}

/// The answers a guest can give.
pub fn partstat(answer: &str) -> Option<&'static str> {
    match answer.trim().to_ascii_lowercase().as_str() {
        "accepted" | "accept" => Some("ACCEPTED"),
        "tentative" => Some("TENTATIVE"),
        "declined" | "decline" => Some("DECLINED"),
        _ => None,
    }
}

// //// Neoffice — the occurrence of a series a server expanded that carries an answer for the whole series: the first of
// `series` (indices in `p.events`) with no change of its own waiting (it would be sent for the series too); `None` when
// each has one (Session::update_occurrence refuses likewise).
fn series_carrier(p: &Pim, series: &[usize]) -> Option<usize> {
    series.iter().copied().find(|&i| p.events.get(i).is_some_and(|e| !(e.dirty && series_line(&e.event, SERIES_CHANGE_LINE).is_none())))
}

// //// Neoffice — the series' change an answer marks its carrier with: a change of the series' rule waiting there stays one.
fn series_change(e: &StoredEvent) -> &'static str {
    if e.dirty && series_line(&e.event, SERIES_CHANGE_LINE) == Some(SERIES_CHANGE_RULE) { SERIES_CHANGE_RULE } else { SERIES_CHANGE }
}

impl Session {
    /// The user's addresses (lower case).
    fn my_addresses(&self) -> Vec<String> {
        self.accounts
            .iter()
            .flat_map(|a| [a.config.email.trim().to_ascii_lowercase(), a.config.username.trim().to_ascii_lowercase()])
            .filter(|e| e.contains('@'))
            .collect()
    }

    /// The calendar in a message (its `text/calendar` part or `.ics` file), if it has one.
    pub fn message_calendar(&self, key: &MessageKey) -> Option<ICalendar> {
        let raw = self.store.as_ref()?.read_body(key)?;
        let parsed = mime::parse(&raw)?;
        let att = parsed
            .attachments
            .iter()
            .find(|a| a.mime.to_ascii_lowercase().starts_with("text/calendar") || a.name.to_ascii_lowercase().ends_with(".ics"))?;
        let data = mime::attachment_data(&raw, att.index)?;
        // //// Neoffice — read_calendar.
        let cal = self.read_calendar(&data);
        (!cal.events.is_empty()).then_some(cal)
    }

    // //// Neoffice — a calendar file, its floating times (and those named in a zone it doesn't describe) placed by the
    // offset of their own date (Clock::zone: daylight saving), not today's.
    pub fn read_calendar(&self, data: &[u8]) -> ICalendar {
        let clock = self.clock;
        ical::parse_floating(&String::from_utf8_lossy(data), &|t| clock.utc(t))
    }

    // //// Neoffice — the occurrences of a series a server expanded are its copy in the calendar (no event of the series
    // here): the invitation is in the calendar (its answer shown, « Show in calendar »), and the cancellation or a guest's
    // answer taken in (apply_invite).
    /// The event with a uid, else the first of its occurrences, and its account.
    fn find_in_calendar(&self, uid: &str) -> Option<(&str, &StoredEvent)> {
        self.find_event(uid).or_else(|| self.pim.iter().find_map(|(a, p)| p.events.iter().find(|e| e.event.uid == uid).map(|e| (a.as_str(), e))))
    }

    /// The invitation in the open message.
    pub fn invite(&self) -> Option<Invite> {
        let r = self.reading.as_ref()?;
        let cal = self.message_calendar(&r.key)?;
        let method = cal.method.clone().unwrap_or_else(|| "PUBLISH".into());
        if !matches!(method.as_str(), "REQUEST" | "REPLY" | "CANCEL") {
            return None;
        }
        let event = cal.events.iter().find(|e| e.recurrence_id.is_none()).or_else(|| cal.events.first())?.clone();
        let mine = self.my_addresses();
        // //// Neoffice — or one of the occurrences of a series a server expanded (find_in_calendar).
        let stored = self.find_in_calendar(&event.uid).map(|(_, e)| e.event.clone());
        let answer = stored
            .as_ref()
            .unwrap_or(&event)
            .attendees
            .iter()
            .find(|a| mine.contains(&a.email.to_ascii_lowercase()))
            .map(|a| a.partstat.clone())
            .unwrap_or_else(|| "NEEDS-ACTION".into());
        let replier = (method == "REPLY").then(|| event.attendees.first().cloned()).flatten();
        Some(Invite { method, event, answer, in_calendar: stored.is_some(), replier })
    }

    /// Answer the invitation in the open message: the event goes into the calendar (or out of
    /// it, when declined) and the organizer gets the answer.
    pub fn respond_to_invite(&mut self, answer: &str) -> CmdResult {
        let stat = partstat(answer).ok_or_else(|| CmdError::Params(format!("`{answer}`: answer accepted, tentative or declined")))?;
        let key = self.reading.as_ref().map(|r| r.key.clone()).ok_or_else(|| CmdError::Disabled("open the invitation first".into()))?;
        let cal = self.message_calendar(&key).ok_or_else(|| CmdError::Disabled("this message has no invitation".into()))?;
        if cal.method.as_deref() != Some("REQUEST") {
            return Err(CmdError::Disabled("this message isn't an invitation".into()));
        }
        let account = key.account.clone();
        let me = self.account(&account).map(|a| a.config.email.clone()).unwrap_or_default();
        let uid = cal.events.first().map(|e| e.uid.clone()).unwrap_or_default();
        let mut events: Vec<CalEvent> = cal.events.into_iter().filter(|e| e.uid == uid).collect();
        for e in &mut events {
            set_answer(e, &self.my_addresses(), &me, stat);
        }
        // Into the calendar, replacing an earlier copy (an updated invitation); out if declined.
        // //// Neoffice — with no copy yet, into a calendar new events can go to: not the one on this computer where it
        // isn't kept (the browser build in a hosting page).
        let target = self.new_event_calendar(&account);
        // //// Neoffice — see `in_place` below.
        let mine = self.my_addresses();
        let jmap = self.pim.get(&account).is_some_and(|p| p.caldav_home.as_deref() == Some("jmap"));
        let p = self.pim_mut(&account);
        // //// Neoffice — the copies in calendars that can be changed: one in a calendar that can't (a server's read-only
        // one, a colleague's shared one) stays as the server has it. With only such copies, or nowhere to keep one, only
        // the answer goes, by mail.
        let locked: Vec<String> = p.calendars.iter().filter(|c| c.read_only).map(|c| c.id.clone()).collect();
        let existing: Vec<StoredEvent> = p.events.iter().filter(|e| e.event.uid == uid && !locked.contains(&e.calendar)).cloned().collect();
        let read_only = existing.is_empty() && p.events.iter().any(|e| e.event.uid == uid);
        if read_only || (existing.is_empty() && target.is_none()) {
            if let Some(master) = events.first().cloned() {
                self.send_reply(&account, &master, stat)?;
            }
            return Ok(serde_json::json!({"answer": stat, "uid": uid, "readOnly": read_only, "inCalendar": read_only}));
        }
        // //// Neoffice — `target` rather than the first writable calendar (the same where the one here is kept).
        let calendar = existing.first().map(|e| e.calendar.clone()).or(target).unwrap_or_else(|| LOCAL.into());
        // //// Neoffice — the copies a JMAP server holds (its events are changed as patches of what the person changed,
        // calendars.rs in mailcraft-jmap) take the answer as they are, not the invitation's copy, which the organizer may
        // have changed since there: as `event.respond` does, the occurrences of a series it expanded (no event of the
        // series here) show it each and it goes once, for the series; declined, they go, a series once by its base event
        // there, as `event.delete` does. Nothing is made.
        let in_place = jmap && !existing.is_empty() && existing.iter().all(|e| e.calendar != LOCAL);
        if in_place {
            let expanded = !existing.iter().any(|e| e.event.recurrence_id.is_none());
            let mut remote = false;
            if stat == "DECLINED" {
                for g in &existing {
                    let href = match series_line(&g.event, SERIES_LINE) {
                        Some(base) if expanded => base.to_string(),
                        _ => g.href.clone(),
                    };
                    if href.is_empty() || p.deleted.iter().any(|t| t.href == href) {
                        continue;
                    }
                    p.deleted.push(Tombstone { href, etag: g.etag.clone(), calendar: g.calendar.clone() });
                    remote = true;
                }
                p.events.retain(|e| e.event.uid != uid || locked.contains(&e.calendar));
            } else {
                let copies: Vec<usize> =
                    p.events.iter().enumerate().filter(|(_, e)| e.event.uid == uid && !locked.contains(&e.calendar)).map(|(i, _)| i).collect();
                // //// Neoffice — the occurrence that carries the answer for the series (series_carrier).
                let series: Vec<usize> = copies
                    .iter()
                    .copied()
                    .filter(|&i| expanded && p.events.get(i).is_some_and(|e| series_line(&e.event, SERIES_LINE).is_some()))
                    .collect();
                let carrier = series_carrier(p, &series);
                if !series.is_empty() && carrier.is_none() {
                    return Err(CmdError::Params(super::pim::CHANGE_NOT_SENT.into()));
                }
                for i in copies {
                    let of_series = series.contains(&i);
                    let goes = !of_series || Some(i) == carrier;
                    if goes {
                        p.keep_as_read(i);
                    }
                    let Some(e) = p.events.get_mut(i) else { continue };
                    set_answer(&mut e.event, &mine, &me, stat);
                    if !goes {
                        continue;
                    }
                    if of_series {
                        let change = series_change(e);
                        set_series_line(&mut e.event, SERIES_CHANGE_LINE, Some(change));
                    }
                    e.dirty = true;
                    remote = true;
                }
            }
            self.after_pim_change(&account, remote);
            if let Some(master) = events.first().cloned() {
                self.send_reply(&account, &master, stat)?;
            }
            return Ok(serde_json::json!({"answer": stat, "uid": uid}));
        }
        // //// Neoffice — see `locked` above.
        p.events.retain(|e| e.event.uid != uid || locked.contains(&e.calendar));
        let mut remote = false;
        if stat == "DECLINED" {
            for g in existing.iter().filter(|g| !g.href.is_empty()) {
                p.deleted.push(Tombstone { href: g.href.clone(), etag: g.etag.clone(), calendar: g.calendar.clone() });
                remote = true;
            }
        } else {
            for (i, e) in events.iter().enumerate() {
                let old = existing.iter().find(|x| x.event.recurrence_id == e.recurrence_id);
                p.events.push(StoredEvent {
                    calendar: calendar.clone(),
                    href: old.map(|o| o.href.clone()).unwrap_or_default(),
                    etag: old.map(|o| o.etag.clone()).unwrap_or_default(),
                    event: e.clone(),
                    dirty: calendar != LOCAL,
                });
                remote |= calendar != LOCAL && i == 0;
            }
        }
        self.after_pim_change(&account, remote);
        if let Some(master) = events.first().cloned() {
            self.send_reply(&account, &master, stat)?;
        }
        Ok(serde_json::json!({"answer": stat, "uid": uid}))
    }

    /// Answer an invitation already in the calendar (from the event form).
    pub fn respond_to_event(&mut self, uid: &str, answer: &str) -> CmdResult {
        let stat = partstat(answer).ok_or_else(|| CmdError::Params(format!("`{answer}`: answer accepted, tentative or declined")))?;
        // //// Neoffice — or the occurrences of a series a server expanded, with no event of the series here.
        let account = self
            .find_event(uid)
            .map(|(a, _)| a.to_string())
            .or_else(|| self.pim.iter().find(|(_, p)| p.events.iter().any(|e| e.event.uid == uid)).map(|(a, _)| a.clone()))
            .ok_or_else(|| CmdError::Params(format!("no event `{uid}`")))?;
        let me = self.account(&account).map(|a| a.config.email.clone()).unwrap_or_default();
        let mine = self.my_addresses();
        let mut master = None;
        let p = self.pim_mut(&account);
        // //// Neoffice — its copies in calendars that can be changed take the answer; one in a calendar that can't (a
        // server's read-only one, a colleague's shared one) stays as the server has it. With only such copies, only the
        // answer goes, by mail.
        let locked: Vec<String> = p.calendars.iter().filter(|c| c.read_only).map(|c| c.id.clone()).collect();
        if !p.events.iter().any(|e| e.event.uid == uid && !locked.contains(&e.calendar)) {
            let first = p.events.iter().find(|e| e.event.uid == uid && e.event.recurrence_id.is_none()).map(|e| e.event.clone());
            if let Some(mut m) = first {
                set_answer(&mut m, &mine, &me, stat);
                self.send_reply(&account, &m, stat)?;
            }
            return Ok(serde_json::json!({"answer": stat, "uid": uid, "readOnly": true}));
        }
        let mut remote = false;
        // //// Neoffice — the occurrences of a series a server expanded show the answer each, and it goes once, for the
        // series, with the first of them that names it (SERIES_CHANGE_LINE); the reply to the organizer is the series'.
        let expanded = !p.events.iter().any(|e| e.event.uid == uid && e.event.recurrence_id.is_none());
        // //// Neoffice — the copies the answer goes with, as read before it (Pim::keep_as_read): each, but the occurrences
        // of a series a server expanded, one of which carries it (series_carrier).
        let copies: Vec<usize> =
            p.events.iter().enumerate().filter(|(_, e)| e.event.uid == uid && !locked.contains(&e.calendar)).map(|(i, _)| i).collect();
        let series: Vec<usize> =
            copies.iter().copied().filter(|&i| expanded && p.events.get(i).is_some_and(|e| series_line(&e.event, SERIES_LINE).is_some())).collect();
        let carrier = series_carrier(p, &series);
        if !series.is_empty() && carrier.is_none() {
            return Err(CmdError::Params(super::pim::CHANGE_NOT_SENT.into()));
        }
        for &i in &copies {
            if !series.contains(&i) || Some(i) == carrier {
                p.keep_as_read(i);
            }
        }
        // //// Neoffice — see `locked` above.
        for i in copies {
            let Some(e) = p.events.get_mut(i) else { continue };
            set_answer(&mut e.event, &mine, &me, stat);
            // //// Neoffice — see `expanded` above.
            let of_series = series.contains(&i);
            if of_series && Some(i) != carrier {
                continue;
            }
            if of_series {
                let change = series_change(e);
                mailcraft_model::pim::set_series_line(&mut e.event, mailcraft_model::pim::SERIES_CHANGE_LINE, Some(change));
                // Without what MailCraft keeps of the series for itself.
                let extra = e.event.extra.iter().filter(|l| !l.starts_with("X-MAILCRAFT-SERIES")).cloned().collect();
                master = Some(CalEvent { recurrence_id: None, extra, ..e.event.clone() });
            }
            remote |= e.calendar != LOCAL;
            e.dirty |= e.calendar != LOCAL;
            if e.event.recurrence_id.is_none() {
                master = Some(e.event.clone());
            }
        }
        self.after_pim_change(&account, remote);
        if let Some(m) = master {
            self.send_reply(&account, &m, stat)?;
        }
        Ok(serde_json::json!({"answer": stat, "uid": uid}))
    }

    /// Take in an answer from a guest, or a cancellation from the organizer, in the open message.
    pub fn apply_invite(&mut self) -> CmdResult {
        let inv = self.invite().ok_or_else(|| CmdError::Disabled("this message has no invitation".into()))?;
        let uid = inv.event.uid.clone();
        // //// Neoffice — into its copies in calendars that can be changed only: one in a calendar that can't (a server's
        // read-only one, a colleague's shared one) stays as the server has it; with only such copies, refused.
        // //// Neoffice — or by the occurrences of a series a server expanded (find_in_calendar).
        let (account, locked) = match self.find_in_calendar(&uid).map(|(a, _)| a.to_string()) {
            Some(a) => {
                let locked: Vec<String> =
                    self.pim.get(&a).map(|p| p.calendars.iter().filter(|c| c.read_only).map(|c| c.id.clone()).collect()).unwrap_or_default();
                (Some(a), locked)
            }
            None => (None, Vec::new()),
        };
        let writable = account
            .as_ref()
            .and_then(|a| self.pim.get(a))
            .is_some_and(|p| p.events.iter().any(|e| e.event.uid == uid && !locked.contains(&e.calendar)));
        if account.is_some() && !writable && matches!(inv.method.as_str(), "CANCEL" | "REPLY") {
            return Err(CmdError::Params("this calendar can't be changed".into()));
        }
        match inv.method.as_str() {
            "CANCEL" => {
                if self.find_event(&uid).is_none() {
                    // //// Neoffice — a series a server expanded (find_in_calendar) goes as « Delete the series » or « Delete
                    // this occurrence » has it go: by its base event there, once, or by that occurrence's own address.
                    if self.find_in_calendar(&uid).is_some() {
                        self.delete_event(&uid, inv.event.recurrence_id)?;
                        return Ok(serde_json::json!({"removed": true}));
                    }
                    return Ok(serde_json::json!({"removed": false}));
                }
                // Cancelled by the organizer: no answer is sent.
                let account = self.find_event(&uid).map(|(a, _)| a.to_string()).unwrap_or_default();
                let p = self.pim_mut(&account);
                let mut remote = false;
                let cancelled_one = inv.event.recurrence_id;
                match cancelled_one {
                    // //// Neoffice — see `locked` above (both cases).
                    Some(at) => {
                        // //// Neoffice — its copy as read, before it changes (Pim::keep_as_read).
                        let master =
                            p.events.iter().position(|e| e.event.uid == uid && e.event.recurrence_id.is_none() && !locked.contains(&e.calendar));
                        if let Some(i) = master {
                            p.keep_as_read(i);
                        }
                        if let Some(m) = master.and_then(|i| p.events.get_mut(i)) {
                            m.event.exdates.push(at);
                            remote = m.calendar != LOCAL;
                            m.dirty |= remote;
                        }
                    }
                    None => {
                        // //// Neoffice — see `locked` above.
                        for g in p.events.iter().filter(|e| e.event.uid == uid && !e.href.is_empty() && !locked.contains(&e.calendar)) {
                            p.deleted.push(Tombstone { href: g.href.clone(), etag: g.etag.clone(), calendar: g.calendar.clone() });
                            remote = true;
                        }
                        // //// Neoffice — see `locked` above.
                        p.events.retain(|e| e.event.uid != uid || locked.contains(&e.calendar));
                    }
                }
                self.after_pim_change(&account, remote);
                Ok(serde_json::json!({"removed": true}))
            }
            "REPLY" => {
                let who = inv.replier.ok_or_else(|| CmdError::Disabled("the answer names no one".into()))?;
                // //// Neoffice — see find_in_calendar.
                let account = self
                    .find_in_calendar(&uid)
                    .map(|(a, _)| a.to_string())
                    .ok_or_else(|| CmdError::Disabled("the event isn't in your calendar".into()))?;
                // //// Neoffice — a server that schedules itself (JMAP: Session::server_schedules) keeps its guests' answers:
                // the answer shows on the copies here, and nothing of it is sent there (nothing MailCraft writes takes a
                // guest's answer, and one it doesn't list would be invited).
                let jmap = self.pim.get(&account).is_some_and(|p| p.caldav_home.as_deref() == Some("jmap"));
                let p = self.pim_mut(&account);
                let mut remote = false;
                // //// Neoffice — the copies it answers: that occurrence, else the event; for a series a server expanded, each
                // of its occurrences where it answers the whole series.
                let expanded = !p.events.iter().any(|e| e.event.uid == uid && e.event.recurrence_id.is_none());
                let whole = expanded && inv.event.recurrence_id.is_none();
                let answers = |e: &StoredEvent| {
                    e.event.uid == uid && (whole || e.event.recurrence_id == inv.event.recurrence_id) && !locked.contains(&e.calendar)
                };
                // //// Neoffice — see `answers` above.
                for e in p.events.iter_mut().filter(|e| answers(e)) {
                    match e.event.attendees.iter_mut().find(|a| a.email.eq_ignore_ascii_case(&who.email)) {
                        Some(a) => a.partstat = who.partstat.clone(),
                        None => e.event.attendees.push(Attendee { rsvp: false, ..who.clone() }),
                    }
                    // //// Neoffice — see `jmap` above.
                    let sent = e.calendar != LOCAL && !jmap;
                    remote |= sent;
                    e.dirty |= sent;
                }
                self.after_pim_change(&account, remote);
                Ok(serde_json::json!({"recorded": who.partstat}))
            }
            _ => Err(CmdError::Disabled("answer the invitation instead".into())),
        }
    }

    /// Send the organizer the user's answer.
    fn send_reply(&mut self, account: &str, event: &CalEvent, stat: &str) -> Result<(), CmdError> {
        let Some(organizer) = event.organizer.clone().filter(|o| addr::is_valid_email(&o.email)) else { return Ok(()) };
        let me = self.account(account).map(|a| (a.config.name.clone(), a.config.email.clone())).unwrap_or_default();
        if organizer.email.eq_ignore_ascii_case(&me.1) {
            return Ok(());
        }
        let mine = self.my_addresses();
        let mut reply = event.clone();
        reply.attendees.retain(|a| mine.contains(&a.email.to_ascii_lowercase()));
        if reply.attendees.is_empty() {
            reply.attendees.push(Attendee { email: me.1.clone(), name: me.0.clone(), partstat: stat.into(), rsvp: false });
        }
        reply.description.clear();
        reply.extra.retain(|x| !x.to_ascii_uppercase().starts_with("BEGIN:VALARM"));
        let word = match stat {
            "ACCEPTED" => "Accepted",
            "TENTATIVE" => "Tentative",
            _ => "Declined",
        };
        let who = if me.0.is_empty() { me.1.clone() } else { me.0.clone() };
        let body = format!("{who} {} the invitation to {}.", word.to_ascii_lowercase(), event.summary);
        let to = vec![EmailAddress::new(organizer.name.clone(), organizer.email.clone())];
        self.send_itip(account, to, "REPLY", &[reply], &format!("{word}: {}", event.summary), &body)
    }

    /// Invite the guests of one of the user's events (or tell them of a change: `REQUEST`, or
    /// that it's off: `CANCEL`).
    pub fn send_invitations(&mut self, event: &CalEvent, account: &str, method: &str) -> Result<usize, CmdError> {
        let me = self.account(account).map(|a| a.config.email.to_ascii_lowercase()).unwrap_or_default();
        let to: Vec<EmailAddress> = event
            .attendees
            .iter()
            .filter(|a| !a.email.eq_ignore_ascii_case(&me) && addr::is_valid_email(&a.email))
            .map(|a| EmailAddress::new(a.name.clone(), a.email.clone()))
            .collect();
        if to.is_empty() {
            return Ok(0);
        }
        let mut e = event.clone();
        let (subject, body) = if method == "CANCEL" {
            e.status = "CANCELLED".into();
            (format!("Cancelled: {}", event.summary), format!("{} has been cancelled.", event.summary))
        } else {
            let when = mailcraft_model::dates::full_time(event.start, &self.clock);
            let place = if event.location.is_empty() { String::new() } else { format!("\nWhere: {}", event.location) };
            (format!("Invitation: {}", event.summary), format!("{}\nWhen: {when}{place}\n\n{}", event.summary, event.description))
        };
        let n = to.len();
        self.send_itip(account, to, method, &[e], &subject, &body)?;
        Ok(n)
    }

    /// Send a message carrying a calendar (`text/calendar; method=…`, RFC 6047).
    fn send_itip(
        &mut self,
        account: &str,
        to: Vec<EmailAddress>,
        method: &str,
        events: &[CalEvent],
        subject: &str,
        body: &str,
    ) -> Result<(), CmdError> {
        let acc = self.account(account).ok_or_else(|| CmdError::Failed("the account is gone".into()))?;
        let from = EmailAddress::new(acc.config.name.clone(), acc.config.email.clone());
        let local = acc.is_local();
        let sent_folder = acc.folder(FolderRole::Sent).map(|f| f.path.clone());
        let ics = ical::write(events, Some(method), self.clock.now);
        let d = mime::Draft {
            to,
            subject: subject.to_string(),
            body: body.to_string(),
            attachments: vec![mime::OutAttachment {
                name: "invite.ics".into(),
                mime: format!("text/calendar; method={method}"),
                data: ics.into_bytes(),
            }],
            ..Default::default()
        };
        let now = self.clock.now;
        let mid = mime::new_message_id(&from.email, now, self.random());
        let raw = mime::build(&d, &from, now, &mid).map_err(CmdError::Failed)?;
        if local {
            if let Some(folder) = &sent_folder {
                self.file_locally(account, folder, &raw, true);
            }
            return Ok(());
        }
        let id = self.fresh_id();
        self.jobs.push(Job::Send { account: account.to_string(), id, raw, from: from.email.clone(), recipients: d.recipients(), sent_folder });
        Ok(())
    }
}

/// Set the user's answer on an event (adding them as a guest when the invitation came through a
/// list or an alias).
fn set_answer(e: &mut CalEvent, mine: &[String], me: &str, stat: &str) {
    let mut found = false;
    for a in e.attendees.iter_mut().filter(|a| mine.contains(&a.email.to_ascii_lowercase())) {
        a.partstat = stat.to_string();
        a.rsvp = false;
        found = true;
    }
    if !found && !me.is_empty() {
        e.attendees.push(Attendee { email: me.to_string(), name: String::new(), partstat: stat.to_string(), rsvp: false });
    }
}
