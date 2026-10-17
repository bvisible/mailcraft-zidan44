// //// Neoffice — added file (no upstream equivalent): JMAP for Contacts (RFC 9610), read and written by the same client
// as the mail.
//
// A server that offers `urn:ietf:params:jmap:contacts` (Neoffice's mail facade does, for the people of the instance)
// gives its address books and the ids of their cards, page after page, then the cards (JSContact, RFC 9553) as many at
// a time as its `maxObjectsInGet` allows. They come back as a [`DavSync`], the shape the engine already merges for
// CardDAV and Microsoft Graph, so the address book, the autocompletion of the recipients and the person card read them
// unchanged; the browser build, which has no CardDAV, gets its contacts this way. A card made or changed here in one of
// the server's books goes back first (`ContactCard/set`, whole properties, as many at a time as its `maxObjectsInSet`
// allows); a book is written to only where the server lets the person write (`myRights.mayWrite`). A card deleted here
// is deleted there too (`destroy`), where the person may write the book (`mayWrite`, which covers destroying its cards)
// and the server doesn't narrow it (a `mayRemoveItems: false`, as Neoffice's facade gives it from the person's rights): a
// refusal is said once, and the card comes back. The note goes
// with the rest where the server keeps notes (not Neoffice's, whose contacts have none), and the organizations and titles
// MailCraft doesn't show (more than one, or more than a name) are kept as they were.

use std::collections::{HashMap, HashSet};

use mailcraft_model::ical::CalEvent;
use mailcraft_model::pim::{BookInfo, DavSync, LOCAL, Pim, StoredCard, StoredEvent};
use mailcraft_model::vcard::{Card, Typed};
use serde_json::{Map, Value, json};

use crate::{CORE, Client, Error, Http, Result, list, query_page};

/// The capability, and the type of the account that holds the address books.
pub const CONTACTS: &str = "urn:ietf:params:jmap:contacts";
/// The most cards read in one sync: a company's whole address book, not a directory.
const MAX_CARDS: usize = 5000;
/// What a card is read with: what [`Card`] shows.
const PROPERTIES: [&str; 9] = ["id", "addressBookIds", "uid", "name", "emails", "phones", "organizations", "titles", "notes"];
/// A card's address on this account (its id on the server), apart from those of CardDAV.
pub const HREF_PREFIX: &str = "jmap:";
/// Where a card keeps the server's organizations and titles that MailCraft doesn't show, so that writing back the one it
/// shows keeps the others: a line of the card's `extra`, as a vCard keeps its other properties there.
const KEPT_ORGANIZATIONS: &str = "X-MAILCRAFT-JSCONTACT-ORGANIZATIONS:";
const KEPT_TITLES: &str = "X-MAILCRAFT-JSCONTACT-TITLES:";

impl<H: Http> Client<H> {
    /// Whether the session offers address books.
    pub fn offers_contacts(&self) -> bool {
        self.contacts.is_some()
    }

    /// What was made, changed or deleted here first (`pim`: the account's address books as the engine keeps them;
    /// `notes`: the server keeps the cards' notes), then the address books and every card in them, as a sync to merge
    /// (`None` when the session offers none). What could not be sent is said in the sync's errors and stays to be sent:
    /// the books are read all the same. When they can't be read, what was sent is handed over all the same, the books
    /// unread (`None`). A token the server refuses ([`Error::Auth`]) stops the sync as such, for the mailbox to sign in
    /// again: what it sent before goes again then, which the server recognises (a card's uid, a deletion done).
    pub async fn sync_contacts(&self, pim: &Pim, notes: bool, now: i64) -> Result<Option<DavSync>> {
        let Some(account) = self.contacts.as_deref() else { return Ok(None) };
        let sent = self.push_cards(account, pim, notes).await?;
        let mut sync = match self.read_books(account, now).await {
            Ok(sync) => sync,
            Err(e @ Error::Auth(_)) => return Err(e),
            Err(e) => {
                log::warn!("the address books were not read: {e}");
                DavSync { carddav_home: Some("jmap".into()), checked: now, ..Default::default() }
            }
        };
        sync.pushed_cards = sent.pushed;
        sync.errors = sent.errors;
        sync.deleted = sent.deleted;
        sync.refused_cards = sent.refused;
        sync.refused_deletions = sent.refused_deletions;
        Ok(Some(sync))
    }

    /// What was made, changed or deleted here, sent, without reading the address books again: a sync of what is left to
    /// send (the books stay as the engine has them, `None`).
    pub async fn send_contacts(&self, pim: &Pim, notes: bool, now: i64) -> Result<Option<DavSync>> {
        let Some(account) = self.contacts.as_deref() else { return Ok(None) };
        let sent = self.push_cards(account, pim, notes).await?;
        Ok(Some(DavSync {
            carddav_home: Some("jmap".into()),
            checked: now,
            pushed_cards: sent.pushed,
            errors: sent.errors,
            deleted: sent.deleted,
            refused_cards: sent.refused,
            refused_deletions: sent.refused_deletions,
            ..Default::default()
        }))
    }

    /// The address books and every card in them.
    async fn read_books(&self, account: &str, now: i64) -> Result<DavSync> {
        let query = json!({"accountId": account});
        let r = self
            .call(
                &[CORE, CONTACTS],
                vec![("AddressBook/get", json!({"accountId": account, "ids": null})), ("ContactCard/query", query_page(&query, 0, MAX_CARDS))],
            )
            .await?;
        let books = list(r.first(), "list").to_vec();
        let first = r.get(1).cloned().unwrap_or(Value::Null);
        let (ids, whole) = self.query_rest(&[CORE, CONTACTS], "ContactCard/query", &query, first, MAX_CARDS).await?;
        let mut cards = Vec::with_capacity(ids.len());
        for chunk in ids.chunks(self.max_get.max(1)) {
            let got = self
                .call(&[CORE, CONTACTS], vec![("ContactCard/get", json!({"accountId": account, "ids": chunk, "properties": PROPERTIES}))])
                .await?;
            cards.extend(list(got.first(), "list").iter().cloned());
        }
        let mut sync = contacts_sync(&books, &cards, now);
        if !whole {
            // One query covers every book: none was read whole, so each is handed over as read in part, and the engine
            // adds and updates the cards read without losing those past the cap.
            log::warn!("the address books were not read whole (more than {MAX_CARDS} cards, or too many pages): merged without removing any");
            sync.partial_books = std::mem::take(&mut sync.synced_books);
        }
        Ok(sync)
    }

    /// Sends what was made, changed or deleted here in the server's books, as many at a time as the server takes: the
    /// cards it stored (with their ids), the deletions it is done with (made, or refused and said: the card comes back at
    /// the next read), the cards whose change it refused for good (said once: the change goes), and why the others were
    /// not (the call failed, or the server failed for the time being: they stay to be sent next time). Not the cards of a
    /// book the server no longer lets the person write: the engine lets their change go. A token the server refuses stops
    /// it (`Err`).
    async fn push_cards(&self, account: &str, pim: &Pim, notes: bool) -> Result<CardsSent> {
        let mut out = CardsSent::default();
        if pim.carddav_home.as_deref() != Some("jmap") {
            return Ok(out);
        }
        let changes: Vec<Change> = card_changes(pim).take(MAX_CARDS).collect();
        for chunk in changes.chunks(self.max_set.max(1)) {
            let (mut create, mut update, mut destroy) = (Map::new(), Map::new(), Vec::new());
            for (n, change) in chunk.iter().enumerate() {
                let c = match change {
                    Change::Card(c) => c,
                    Change::Gone(href) => {
                        destroy.extend(href.strip_prefix(HREF_PREFIX));
                        continue;
                    }
                };
                let mut body = card_json(&c.card, notes);
                match c.href.strip_prefix(HREF_PREFIX) {
                    Some(id) => {
                        update.insert(id.to_string(), body);
                    }
                    None => {
                        body["addressBookIds"] = json!({ c.book.clone(): true });
                        // Its own uid: a server that stored it before (its answer lost on the way) knows it again.
                        if !c.card.uid.trim().is_empty() {
                            body["uid"] = json!(c.card.uid);
                        }
                        create.insert(format!("k{n}"), body);
                    }
                }
            }
            let mut args = json!({"accountId": account, "create": create, "update": update});
            if !destroy.is_empty() {
                args["destroy"] = json!(destroy);
            }
            let answer = match self.call(&[CORE, CONTACTS], vec![("ContactCard/set", args)]).await {
                Ok(r) => r.into_iter().next().unwrap_or(Value::Null),
                Err(e @ Error::Auth(_)) => return Err(e),
                Err(e) => {
                    for change in chunk {
                        out.errors.push(match change {
                            Change::Card(c) => format!("a contact was not saved: {} — {e}", c.card.display()),
                            Change::Gone(_) => format!("a contact was not deleted: {e}"),
                        });
                    }
                    continue;
                }
            };
            for (n, change) in chunk.iter().enumerate() {
                let c = match change {
                    Change::Card(c) => c,
                    Change::Gone(href) => {
                        let id = href.strip_prefix(HREF_PREFIX).unwrap_or(href);
                        let done = answer.get("destroyed").and_then(Value::as_array).is_some_and(|d| d.iter().any(|x| x.as_str() == Some(id)));
                        let refused = answer.get("notDestroyed").and_then(|m| m.get(id));
                        // Already gone there (deleted by a sync whose answer was lost, or by someone else): done.
                        let gone = refused.and_then(|e| e.get("type")).and_then(Value::as_str) == Some("notFound");
                        match refused.map(Refusal::of) {
                            _ if done || gone => out.deleted.push((*href).to_string()),
                            // Not taken for now: it stays to be sent, said where the server failed.
                            Some(Refusal::Wait) => {}
                            Some(Refusal::Failed) => out.errors.push(format!("a contact was not deleted: {FAILED_FOR_NOW}")),
                            // Said once; the card is the server's still, and comes back at the next read.
                            Some(Refusal::Refused) => {
                                out.errors.push(format!("a contact was not deleted: {}", refusal_words(refused)));
                                out.deleted.push((*href).to_string());
                                out.refused_deletions.push((*href).to_string());
                            }
                            None => {}
                        }
                        continue;
                    }
                };
                let (key, id) = match c.href.strip_prefix(HREF_PREFIX) {
                    Some(id) => (id.to_string(), answer.get("updated").and_then(|u| u.get(id)).map(|_| id.to_string())),
                    None => {
                        let key = format!("k{n}");
                        // Made before (an answer lost on the way, a sync run twice): the server says which it is
                        // (`alreadyExists` and its `existingId`, RFC 8620 §5.3), and that one it is.
                        let existing = answer
                            .get("notCreated")
                            .and_then(|m| m.get(&key))
                            .filter(|e| e.get("type").and_then(Value::as_str) == Some("alreadyExists"))
                            .and_then(|e| e.get("existingId"));
                        let id = answer.get("created").and_then(|m| m.get(&key)).and_then(|v| v.get("id")).or(existing);
                        (key, id.and_then(Value::as_str).map(str::to_string))
                    }
                };
                match id {
                    Some(id) => out.pushed.push(StoredCard {
                        book: c.book.clone(),
                        href: format!("{HREF_PREFIX}{id}"),
                        etag: String::new(),
                        card: c.card.clone(),
                        dirty: false,
                    }),
                    None => {
                        // Refused for good: said once, and the change goes (the card follows the server again); for the
                        // time being, said, or not where the server asked to wait: it stays to be sent.
                        let refusal = ["notCreated", "notUpdated"].iter().find_map(|k| answer.get(*k).and_then(|m| m.get(&key)));
                        let why = match refusal.map(Refusal::of) {
                            Some(Refusal::Wait) => continue,
                            Some(Refusal::Failed) => FAILED_FOR_NOW.to_string(),
                            Some(Refusal::Refused) => {
                                out.refused.push((*c).clone());
                                refusal_words(refusal)
                            }
                            None => refusal_words(refusal),
                        };
                        out.errors.push(format!("a contact was not saved: {} — {why}", c.card.display()));
                    }
                }
            }
        }
        Ok(out)
    }
}

/// A refusal of a `/set` (a SetError, RFC 8620 §5.3) in words the interface translates (messages.rs in the interface):
/// « not found », « not allowed » with the server's reason, « too many changes at once », else « refused by the server »
/// with what it said.
pub(crate) fn refusal_words(refusal: Option<&Value>) -> String {
    let kind = refusal.and_then(|r| r.get("type")).and_then(Value::as_str).unwrap_or_default();
    let said = refusal.and_then(|r| r.get("description")).and_then(Value::as_str).map(str::trim).filter(|d| !d.is_empty());
    match (kind, said) {
        ("notFound", _) => "not found".into(),
        ("forbidden", Some(d)) => format!("not allowed: {d}"),
        ("forbidden", None) => "not allowed".into(),
        ("requestTooLarge" | "tooManyChanges", _) => "too many changes at once".into(),
        (_, Some(d)) => format!("refused by the server: {d}"),
        ("", None) => "refused by the server".into(),
        (k, None) => format!("refused by the server: {k}"),
    }
}

/// What a change the server didn't make is said with where it failed for the time being (Refusal::Failed).
pub(crate) const FAILED_FOR_NOW: &str = "the server failed; tried again at the next sync";

/// What the server's answer to a change it didn't make (a SetError, RFC 8620 §5.3) means for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// It asked to wait (`rateLimit`): sent again at the next sync, nothing said.
    Wait,
    /// It failed for the time being (`serverFail`, `serverUnavailable`, `serverPartialFail`; Neoffice's when the mail
    /// server doesn't answer): sent again at the next sync, said ([`FAILED_FOR_NOW`]).
    Failed,
    /// It refused the change (`forbidden`, `invalidProperties`, `notFound`…): said in words ([`refusal_words`]).
    Refused,
}

impl Refusal {
    pub(crate) fn of(refusal: &Value) -> Refusal {
        match refusal.get("type").and_then(Value::as_str) {
            Some("rateLimit") => Refusal::Wait,
            Some("serverFail" | "serverUnavailable" | "serverPartialFail") => Refusal::Failed,
            _ => Refusal::Refused,
        }
    }
}

/// What a send of the cards did (push_cards).
#[derive(Default)]
struct CardsSent {
    /// The cards stored, with their addresses.
    pushed: Vec<StoredCard>,
    errors: Vec<String>,
    /// The deletions done with (made, or refused and said).
    deleted: Vec<String>,
    /// The cards whose change the server refused for good.
    refused: Vec<StoredCard>,
    /// The deletions the server refused for good.
    refused_deletions: Vec<String>,
}

/// One change of a card to send.
enum Change<'a> {
    Card(&'a StoredCard),
    /// A card deleted here: its address there (`jmap:<id>`).
    Gone(&'a str),
}

/// What was made, changed or deleted here in the server's books: new cards (no address yet) and the server's own
/// (`jmap:<id>`, a card with another address is not this server's) in a book the person may still write, and their
/// deletions (a card's tombstone names no calendar, an event's does).
fn card_changes(pim: &Pim) -> impl Iterator<Item = Change<'_>> {
    let cards = pim
        .cards
        .iter()
        .filter(|c| c.dirty && c.book != LOCAL && (c.href.is_empty() || c.href.starts_with(HREF_PREFIX)))
        .filter(|c| !pim.book_read_only(&c.book))
        .map(Change::Card);
    let gone = pim.deleted.iter().filter(|t| t.href.starts_with(HREF_PREFIX) && t.calendar.is_empty()).map(|t| Change::Gone(t.href.as_str()));
    cards.chain(gone)
}

/// Whether a sync that only sent (`send_contacts`, `send_calendars`: nothing read) must be followed by a read: the server
/// refused something for good (a change, or a deletion: what was refused shows again as the server has it), not a failure
/// for the time being nor a call that failed (what they left is sent again).
pub fn read_after(sync: &DavSync) -> bool {
    !sync.refused_events.is_empty() || !sync.refused_cards.is_empty() || !sync.refused_deletions.is_empty()
}

/// Whether something made, changed or deleted here waits to be sent to the server's address books or calendars, as
/// their next sync would send it (`pim` as [`Sent::resume`] leaves it). A sync of the mail syncs them then, rather than
/// at its own pace: a change the server could not take for the time being goes again at the next sync, as said.
pub fn waiting(pim: &Pim) -> bool {
    (pim.carddav_home.as_deref() == Some("jmap") && card_changes(pim).next().is_some()) || crate::calendars::waiting(pim)
}

/// What syncs did on the server that the engine may not have taken in when the next sync starts (it can start before):
/// the cards and events they stored, the changes the server refused for good (said once), and the deletions they are done
/// with. Kept by the caller from one sync to the next, so that nothing is made twice, nor a deletion sent twice, nor a
/// refusal said twice; let go of once the engine has taken in the last sync recorded (`recorded`), so that a change made
/// since (an edit after a refusal, a deletion tried again) goes. A card or an event stored or refused is known by its
/// content too: one changed since is a new change, sent whatever was recorded.
#[derive(Clone, Debug, Default)]
pub struct Sent {
    /// The cards stored: their uid here → their address there.
    pub created: HashMap<String, String>,
    /// The cards' deletions done with (made, or refused and said), by address.
    pub deleted: HashSet<String>,
    /// The cards stored, as they were sent, by their address there.
    pub cards_sent: HashMap<String, Card>,
    /// The cards whose change the server refused for good (said once), as they were sent: by address, or by uid for one
    /// never stored.
    pub cards_refused: HashMap<String, Card>,
    /// The events made here and stored there: their uid → their address there (calendars.rs).
    pub events_created: HashMap<String, String>,
    /// The events stored, as they were sent, by their address there.
    pub events_sent: HashMap<String, CalEvent>,
    /// The events whose change the server refused (said once), as they were sent: by address, or by uid for one never
    /// stored.
    pub events_refused: HashMap<String, CalEvent>,
    /// The events' deletions done with, by address.
    pub events_deleted: HashSet<String>,
    /// The last sync recorded (its `checked`, which the caller makes grow from one sync to the next): once the engine has
    /// taken it in (`Pim::dav_checked`), all of the above is let go of.
    pub recorded: Option<i64>,
}

/// What a refused event is known by: its address there, or its uid for one never stored.
fn refused_key(e: &StoredEvent) -> String {
    if e.href.is_empty() { format!("uid:{}", e.event.uid) } else { e.href.clone() }
}

/// What a refused card is known by, likewise.
fn refused_card_key(c: &StoredCard) -> String {
    if c.href.is_empty() { format!("uid:{}", c.card.uid) } else { c.href.clone() }
}

impl Sent {
    /// What `sync` (the address books') sent, refused and deleted, kept until the engine has taken it in.
    pub fn record(&mut self, sync: &DavSync) {
        self.created.extend(sync.pushed_cards.iter().filter(|c| !c.card.uid.is_empty()).map(|c| (c.card.uid.clone(), c.href.clone())));
        self.cards_sent.extend(sync.pushed_cards.iter().filter(|c| !c.href.is_empty()).map(|c| (c.href.clone(), c.card.clone())));
        self.cards_refused.extend(sync.refused_cards.iter().map(|c| (refused_card_key(c), c.card.clone())));
        self.deleted.extend(sync.deleted.iter().cloned());
        self.recorded = self.recorded.max(Some(sync.checked));
    }

    /// What `sync` (the calendars', calendars.rs) sent, refused and deleted, kept until the engine has taken it in.
    pub fn record_events(&mut self, sync: &DavSync) {
        let made = sync.pushed_events.iter().filter(|e| e.event.recurrence_id.is_none() && !e.event.uid.is_empty());
        self.events_created.extend(made.map(|e| (e.event.uid.clone(), e.href.clone())));
        self.events_sent.extend(sync.pushed_events.iter().filter(|e| !e.href.is_empty()).map(|e| (e.href.clone(), e.event.clone())));
        self.events_refused.extend(sync.refused_events.iter().map(|e| (refused_key(e), e.event.clone())));
        self.events_deleted.extend(sync.deleted.iter().cloned());
        self.recorded = self.recorded.max(Some(sync.checked));
    }

    /// `pim` (a copy of the engine's) as the server has it now: the cards and events stored but not yet taken in,
    /// addressed as stored (this sync updates them rather than making them again), the refused changes no longer to be
    /// sent, and the deletions done no longer to be sent. What the engine has taken in, or is gone, is let go of. A
    /// card's tombstone names no calendar, an event's does.
    pub fn resume(&mut self, pim: &mut Pim) {
        // The engine has taken in the last sync recorded: nothing it did is to be told again.
        if self.recorded.is_some_and(|last| pim.dav_checked.is_some_and(|taken| taken >= last)) {
            *self = Sent::default();
        }
        self.created.retain(|uid, _| pim.cards.iter().any(|c| &c.card.uid == uid && c.href.is_empty()));
        for c in pim.cards.iter_mut().filter(|c| c.href.is_empty()) {
            if let Some(href) = self.created.get(&c.card.uid) {
                c.href.clone_from(href);
            }
        }
        // A card stored, or refused for good, as it still is here: not to be sent again. What no change waits for any more
        // is let go of.
        self.cards_sent.retain(|href, _| pim.cards.iter().any(|c| c.dirty && &c.href == href));
        self.cards_refused.retain(|k, _| pim.cards.iter().any(|c| c.dirty && &refused_card_key(c) == k));
        for c in pim.cards.iter_mut().filter(|c| c.dirty) {
            let sent = self.cards_sent.get(&c.href).is_some_and(|x| !c.href.is_empty() && *x == c.card);
            if sent || self.cards_refused.get(&refused_card_key(c)).is_some_and(|x| *x == c.card) {
                c.dirty = false;
            }
        }
        self.deleted.retain(|href| pim.deleted.iter().any(|t| t.calendar.is_empty() && &t.href == href));
        pim.deleted.retain(|t| !t.calendar.is_empty() || !self.deleted.contains(&t.href));
        // The events', likewise.
        self.events_created.retain(|uid, _| pim.events.iter().any(|e| &e.event.uid == uid && e.href.is_empty()));
        for e in pim.events.iter_mut().filter(|e| e.href.is_empty() && e.event.recurrence_id.is_none()) {
            if let Some(href) = self.events_created.get(&e.event.uid) {
                e.href.clone_from(href);
            }
        }
        // An event stored, or refused (said once), as it still is here: not to be sent again; one changed since goes.
        self.events_sent.retain(|href, _| pim.events.iter().any(|e| e.dirty && &e.href == href));
        self.events_refused.retain(|k, _| pim.events.iter().any(|e| e.dirty && &refused_key(e) == k));
        for e in pim.events.iter_mut().filter(|e| e.dirty) {
            let sent = self.events_sent.get(&e.href).is_some_and(|x| !e.href.is_empty() && *x == e.event);
            if sent || self.events_refused.get(&refused_key(e)).is_some_and(|x| *x == e.event) {
                e.dirty = false;
            }
        }
        self.events_deleted.retain(|href| pim.deleted.iter().any(|t| !t.calendar.is_empty() && &t.href == href));
        pim.deleted.retain(|t| t.calendar.is_empty() || !self.events_deleted.contains(&t.href));
    }
}

/// A card as JSContact (RFC 9553), with the whole properties MailCraft edits (a `/set` update replaces each of them),
/// its note too where the server keeps notes (`notes`). No `name` when the card has none, so that the server names it
/// from its address. The organizations and titles it doesn't show are written back as they were read.
pub fn card_json(c: &Card, notes: bool) -> Value {
    let mut components = Vec::new();
    if !c.given.trim().is_empty() {
        components.push(json!({"@type": "NameComponent", "kind": "given", "value": c.given.trim()}));
    }
    if !c.family.trim().is_empty() {
        components.push(json!({"@type": "NameComponent", "kind": "surname", "value": c.family.trim()}));
    }
    let full =
        if c.name.trim().is_empty() { format!("{} {}", c.given.trim(), c.family.trim()).trim().to_string() } else { c.name.trim().to_string() };
    let context = |kind: &str| match kind {
        "work" => json!({"work": true}),
        "home" => json!({"private": true}),
        _ => json!({}),
    };
    let emails: Map<String, Value> = c
        .emails
        .iter()
        .filter(|e| e.value.contains('@'))
        .enumerate()
        .map(|(n, e)| (format!("e{}", n + 1), json!({"@type": "EmailAddress", "address": e.value.trim(), "contexts": context(&e.kind)})))
        .collect();
    let phones: Map<String, Value> = c
        .phones
        .iter()
        .filter(|p| !p.value.trim().is_empty())
        .enumerate()
        .map(|(n, p)| {
            let features = if p.kind == "cell" { json!({"mobile": true}) } else { json!({"voice": true}) };
            (format!("p{}", n + 1), json!({"@type": "Phone", "number": p.value.trim(), "features": features, "contexts": context(&p.kind)}))
        })
        .collect();
    let mut v = json!({
        "@type": "Card",
        "version": "1.0",
        "emails": emails,
        "phones": phones,
        "organizations": with_shown(kept(c, KEPT_ORGANIZATIONS), &c.org, "o", |name| json!({"@type": "Organization", "name": name})),
        "titles": with_shown(kept(c, KEPT_TITLES), &c.title, "t", |name| json!({"@type": "Title", "name": name, "kind": "title"})),
    });
    if !full.is_empty() {
        v["name"] = json!({"@type": "Name", "full": full, "components": components});
    }
    if notes {
        v["notes"] = if c.note.trim().is_empty() { json!({}) } else { json!({"n1": {"@type": "Note", "note": c.note.trim()}}) };
    }
    v
}

/// The organizations or titles a card keeps from the server ([`KEPT_ORGANIZATIONS`], [`KEPT_TITLES`]), if any.
fn kept(c: &Card, prefix: &str) -> Option<Map<String, Value>> {
    c.extra.iter().find_map(|x| x.strip_prefix(prefix)).and_then(|j| serde_json::from_str::<Value>(j).ok()).and_then(|v| v.as_object().cloned())
}

/// Organizations or titles to write: those kept from the server with the one MailCraft shows (the first with a name, as
/// [`card_of`] reads it) given `shown`'s name, or gone when it is empty; else just that one (`fresh`), under `letter`1.
fn with_shown(kept: Option<Map<String, Value>>, shown: &str, letter: &str, fresh: impl Fn(&str) -> Value) -> Value {
    let shown = shown.trim();
    let mut all = kept.unwrap_or_default();
    let first = all.iter().find(|(_, e)| e.get("name").and_then(Value::as_str).is_some_and(|n| !n.trim().is_empty())).map(|(k, _)| k.clone());
    match first {
        Some(key) if shown.is_empty() => {
            all.remove(&key);
        }
        Some(key) => {
            if let Some(entry) = all.get_mut(&key).and_then(Value::as_object_mut) {
                entry.insert("name".into(), json!(shown));
            }
        }
        None if !shown.is_empty() => {
            let key = (1..).map(|n| format!("{letter}{n}")).find(|k| !all.contains_key(k)).unwrap_or_else(|| format!("{letter}1"));
            all.insert(key, fresh(shown));
        }
        None => {}
    }
    Value::Object(all)
}

/// Whether organizations or titles hold more than MailCraft shows and writes back (more than one, or more than a name).
fn beyond_shown(all: &Map<String, Value>) -> bool {
    all.len() > 1
        || all.values().any(|e| {
            e.as_object().is_some_and(|o| {
                o.iter().any(|(k, v)| match k.as_str() {
                    "@type" | "name" => false,
                    "kind" => v.as_str() != Some("title"),
                    _ => true,
                })
            })
        })
}

/// The sync the engine merges, from the server's address books and cards.
pub fn contacts_sync(books: &[Value], cards: &[Value], now: i64) -> DavSync {
    let list_books = books;
    let books: Vec<BookInfo> = books
        .iter()
        .filter_map(|b| {
            let id = b.get("id")?.as_str()?.to_string();
            let name = b.get("name").and_then(Value::as_str).filter(|n| !n.trim().is_empty()).unwrap_or("Contacts").to_string();
            // Writable where the server lets the person write (their rights in Neoffice).
            let writes = b.get("myRights").and_then(|r| r.get("mayWrite")).and_then(Value::as_bool).unwrap_or(false);
            Some(BookInfo { id, name, read_only: !writes, ctag: String::new() })
        })
        .collect();
    // Whose cards the person may delete: those of a book they may write (RFC 9610: destroying cards is `mayWrite`'s, while
    // `mayDelete` is about the book itself), unless the server narrows it with `mayRemoveItems: false` (not in the RFC:
    // Neoffice's facade says so to someone who may write contacts but not delete them).
    let no_delete_books = list_books
        .iter()
        .filter(|b| {
            let right = |key: &str| b.get("myRights").and_then(|r| r.get(key)).and_then(Value::as_bool);
            right("mayWrite") != Some(true) || right("mayRemoveItems") == Some(false)
        })
        .filter_map(|b| b.get("id").and_then(Value::as_str).map(str::to_string))
        .collect();
    let mut stored = Vec::new();
    for v in cards {
        let Some(id) = v.get("id").and_then(Value::as_str) else { continue };
        let Some(book) = home_book(&books, v.get("addressBookIds")) else { continue };
        stored.push(StoredCard { book, href: format!("{HREF_PREFIX}{id}"), etag: String::new(), card: card_of(v), dirty: false });
    }
    DavSync {
        carddav_home: Some("jmap".into()),
        checked: now,
        synced_books: books.iter().map(|b| b.id.clone()).collect(),
        books: Some(books),
        cards: stored,
        no_delete_books,
        ..Default::default()
    }
}

/// The one book a card is placed in (the engine keeps a card once, by its uid, whatever books hold it): the first of its
/// books, in the server's order, that the person may write, else the first of them; `None` when the server lists none of
/// its books.
fn home_book(books: &[BookInfo], ids: Option<&Value>) -> Option<String> {
    let holds = |b: &&BookInfo| ids.and_then(|m| m.get(&b.id)).is_some_and(|on| on.as_bool() != Some(false));
    books.iter().filter(holds).find(|b| !b.read_only).or_else(|| books.iter().find(holds)).map(|b| b.id.clone())
}

/// A JSContact card (RFC 9553) as MailCraft shows it.
pub fn card_of(v: &Value) -> Card {
    let text = |v: Option<&Value>| v.and_then(Value::as_str).map(str::trim).unwrap_or_default().to_string();
    let entries = |key: &str| v.get(key).and_then(Value::as_object).map(|m| m.values().collect::<Vec<_>>()).unwrap_or_default();
    let name = v.get("name");
    let part = |kind: &str| {
        name.and_then(|n| n.get("components"))
            .and_then(Value::as_array)
            .map(|c| {
                c.iter()
                    .filter(|p| p.get("kind").and_then(Value::as_str) == Some(kind))
                    .filter_map(|p| p.get("value").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default()
    };
    let context = |e: &Value| {
        let on = |key: &str| e.get("contexts").and_then(|c| c.get(key)).and_then(Value::as_bool).unwrap_or(false);
        if on("work") {
            "work"
        } else if on("private") {
            "home"
        } else {
            ""
        }
    };
    let emails = entries("emails")
        .into_iter()
        .filter_map(|e| Some(Typed { value: e.get("address")?.as_str()?.trim().to_string(), kind: context(e).into() }))
        .filter(|t| t.value.contains('@'))
        .collect();
    let phones = entries("phones")
        .into_iter()
        .filter_map(|p| {
            let number = p.get("number")?.as_str()?.trim().to_string();
            let mobile = p.get("features").and_then(|f| f.get("mobile")).and_then(Value::as_bool).unwrap_or(false);
            Some(Typed { value: number, kind: if mobile { "cell".into() } else { context(p).into() } })
        })
        .filter(|t| !t.value.is_empty())
        .collect();
    let first_text = |key: &str, field: &str| {
        entries(key)
            .into_iter()
            .find_map(|e| e.get(field).and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty()).map(str::to_string))
            .unwrap_or_default()
    };
    // The engine tells cards apart by their uid: one without (the RFC requires it, a server may forget) takes its id.
    let uid = text(v.get("uid")).trim_start_matches("urn:uuid:").to_string();
    // What MailCraft doesn't show of the organizations and titles, kept to be written back as it was.
    let mut extra = Vec::new();
    for (key, prefix) in [("organizations", KEPT_ORGANIZATIONS), ("titles", KEPT_TITLES)] {
        if let Some(all) = v.get(key).and_then(Value::as_object).filter(|all| beyond_shown(all)) {
            extra.push(format!("{prefix}{}", Value::Object(all.clone())));
        }
    }
    Card {
        uid: if uid.is_empty() { format!("{HREF_PREFIX}{}", text(v.get("id"))) } else { uid },
        name: text(name.and_then(|n| n.get("full"))),
        given: part("given"),
        family: part("surname"),
        emails,
        phones,
        org: first_text("organizations", "name"),
        title: first_text("titles", "name"),
        note: first_text("notes", "note"),
        extra,
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use mailcraft_model::pim::Tombstone;

    use super::*;

    fn jeanne() -> Value {
        json!({
            "@type": "Card", "version": "1.0", "id": "C1", "uid": "urn:uuid:4f2b", "addressBookIds": {"B1": true},
            "name": {"full": "Jeanne Muster", "components": [{"kind": "given", "value": "Jeanne"}, {"kind": "surname", "value": "Muster"}]},
            "emails": {"e1": {"address": "jeanne@example.ch", "contexts": {"work": true}}, "e2": {"address": "jm@example.org", "contexts": {"private": true}}, "e3": {"address": "not an address"}},
            "phones": {"p1": {"number": "+41 79 000 00 00", "features": {"mobile": true}}, "p2": {"number": "+41 21 000 00 00", "contexts": {"work": true}}},
            "organizations": {"o1": {"name": "Example SA"}},
            "titles": {"t1": {"name": "Comptable", "kind": "title"}},
            "notes": {"n1": {"note": "Préfère le téléphone."}}
        })
    }

    #[test]
    fn a_jscontact_card_reads_as_the_card_mailcraft_shows() {
        let card = card_of(&jeanne());
        assert_eq!((card.uid.as_str(), card.name.as_str(), card.given.as_str(), card.family.as_str()), ("4f2b", "Jeanne Muster", "Jeanne", "Muster"));
        assert_eq!(
            card.emails,
            vec![Typed { value: "jeanne@example.ch".into(), kind: "work".into() }, Typed { value: "jm@example.org".into(), kind: "home".into() }]
        );
        assert_eq!(
            card.phones,
            vec![Typed { value: "+41 79 000 00 00".into(), kind: "cell".into() }, Typed { value: "+41 21 000 00 00".into(), kind: "work".into() }]
        );
        assert_eq!((card.org.as_str(), card.title.as_str(), card.note.as_str()), ("Example SA", "Comptable", "Préfère le téléphone."));
        // A card with nearly nothing still reads: its display falls back on its address, its uid on its id.
        let bare = card_of(&json!({"id": "C2", "emails": {"e": {"address": "x@example.ch"}}}));
        assert_eq!(bare.display(), "x@example.ch");
        assert_eq!(bare.uid, "jmap:C2");
    }

    /// A canned server: answers in order, and keeps what was asked.
    struct Canned(RefCell<Vec<Value>>, RefCell<Vec<Value>>);

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

    /// A session offering address books, with these core limits.
    fn session(core: Value) -> Value {
        json!({
            "capabilities": {"urn:ietf:params:jmap:core": core, "urn:ietf:params:jmap:mail": {}, CONTACTS: {}},
            "primaryAccounts": {"urn:ietf:params:jmap:mail": "A1"},
            "apiUrl": "/api/", "downloadUrl": "/d/{accountId}/{blobId}", "uploadUrl": "/u/{accountId}/",
        })
    }

    /// A server that answers each method call with `answer(name, args)` (`{"error": …}` is a method error,
    /// `{"transport": …}` a network failure of the whole request, `{"status": 401}` that HTTP status for it), and keeps
    /// the calls in order.
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

    fn connect<F: Fn(&str, &Value) -> Value>(core: Value, answer: F) -> Client<Server<F>> {
        let server = Server { session: session(core), answer, asked: RefCell::new(Vec::new()) };
        crate::block_on(Client::connect(server, "jmap.example.com", "me", "pw", true)).unwrap()
    }

    fn calls<F: Fn(&str, &Value) -> Value>(client: &Client<Server<F>>, method: &str) -> Vec<Value> {
        client.http.asked.borrow().iter().filter(|(name, _)| name == method).map(|(_, args)| args.clone()).collect()
    }

    /// A card of book B1 with an address.
    fn card(id: &str) -> Value {
        json!({"id": id, "addressBookIds": {"B1": true}, "emails": {"e": {"address": format!("{id}@example.ch")}}})
    }

    /// The ids asked for, a card each.
    fn got(args: &Value) -> Value {
        json!({"list": args["ids"].as_array().cloned().unwrap_or_default().iter().filter_map(Value::as_str).map(card).collect::<Vec<_>>()})
    }

    #[test]
    fn the_cards_are_read_as_many_at_a_time_as_the_server_allows() {
        let answers = vec![
            session(json!({"maxObjectsInGet": 2})),
            json!({"methodResponses": [
                ["AddressBook/get", {"list": [{"id": "B1", "name": "Neoffice"}]}, "c0"],
                ["ContactCard/query", {"ids": ["C1", "C2", "C3"], "position": 0, "total": 3}, "c1"]
            ]}),
            json!({"methodResponses": [["ContactCard/get", {"list": [card("C1"), card("C2")]}, "c0"]]}),
            json!({"methodResponses": [["ContactCard/get", {"list": [card("C3")]}, "c0"]]}),
        ];
        let server = Canned(answers.into(), Vec::new().into());
        let client = crate::block_on(Client::connect(server, "jmap.example.com", "me", "pw", true)).unwrap();
        let sync = crate::block_on(client.sync_contacts(&Pim::default(), false, 9)).unwrap().unwrap();
        let hrefs: Vec<&str> = sync.cards.iter().map(|c| c.href.as_str()).collect();
        assert_eq!(hrefs, vec!["jmap:C1", "jmap:C2", "jmap:C3"]);
        assert_eq!(sync.synced_books, vec!["B1"], "every card of the book was read: it is read in full");
        let asked = client.http.1.borrow();
        let gets: Vec<usize> = asked.iter().skip(1).filter_map(|r| r["methodCalls"][0][1]["ids"].as_array().map(Vec::len)).collect();
        assert_eq!(gets, vec![2, 1], "two cards at a time, as the server allows");
        assert_eq!(asked[0]["using"], json!(["urn:ietf:params:jmap:core", CONTACTS]));
        let query = &asked[0]["methodCalls"][1][1];
        assert_eq!((&query["position"], &query["limit"], &query["calculateTotal"]), (&json!(0), &json!(MAX_CARDS), &json!(true)));
    }

    #[test]
    fn the_cards_are_read_page_after_page_to_the_end() {
        // The server pages by two (it says so in `limit`) and gives the total: three pages.
        let all: Vec<String> = (1..=5).map(|n| format!("C{n}")).collect();
        let paged = |name: &str, args: &Value| match name {
            "AddressBook/get" => json!({"list": [{"id": "B1", "name": "Neoffice"}]}),
            "ContactCard/query" => {
                let at = args["position"].as_u64().unwrap_or(0) as usize;
                let ids: Vec<&String> = all.iter().skip(at).take(2).collect();
                json!({"ids": ids, "position": at, "limit": 2, "total": all.len()})
            }
            "ContactCard/get" => got(args),
            _ => json!({}),
        };
        let client = connect(json!({}), paged);
        let sync = crate::block_on(client.sync_contacts(&Pim::default(), false, 9)).unwrap().unwrap();
        assert_eq!(sync.cards.len(), 5, "every page's cards");
        assert_eq!(sync.synced_books, vec!["B1"]);
        let pages: Vec<(u64, u64)> =
            calls(&client, "ContactCard/query").iter().map(|q| (q["position"].as_u64().unwrap(), q["limit"].as_u64().unwrap())).collect();
        assert_eq!(pages, vec![(0, MAX_CARDS as u64), (2, 2), (4, 2)], "each page from where the last one ended, by the server's limit");
        assert!(calls(&client, "ContactCard/query").iter().all(|q| q["calculateTotal"] == json!(true)));

        // A server that gives no total: on until a page comes back empty.
        let no_total = |name: &str, args: &Value| match name {
            "AddressBook/get" => json!({"list": [{"id": "B1", "name": "Neoffice"}]}),
            "ContactCard/query" => {
                let at = args["position"].as_u64().unwrap_or(0) as usize;
                let ids: Vec<&str> = ["C1", "C2", "C3"].into_iter().skip(at).take(2).collect();
                json!({"ids": ids, "position": at})
            }
            "ContactCard/get" => got(args),
            _ => json!({}),
        };
        let client = connect(json!({}), no_total);
        let sync = crate::block_on(client.sync_contacts(&Pim::default(), false, 9)).unwrap().unwrap();
        assert_eq!((sync.cards.len(), sync.synced_books.len()), (3, 1));
        let positions: Vec<u64> = calls(&client, "ContactCard/query").iter().map(|q| q["position"].as_u64().unwrap()).collect();
        assert_eq!(positions, vec![0, 2, 3], "the empty page at 3 ends it");
    }

    #[test]
    fn books_not_read_whole_are_handed_over_as_read_in_part() {
        // More cards than one sync reads (the server's total says so): the cards read are handed over, and the books as
        // read in part, which the engine merges without removing anything.
        let many: Vec<String> = (0..MAX_CARDS + 2000).map(|n| format!("C{n}")).collect();
        let huge = |name: &str, args: &Value| match name {
            "AddressBook/get" => json!({"list": [{"id": "B1", "name": "Neoffice"}]}),
            "ContactCard/query" => {
                let at = args["position"].as_u64().unwrap_or(0) as usize;
                let n = args["limit"].as_u64().unwrap_or(0) as usize;
                json!({"ids": many.iter().skip(at).take(n).collect::<Vec<_>>(), "position": at, "total": many.len()})
            }
            "ContactCard/get" => got(args),
            _ => json!({}),
        };
        let client = connect(json!({"maxObjectsInGet": 1000}), huge);
        let sync = crate::block_on(client.sync_contacts(&Pim::default(), false, 9)).unwrap().unwrap();
        assert!(sync.synced_books.is_empty(), "not read whole: not replaced");
        assert_eq!(sync.partial_books, vec!["B1"], "read in part: merged without removing");
        assert_eq!((sync.books.as_ref().map(Vec::len), sync.cards.len()), (Some(1), MAX_CARDS), "the books listed, the cap's cards handed over");
        assert_eq!(calls(&client, "ContactCard/query").len(), 1, "the cap reached on the first page: no more asked");
    }

    #[test]
    fn a_query_stops_at_the_cap_and_at_a_page_that_brings_nothing_new() {
        // Four of them, read three at most, two a page: not all of them.
        let pages = |name: &str, args: &Value| match name {
            "Thing/query" => {
                let at = args["position"].as_u64().unwrap_or(0) as usize;
                let ids: Vec<&str> = ["T1", "T2", "T3", "T4"].into_iter().skip(at).take(2).collect();
                json!({"ids": ids, "position": at, "total": 4})
            }
            _ => json!({}),
        };
        let client = connect(json!({}), pages);
        let args = json!({"accountId": "A1"});
        let first = crate::block_on(client.call(&[CORE], vec![("Thing/query", query_page(&args, 0, 3))])).unwrap().remove(0);
        let (ids, whole) = crate::block_on(client.query_rest(&[CORE], "Thing/query", &args, first, 3)).unwrap();
        assert_eq!((ids, whole), (vec!["T1".to_string(), "T2".into(), "T3".into()], false));
        let last = calls(&client, "Thing/query").last().cloned().unwrap();
        assert_eq!(last["limit"], json!(1), "never more than the cap asked");
        // A server that ignores `position` says its page again: that is all it gives.
        let repeats = |_: &str, _: &Value| json!({"ids": ["T1", "T2"]});
        let client = connect(json!({}), repeats);
        let first = crate::block_on(client.call(&[CORE], vec![("Thing/query", query_page(&args, 0, 10))])).unwrap().remove(0);
        let (ids, whole) = crate::block_on(client.query_rest(&[CORE], "Thing/query", &args, first, 10)).unwrap();
        assert_eq!((ids.len(), whole), (2, true));
        assert_eq!(client.http.asked.borrow().len(), 2, "asked once more, not for ever");
    }

    /// The account's address books as the engine keeps them after a sync from this server.
    fn kept(cards: Vec<StoredCard>) -> Pim {
        Pim {
            books: vec![BookInfo { id: "B1".into(), name: "Neoffice".into(), read_only: false, ctag: String::new() }],
            cards,
            carddav_home: Some("jmap".into()),
            ..Default::default()
        }
    }

    fn stored(book: &str, href: &str, card: Card, dirty: bool) -> StoredCard {
        StoredCard { book: book.into(), href: href.into(), etag: String::new(), card, dirty }
    }

    fn named(uid: &str, name: &str, email: &str) -> Card {
        Card { uid: uid.into(), name: name.into(), emails: vec![Typed { value: email.into(), kind: "work".into() }], ..Default::default() }
    }

    /// The server's book B1, writable, its cards C1 and C7; a set stores what it is given (`created`: N1, N2… in the order
    /// of the call's keys).
    fn writable(name: &str, args: &Value) -> Value {
        match name {
            "AddressBook/get" => {
                json!({"list": [{"id": "B1", "name": "Neoffice", "myRights": {"mayRead": true, "mayWrite": true, "mayDelete": true}}]})
            }
            "ContactCard/query" => json!({"ids": ["C1", "C7"], "position": 0, "total": 2}),
            "ContactCard/get" => got(args),
            "ContactCard/set" => {
                let created: Map<String, Value> = args["create"]
                    .as_object()
                    .map(|m| {
                        m.keys().enumerate().map(|(n, k)| (k.clone(), json!({"id": format!("N{}", n + 1), "uid": format!("N{}", n + 1)}))).collect()
                    })
                    .unwrap_or_default();
                let updated: Map<String, Value> =
                    args["update"].as_object().map(|m| m.keys().map(|k| (k.clone(), Value::Null)).collect()).unwrap_or_default();
                json!({"created": created, "updated": updated, "destroyed": args.get("destroy").cloned().unwrap_or(json!([]))})
            }
            _ => json!({}),
        }
    }

    #[test]
    fn cards_made_or_changed_here_go_back_first_in_one_set() {
        let mut changed = named("C7", "Paul Exemple", "paul@example.ch");
        changed.org = "Exemple SA".into();
        changed.phones = vec![Typed { value: "+41 79 111 11 11".into(), kind: "cell".into() }];
        let mut pim = kept(vec![
            stored("B1", "", named("u-new@mailcraft", "Jeanne Muster", "jeanne@example.ch"), true),
            stored("B1", "jmap:C7", changed, true),
            stored("local", "", named("u-here@mailcraft", "Only Here", "here@example.ch"), true),
            stored("B1", "jmap:C1", named("C1", "Unchanged", "c1@example.ch"), false),
        ]);
        pim.deleted = vec![
            Tombstone { href: "jmap:C9".into(), ..Default::default() },
            Tombstone { href: "jmap:E1".into(), calendar: "K1".into(), ..Default::default() },
        ];
        let client = connect(json!({}), writable);
        let sync = crate::block_on(client.sync_contacts(&pim, false, 9)).unwrap().unwrap();

        let order: Vec<String> = client.http.asked.borrow().iter().map(|(name, _)| name.clone()).collect();
        assert_eq!(order.first().map(String::as_str), Some("ContactCard/set"), "what changed here goes first: {order:?}");
        let sets = calls(&client, "ContactCard/set");
        assert_eq!(sets.len(), 1, "the new card and the changed one in one set");
        let set = &sets[0];
        let create = set["create"].as_object().unwrap();
        assert_eq!(create.len(), 1, "the card on this computer only is not sent, nor the unchanged one");
        let new = create.values().next().unwrap();
        assert_eq!(new["addressBookIds"], json!({"B1": true}));
        assert_eq!(new["uid"], json!("u-new@mailcraft"), "its own uid, so that a create already made is known again");
        assert_eq!(new["name"]["full"], json!("Jeanne Muster"));
        assert_eq!(new["emails"]["e1"]["address"], json!("jeanne@example.ch"));
        let update = set["update"].as_object().unwrap();
        assert_eq!(update.keys().collect::<Vec<_>>(), vec!["C7"]);
        let whole = &update["C7"];
        assert!(whole.get("addressBookIds").is_none() && whole.get("uid").is_none(), "an update moves no card and keeps its uid");
        assert_eq!(whole["organizations"]["o1"]["name"], json!("Exemple SA"));
        assert_eq!(whole["phones"]["p1"]["features"], json!({"mobile": true}));
        assert_eq!((whole["titles"].clone(), whole["name"]["full"].clone()), (json!({}), json!("Paul Exemple")), "whole properties, empty ones too");
        assert_eq!(set["destroy"], json!(["C9"]), "a card deleted here is deleted there; an event's tombstone is not a card's");

        let pushed: Vec<(&str, &str, &str)> = sync.pushed_cards.iter().map(|c| (c.book.as_str(), c.href.as_str(), c.card.uid.as_str())).collect();
        assert_eq!(pushed, vec![("B1", "jmap:N1", "u-new@mailcraft"), ("B1", "jmap:C7", "C7")], "the new card with the server's id");
        assert!(sync.pushed_cards.iter().all(|c| !c.dirty));
        assert_eq!(sync.deleted, vec!["jmap:C9"], "done with it");
        assert!(sync.errors.is_empty(), "{:?}", sync.errors);
        assert_eq!(sync.cards.len(), 2, "then the book is read");
    }

    #[test]
    fn a_refused_or_failed_push_is_said_and_the_books_are_read_all_the_same() {
        let pim = kept(vec![
            stored("B1", "", named("u-1@mailcraft", "Jeanne Muster", "jeanne@example.ch"), true),
            stored("B1", "jmap:C7", named("C7", "Paul Exemple", "paul@example.ch"), true),
        ]);
        let refusing = |name: &str, args: &Value| match name {
            "ContactCard/set" => json!({
                "notCreated": {"k0": {"type": "forbidden", "description": "Your rights do not allow it."}},
                "notUpdated": {"C7": {"type": "notFound"}}
            }),
            _ => writable(name, args),
        };
        let client = connect(json!({}), refusing);
        let sync = crate::block_on(client.sync_contacts(&pim, false, 9)).unwrap().unwrap();
        // In words the interface translates, not the server's kinds of error.
        assert_eq!(
            sync.errors,
            vec![
                "a contact was not saved: Jeanne Muster — not allowed: Your rights do not allow it.",
                "a contact was not saved: Paul Exemple — not found"
            ]
        );
        assert!(sync.pushed_cards.is_empty(), "nothing stored: both stay to be sent");
        assert_eq!((sync.cards.len(), sync.synced_books.len()), (2, 1), "the book is read all the same");
        // Not taken for now (the server failed, or asked to wait): they stay to be sent, said where it failed only.
        let busy = |name: &str, args: &Value| match name {
            "ContactCard/set" => json!({"notCreated": {"k0": {"type": "serverUnavailable"}}, "notUpdated": {"C7": {"type": "rateLimit"}}}),
            _ => writable(name, args),
        };
        let sync = crate::block_on(connect(json!({}), busy).sync_contacts(&pim, false, 9)).unwrap().unwrap();
        assert_eq!(sync.errors, vec!["a contact was not saved: Jeanne Muster — the server failed; tried again at the next sync"]);
        assert!(sync.pushed_cards.is_empty(), "both stay to be sent");

        // The set fails as a whole (a method error, or the network): said for each card, and the book is still read.
        for failure in [json!({"error": {"type": "invalidArguments"}}), json!({"transport": "reset"})] {
            let failing = |name: &str, args: &Value| if name == "ContactCard/set" { failure.clone() } else { writable(name, args) };
            let client = connect(json!({}), failing);
            let sync = crate::block_on(client.sync_contacts(&pim, false, 9)).unwrap().unwrap();
            assert_eq!(sync.errors.len(), 2, "{:?}", sync.errors);
            assert!(sync.errors[0].starts_with("a contact was not saved: Jeanne Muster — "), "{:?}", sync.errors);
            assert!(sync.pushed_cards.is_empty());
            assert_eq!(sync.cards.len(), 2, "the books are read after a push that failed");
        }
    }

    #[test]
    fn cards_go_back_in_batches_the_server_takes() {
        let pim = kept((1..=3).map(|n| stored("B1", "", named(&format!("u-{n}@mailcraft"), &format!("Person {n}"), "p@example.ch"), true)).collect());
        let client = connect(json!({"maxObjectsInSet": 2}), writable);
        let sync = crate::block_on(client.sync_contacts(&pim, false, 9)).unwrap().unwrap();
        let sizes: Vec<usize> = calls(&client, "ContactCard/set").iter().map(|s| s["create"].as_object().map_or(0, Map::len)).collect();
        assert_eq!(sizes, vec![2, 1], "two at a time, as the server allows");
        assert_eq!(sync.pushed_cards.len(), 3);
        // Nothing goes back to a server whose books the engine did not get from it.
        let mut elsewhere = pim.clone();
        elsewhere.carddav_home = None;
        let client = connect(json!({}), writable);
        crate::block_on(client.sync_contacts(&elsewhere, false, 9)).unwrap();
        assert!(calls(&client, "ContactCard/set").is_empty());
    }

    #[test]
    fn a_card_goes_back_with_the_properties_mailcraft_edits() {
        let mut c = named("u@mailcraft", "", "x@example.ch");
        c.given = "Jeanne".into();
        c.family = "Muster".into();
        c.title = "Comptable".into();
        let v = card_json(&c, false);
        assert_eq!(v["name"]["full"], json!("Jeanne Muster"), "the parts make the name when there is no other");
        assert_eq!(v["name"]["components"][1], json!({"@type": "NameComponent", "kind": "surname", "value": "Muster"}));
        assert_eq!(v["titles"]["t1"]["name"], json!("Comptable"));
        assert_eq!(v["emails"]["e1"]["contexts"], json!({"work": true}));
        // No name at all: none is sent, and the server names the card from its address.
        assert!(card_json(&named("u@mailcraft", "", "x@example.ch"), false).get("name").is_none());
        assert!(v.get("uid").is_none(), "the uid goes with a create only");
    }

    #[test]
    fn the_books_and_their_cards_come_back_as_a_sync_to_merge() {
        let books = vec![
            json!({"id": "B1", "name": "Neoffice"}),
            json!({"id": "B2", "name": " ", "myRights": {"mayRead": true, "mayWrite": true}}),
            json!({"name": "no id"}),
        ];
        let mut in_two = jeanne();
        in_two["id"] = json!("C3");
        in_two["addressBookIds"] = json!({"B1": true, "B2": true, "B9": true});
        let sync = contacts_sync(&books, &[jeanne(), in_two, json!({"no": "id"})], 7);
        let names: Vec<(&str, &str, bool)> = sync.books.iter().flatten().map(|b| (b.id.as_str(), b.name.as_str(), b.read_only)).collect();
        assert_eq!(
            names,
            vec![("B1", "Neoffice", true), ("B2", "Contacts", false)],
            "writable where the person may write; a book with no name is named"
        );
        assert_eq!(sync.synced_books, vec!["B1", "B2"], "every book is read in full: what is gone from them goes");
        let placed: Vec<(&str, &str)> = sync.cards.iter().map(|c| (c.book.as_str(), c.href.as_str())).collect();
        assert_eq!(
            placed,
            vec![("B1", "jmap:C1"), ("B2", "jmap:C3")],
            "a card in two books is placed once, as the engine keeps it (by its uid): in the first of them the person may write; an unknown book is left out"
        );
        assert_eq!((sync.carddav_home.as_deref(), sync.checked, sync.caldav_home.as_deref()), (Some("jmap"), 7, None));
    }

    #[test]
    fn a_card_made_once_is_never_made_twice() {
        // The server says it has it already (an answer lost on the way): that card it is.
        let pim = kept(vec![stored("B1", "", named("u-1@mailcraft", "Jeanne Muster", "jeanne@example.ch"), true)]);
        let exists = |name: &str, args: &Value| match name {
            "ContactCard/set" => json!({"notCreated": {"k0": {"type": "alreadyExists", "existingId": "N7"}}}),
            _ => writable(name, args),
        };
        let client = connect(json!({}), exists);
        let sync = crate::block_on(client.sync_contacts(&pim, false, 9)).unwrap().unwrap();
        assert_eq!(sync.pushed_cards.iter().map(|c| c.href.as_str()).collect::<Vec<_>>(), ["jmap:N7"]);
        assert!(sync.errors.is_empty(), "{:?}", sync.errors);

        // Synced again before the engine took that in: in its copy the card is still new; it is neither made nor sent again,
        // and a change made to it since goes as a change of the card made, not as a new card.
        let mut sent = Sent::default();
        sent.record(&sync);
        let mut copy = pim.clone();
        sent.resume(&mut copy);
        assert_eq!((copy.cards[0].href.as_str(), copy.cards[0].dirty), ("jmap:N7", false));
        let client = connect(json!({}), writable);
        crate::block_on(client.sync_contacts(&copy, false, 9)).unwrap();
        assert!(calls(&client, "ContactCard/set").is_empty(), "nothing left to send");
        let mut changed = pim.clone();
        changed.cards[0].card.title = "Head of site".into();
        sent.resume(&mut changed);
        assert_eq!((changed.cards[0].href.as_str(), changed.cards[0].dirty), ("jmap:N7", true));
        let client = connect(json!({}), writable);
        crate::block_on(client.sync_contacts(&changed, false, 9)).unwrap();
        let set = calls(&client, "ContactCard/set").remove(0);
        assert!(set["create"].as_object().is_some_and(Map::is_empty) && set["update"].get("N7").is_some(), "{set}");
        // Once the engine has it (its address), or it is gone, it is let go of.
        let mut taken = pim.clone();
        taken.cards[0].href = "jmap:N7".into();
        sent.resume(&mut taken);
        assert!(sent.created.is_empty());

        // A deletion done is not sent again by the next sync, while the engine still holds its tombstone.
        let mut gone = kept(Vec::new());
        gone.deleted = vec![Tombstone { href: "jmap:C9".into(), ..Default::default() }];
        let client = connect(json!({}), writable);
        let first = crate::block_on(client.sync_contacts(&gone, false, 9)).unwrap().unwrap();
        sent.record(&first);
        let mut again = gone.clone();
        sent.resume(&mut again);
        let client = connect(json!({}), writable);
        crate::block_on(client.sync_contacts(&again, false, 9)).unwrap();
        assert!(calls(&client, "ContactCard/set").is_empty(), "nothing left to send");
        // Once the engine has let the tombstone go, so does the record.
        sent.resume(&mut kept(Vec::new()));
        assert!(sent.deleted.is_empty());
    }

    #[test]
    fn what_a_sync_did_to_the_events_is_not_done_twice() {
        use mailcraft_model::ical::CalEvent;
        use mailcraft_model::pim::StoredEvent;
        let event = |href: &str, uid: &str, dirty: bool| StoredEvent {
            calendar: "K1".into(),
            href: href.into(),
            etag: String::new(),
            event: CalEvent { uid: uid.into(), summary: uid.into(), ..Default::default() },
            dirty,
        };
        let mut pim = kept(Vec::new());
        pim.events = vec![event("", "made", true), event("jmap:E2", "refused", true)];
        pim.deleted = vec![
            Tombstone { href: "jmap:E3".into(), calendar: "K1".into(), ..Default::default() },
            // A card's, with the same id: not the event's.
            Tombstone { href: "jmap:E3".into(), ..Default::default() },
        ];
        // What the calendars' sync did: one made (E1), one refused (said), one deleted.
        let sync = DavSync {
            pushed_events: vec![event("jmap:E1", "made", false)],
            refused_events: vec![event("jmap:E2", "refused", true)],
            deleted: vec!["jmap:E3".into()],
            ..Default::default()
        };
        let mut sent = Sent::default();
        sent.record_events(&sync);
        // Synced again before the engine took that in: the event made is updated, not made again; the refusal is not
        // sent again (nor said); the deletion not sent again; the card's tombstone stays.
        let mut copy = pim.clone();
        sent.resume(&mut copy);
        assert_eq!(copy.events[0].href, "jmap:E1");
        assert!(!copy.events[1].dirty);
        assert_eq!(copy.deleted, vec![Tombstone { href: "jmap:E3".into(), ..Default::default() }]);
        // Once the engine has taken it all in, it is let go of.
        let mut taken = kept(Vec::new());
        taken.events = vec![event("jmap:E1", "made", false)];
        sent.resume(&mut taken);
        assert!(sent.events_created.is_empty() && sent.events_refused.is_empty() && sent.events_deleted.is_empty());
    }

    #[test]
    fn what_was_sent_comes_back_even_when_the_books_cannot_be_read() {
        let mut pim = kept(vec![stored("B1", "", named("u-1@mailcraft", "Jeanne Muster", "jeanne@example.ch"), true)]);
        pim.deleted = vec![Tombstone { href: "jmap:C9".into(), ..Default::default() }];
        for failure in [json!({"error": {"type": "serverFail"}}), json!({"transport": "reset"})] {
            let unreadable = |name: &str, args: &Value| if name == "AddressBook/get" { failure.clone() } else { writable(name, args) };
            let client = connect(json!({}), unreadable);
            let sync = crate::block_on(client.sync_contacts(&pim, false, 9)).unwrap().unwrap();
            assert_eq!(sync.pushed_cards.iter().map(|c| c.href.as_str()).collect::<Vec<_>>(), ["jmap:N1"], "what was stored is handed over");
            assert_eq!(sync.deleted, vec!["jmap:C9"]);
            assert!(sync.books.is_none() && sync.synced_books.is_empty() && sync.cards.is_empty(), "the books unread stay as they are");
            assert_eq!(sync.carddav_home.as_deref(), Some("jmap"));
        }
    }

    #[test]
    fn the_note_goes_where_the_server_keeps_notes() {
        let mut c = named("u@mailcraft", "Jeanne Muster", "jeanne@example.ch");
        c.note = "Prefers the phone".into();
        assert_eq!(card_json(&c, true)["notes"], json!({"n1": {"@type": "Note", "note": "Prefers the phone"}}));
        assert!(card_json(&c, false).get("notes").is_none(), "not to a server that keeps none (Neoffice's)");
        assert_eq!(card_of(&card_json(&c, true)).note, "Prefers the phone", "and read back");
        let mut cleared = c.clone();
        cleared.note.clear();
        assert_eq!(card_json(&cleared, true)["notes"], json!({}), "a note taken away goes there too");
        // Sent with the rest of the card.
        let client = connect(json!({}), writable);
        crate::block_on(client.sync_contacts(&kept(vec![stored("B1", "", c, true)]), true, 9)).unwrap();
        let set = calls(&client, "ContactCard/set").remove(0);
        assert_eq!(set["create"]["k0"]["notes"]["n1"]["note"], json!("Prefers the phone"));
    }

    #[test]
    fn the_organizations_and_titles_not_shown_are_written_back_as_they_were() {
        let read = json!({
            "id": "C1", "uid": "c1", "name": {"full": "Jeanne Muster"},
            "organizations": {
                "org-a": {"@type": "Organization", "name": "Example SA", "units": [{"name": "Finance"}]},
                "org-b": {"@type": "Organization", "name": "Club"}
            },
            "titles": {"t-a": {"@type": "Title", "name": "Comptable", "kind": "title"}, "t-b": {"@type": "Title", "name": "Treasurer", "kind": "role"}}
        });
        let mut card = card_of(&read);
        assert_eq!((card.org.as_str(), card.title.as_str()), ("Example SA", "Comptable"));
        card.org = "Example Group".into();
        let v = card_json(&card, false);
        assert_eq!(
            v["organizations"]["org-a"],
            json!({"@type": "Organization", "name": "Example Group", "units": [{"name": "Finance"}]}),
            "the one shown, renamed, its units kept"
        );
        assert_eq!(v["organizations"]["org-b"]["name"], json!("Club"), "the one not shown, as it was");
        assert_eq!(v["titles"], read["titles"], "unchanged, as read");
        // The title shown taken away: the others stay.
        card.title.clear();
        assert_eq!(card_json(&card, false)["titles"], json!({"t-b": {"@type": "Title", "name": "Treasurer", "kind": "role"}}));
        // A card with just a name for each keeps nothing aside.
        assert!(card_of(&jeanne()).extra.is_empty());
    }

    #[test]
    fn the_cards_of_a_book_no_longer_writable_are_not_sent() {
        let mut pim = kept(vec![stored("B1", "jmap:C7", named("C7", "Paul Exemple", "paul@example.ch"), true)]);
        pim.books[0].read_only = true;
        let client = connect(json!({}), writable);
        let sync = crate::block_on(client.sync_contacts(&pim, false, 9)).unwrap().unwrap();
        assert!(calls(&client, "ContactCard/set").is_empty() && sync.errors.is_empty(), "not refused again at every sync");
    }

    #[test]
    fn an_empty_page_before_the_total_and_the_last_page_bound_are_not_a_whole_read() {
        let args = json!({"accountId": "A1"});
        // The server said four, and gives no more after two: not all of them.
        let short = |_: &str, a: &Value| {
            let at = a["position"].as_u64().unwrap_or(0) as usize;
            let ids: Vec<&str> = ["T1", "T2"].into_iter().skip(at).take(2).collect();
            json!({"ids": ids, "position": at, "total": 4})
        };
        let client = connect(json!({}), short);
        let first = crate::block_on(client.call(&[CORE], vec![("Thing/query", query_page(&args, 0, 10))])).unwrap().remove(0);
        let (ids, whole) = crate::block_on(client.query_rest(&[CORE], "Thing/query", &args, first, 10)).unwrap();
        assert_eq!((ids.len(), whole), (2, false));
        // A server that pages by one: no more pages asked than read.
        let by_one = |_: &str, a: &Value| {
            let at = a["position"].as_u64().unwrap_or(0);
            json!({"ids": [format!("T{at}")], "position": at, "total": 1000})
        };
        let client = connect(json!({}), by_one);
        let first = crate::block_on(client.call(&[CORE], vec![("Thing/query", query_page(&args, 0, 1000))])).unwrap().remove(0);
        let (ids, whole) = crate::block_on(client.query_rest(&[CORE], "Thing/query", &args, first, 1000)).unwrap();
        assert_eq!((ids.len(), whole), (crate::MAX_QUERY_PAGES, false));
        assert_eq!(client.http.asked.borrow().len(), crate::MAX_QUERY_PAGES, "each page asked is read");
    }

    #[test]
    fn a_token_refused_during_the_sync_is_said_as_such() {
        // The server refuses the token while the address books are read (it expired, or died with the desk's session):
        // said as a refused sign-in, so that the mailbox signs in again, rather than swallowed with the books unread.
        let refused_read = |name: &str, args: &Value| if name == "ContactCard/get" { json!({"status": 401}) } else { writable(name, args) };
        let client = connect(json!({}), refused_read);
        let r = crate::block_on(client.sync_contacts(&kept(Vec::new()), false, 9));
        assert!(matches!(r, Err(crate::Error::Auth(_))), "{r:?}");
        // Likewise while what changed here is sent: the change stays to be sent once signed in again.
        let pim = kept(vec![stored("B1", "", named("u1", "Jeanne Muster", "jeanne@example.ch"), true)]);
        let refused_push = |name: &str, args: &Value| if name == "ContactCard/set" { json!({"status": 401}) } else { writable(name, args) };
        let client = connect(json!({}), refused_push);
        let r = crate::block_on(client.sync_contacts(&pim, false, 9));
        assert!(matches!(r, Err(crate::Error::Auth(_))), "{r:?}");
        assert!(calls(&client, "ContactCard/get").is_empty(), "nothing more asked with a refused token");
    }

    #[test]
    fn a_deletion_the_server_refuses_is_said_once_and_the_card_comes_back() {
        let mut pim = kept(Vec::new());
        pim.deleted = vec![Tombstone { href: "jmap:C1".into(), ..Default::default() }, Tombstone { href: "jmap:C7".into(), ..Default::default() }];
        let linked = |name: &str, args: &Value| match name {
            "ContactCard/set" => json!({
                "destroyed": ["C7"],
                "notDestroyed": {"C1": {"type": "forbidden", "description": "This contact is linked to documents"}}
            }),
            _ => writable(name, args),
        };
        let client = connect(json!({}), linked);
        let sync = crate::block_on(client.sync_contacts(&pim, false, 9)).unwrap().unwrap();
        assert_eq!(calls(&client, "ContactCard/set")[0]["destroy"], json!(["C1", "C7"]));
        assert_eq!(sync.deleted, vec!["jmap:C1", "jmap:C7"], "both done with: one deleted, one refused");
        assert_eq!(sync.errors, vec!["a contact was not deleted: not allowed: This contact is linked to documents"], "the refusal said once");
        assert_eq!(sync.refused_deletions, vec!["jmap:C1"], "refused for good: read after a send alone (read_after)");
        // One already gone there (its deletion sent before, its answer lost on the way): done, nothing said.
        let gone = |name: &str, args: &Value| match name {
            "ContactCard/set" => json!({"destroyed": ["C7"], "notDestroyed": {"C1": {"type": "notFound"}}}),
            _ => writable(name, args),
        };
        let again = crate::block_on(connect(json!({}), gone).sync_contacts(&pim, false, 9)).unwrap().unwrap();
        assert_eq!((again.deleted.len(), again.errors.len()), (2, 0), "{:?}", again.errors);
        assert!(sync.cards.iter().any(|c| c.href == "jmap:C1"), "the card refused is read back");
        // A call that fails keeps the deletions for the next sync.
        let failing = |name: &str, args: &Value| if name == "ContactCard/set" { json!({"transport": "reset"}) } else { writable(name, args) };
        let client = connect(json!({}), failing);
        let sync = crate::block_on(client.sync_contacts(&pim, false, 9)).unwrap().unwrap();
        assert!(sync.deleted.is_empty() && sync.errors.len() == 2, "{:?}", sync.errors);
        assert!(sync.refused_deletions.is_empty(), "a call that failed refuses nothing for good");
        // So does one the server could not take for now (it failed, or asked to wait): said where it failed.
        let busy = |name: &str, args: &Value| match name {
            "ContactCard/set" => json!({"notDestroyed": {"C1": {"type": "serverFail"}, "C7": {"type": "rateLimit"}}}),
            _ => writable(name, args),
        };
        let sync = crate::block_on(connect(json!({}), busy).sync_contacts(&pim, false, 9)).unwrap().unwrap();
        assert!(sync.deleted.is_empty() && sync.refused_deletions.is_empty(), "both stay to be sent");
        assert_eq!(sync.errors, vec!["a contact was not deleted: the server failed; tried again at the next sync"]);
    }

    #[test]
    fn a_book_says_whether_its_cards_may_be_deleted() {
        let rights = |id: &str, r: Value| json!({"id": id, "name": id, "myRights": r});
        let books = vec![
            // A standard server's default book: the person may write it (and so destroy its cards), not delete the book.
            rights("B1", json!({"mayRead": true, "mayWrite": true, "mayDelete": false})),
            // Neoffice's facade, for someone who may write contacts but not delete them.
            rights("B2", json!({"mayRead": true, "mayWrite": true, "mayDelete": false, "mayRemoveItems": false})),
            rights("B3", json!({"mayRead": true, "mayWrite": true, "mayRemoveItems": true})),
            rights("B4", json!({"mayRead": true, "mayWrite": false})),
            json!({"id": "B5", "name": "Old"}),
        ];
        let sync = contacts_sync(&books, &[], 1);
        assert_eq!(sync.no_delete_books, vec!["B2", "B4", "B5"], "mayWrite decides, mayRemoveItems narrows it; mayDelete (the book) doesn't count");
    }

    #[test]
    fn a_change_waits_to_be_sent_until_it_goes() {
        use mailcraft_model::ical::CalEvent;
        let event = |calendar: &str, href: &str| StoredEvent {
            calendar: calendar.into(),
            href: href.into(),
            etag: String::new(),
            event: CalEvent { uid: "u1".into(), summary: "Point chantier".into(), ..Default::default() },
            dirty: true,
        };
        let jeanne = |dirty: bool| stored("B1", "jmap:C1", named("c1", "Jeanne Muster", "jeanne@example.ch"), dirty);
        let clean = kept(vec![jeanne(false)]);
        assert!(!waiting(&clean), "nothing changed here");
        // A card changed in a book the person may write waits; in a book now read-only it doesn't (the engine lets it go).
        let changed = kept(vec![jeanne(true)]);
        assert!(waiting(&changed));
        let mut locked = changed.clone();
        locked.books[0].read_only = true;
        assert!(!waiting(&locked));
        // Nor before the books were found on this server (a CardDAV server's).
        let mut elsewhere = changed.clone();
        elsewhere.carddav_home = Some("https://dav.example.ch/".into());
        assert!(!waiting(&elsewhere));
        // A card deleted here waits (its tombstone names no calendar).
        let mut gone = clean.clone();
        gone.deleted = vec![Tombstone { href: "jmap:C1".into(), ..Default::default() }];
        assert!(waiting(&gone));
        // An event changed or deleted in one of the server's calendars waits; one in a calendar kept here only doesn't.
        let mut calendars = clean.clone();
        calendars.caldav_home = Some("jmap".into());
        calendars.events = vec![event("K1", "jmap:E1")];
        assert!(waiting(&calendars));
        calendars.events = vec![event(LOCAL, "")];
        assert!(!waiting(&calendars));
        calendars.deleted = vec![Tombstone { href: "jmap:E3".into(), calendar: "K1".into(), ..Default::default() }];
        assert!(waiting(&calendars));
        // What a sync sent that the engine hasn't taken in yet doesn't wait any more: a refused change, said once.
        calendars.deleted.clear();
        calendars.events = vec![event("K1", "jmap:E2")];
        let mut sent = Sent::default();
        sent.record_events(&DavSync { refused_events: vec![event("K1", "jmap:E2")], ..Default::default() });
        sent.resume(&mut calendars);
        assert!(!waiting(&calendars));
    }

    /// A change the server refused is not sent again before the engine has taken the refusal in; a change made since goes,
    /// and so does everything once the engine has taken in the last sync recorded (a deletion tried again too).
    #[test]
    fn a_change_made_after_a_refusal_goes() {
        use mailcraft_model::ical::CalEvent;
        let event = |summary: &str| StoredEvent {
            calendar: "K1".into(),
            href: "jmap:E1".into(),
            etag: String::new(),
            event: CalEvent { uid: "u1".into(), summary: summary.into(), ..Default::default() },
            dirty: true,
        };
        let mut pim = kept(Vec::new());
        pim.caldav_home = Some("jmap".into());
        pim.events = vec![event("Refused title")];
        let mut sent = Sent::default();
        sent.record_events(&DavSync { checked: 10, refused_events: vec![event("Refused title")], ..Default::default() });
        let mut copy = pim.clone();
        sent.resume(&mut copy);
        assert!(!copy.events[0].dirty && !waiting(&copy), "not sent again before the engine takes the refusal in");
        let mut changed = pim.clone();
        changed.events[0].event.summary = "Corrected title".into();
        sent.resume(&mut changed);
        assert!(changed.events[0].dirty && waiting(&changed), "changed since: it goes");
        let mut taken = pim.clone();
        taken.dav_checked = Some(10);
        sent.resume(&mut taken);
        assert!(taken.events[0].dirty && waiting(&taken), "the engine took the refusal in: the change made since goes");
        assert!(sent.recorded.is_none() && sent.events_refused.is_empty());

        // A card refused for good and a deletion done with, likewise.
        let mut cards = kept(vec![stored("B1", "jmap:C1", named("c1", "Jeanne Muster", "jeanne@example.ch"), true)]);
        cards.deleted = vec![Tombstone { href: "jmap:C2".into(), ..Default::default() }];
        let mut sent = Sent::default();
        sent.record(&DavSync { checked: 20, refused_cards: vec![cards.cards[0].clone()], deleted: vec!["jmap:C2".into()], ..Default::default() });
        let mut copy = cards.clone();
        sent.resume(&mut copy);
        assert!(!copy.cards[0].dirty && copy.deleted.is_empty() && !waiting(&copy), "not sent again before the engine takes them in");
        let mut taken = cards.clone();
        taken.dav_checked = Some(20);
        sent.resume(&mut taken);
        assert!(taken.cards[0].dirty && taken.deleted.len() == 1 && waiting(&taken), "tried again after the engine took them in: they go");
    }

    /// A card's change the server refuses for good is said once and handed over as refused (the change goes); one it
    /// could not take for the time being stays to be sent. A send alone reads no address book.
    #[test]
    fn a_card_refused_for_good_goes_and_a_send_reads_nothing() {
        let pim = kept(vec![
            stored("B1", "jmap:C1", named("c1", "Jeanne Muster", "jeanne@example.ch"), true),
            stored("B1", "jmap:C7", named("c7", "Paul Exemple", "paul@example.ch"), true),
        ]);
        let refusing = |name: &str, args: &Value| match name {
            "ContactCard/set" => json!({"notUpdated": {
                "C1": {"type": "invalidProperties", "description": "phone"},
                "C7": {"type": "serverFail"}
            }}),
            _ => writable(name, args),
        };
        let client = connect(json!({}), refusing);
        let sync = crate::block_on(client.send_contacts(&pim, false, 9)).unwrap().unwrap();
        assert_eq!(sync.refused_cards.iter().map(|c| c.href.as_str()).collect::<Vec<_>>(), ["jmap:C1"], "only the refusal for good");
        assert_eq!(sync.errors.len(), 2, "{:?}", sync.errors);
        assert!(sync.books.is_none() && sync.synced_books.is_empty() && sync.cards.is_empty(), "the books stay as the engine has them");
        let asked: Vec<String> = client.http.asked.borrow().iter().map(|(name, _)| name.clone()).collect();
        assert!(asked.iter().all(|name| name == "ContactCard/set"), "{asked:?}");
    }

    #[test]
    fn a_send_alone_is_read_after_only_a_refusal_for_good() {
        let quiet = DavSync { errors: vec![format!("an event was not saved: Weekly — {FAILED_FOR_NOW}")], ..Default::default() };
        assert!(!read_after(&quiet), "for the time being: sent again, nothing to read");
        let down = DavSync { errors: vec!["an event was not saved: Weekly — network: offline".into()], ..Default::default() };
        assert!(!read_after(&down), "a call that failed: sent again, nothing to read");
        let deletion = DavSync {
            errors: vec!["a contact was not deleted: Ce contact est lié à des documents".into()],
            deleted: vec!["jmap:C1".into()],
            refused_deletions: vec!["jmap:C1".into()],
            ..Default::default()
        };
        assert!(read_after(&deletion), "a deletion refused: the contact comes back");
        let card = DavSync { refused_cards: vec![stored("B1", "jmap:C1", named("c1", "Jeanne", "jeanne@example.ch"), true)], ..Default::default() };
        assert!(read_after(&card));
        assert!(!read_after(&DavSync::default()));
    }
}
