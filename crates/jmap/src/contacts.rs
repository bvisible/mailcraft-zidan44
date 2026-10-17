// //// Neoffice — added file (no upstream equivalent): JMAP for Contacts (RFC 9610), read by the same client as the mail.
//
// A server that offers `urn:ietf:params:jmap:contacts` (Neoffice's mail facade does, for the people of the instance)
// gives its address books and the ids of their cards in one batch, then the cards (JSContact, RFC 9553) as many at a
// time as its `maxObjectsInGet` allows. They come back as a [`DavSync`], the
// shape the engine already merges for CardDAV and Microsoft Graph, so the address book, the autocompletion of the
// recipients and the person card read them unchanged; the browser build, which has no CardDAV, gets its contacts this
// way. Read only for now: the books are marked read-only, and nothing changed here is sent back.

use mailcraft_model::pim::{BookInfo, DavSync, StoredCard};
use mailcraft_model::vcard::{Card, Typed};
use serde_json::{Value, json};

use crate::{CORE, Client, Http, Result, list};

/// The capability, and the type of the account that holds the address books.
pub const CONTACTS: &str = "urn:ietf:params:jmap:contacts";
/// The most cards read in one sync: a company's whole address book, not a directory.
const MAX_CARDS: usize = 5000;
/// What a card is read with: what [`Card`] shows.
const PROPERTIES: [&str; 9] = ["id", "addressBookIds", "uid", "name", "emails", "phones", "organizations", "titles", "notes"];
/// A card's address on this account (its id on the server), apart from those of CardDAV.
pub const HREF_PREFIX: &str = "jmap:";

impl<H: Http> Client<H> {
    /// Whether the session offers address books.
    pub fn offers_contacts(&self) -> bool {
        self.contacts.is_some()
    }

    /// The address books and every card in them, as a sync to merge (`None` when the session offers none).
    pub async fn sync_contacts(&self, now: i64) -> Result<Option<DavSync>> {
        let Some(account) = self.contacts.as_deref() else { return Ok(None) };
        let r = self
            .call(
                &[CORE, CONTACTS],
                vec![
                    ("AddressBook/get", json!({"accountId": account, "ids": null})),
                    ("ContactCard/query", json!({"accountId": account, "limit": MAX_CARDS})),
                ],
            )
            .await?;
        let books = list(r.first(), "list").to_vec();
        let ids: Vec<&str> = list(r.get(1), "ids").iter().filter_map(Value::as_str).take(MAX_CARDS).collect();
        let mut cards = Vec::with_capacity(ids.len());
        for chunk in ids.chunks(self.max_get.max(1)) {
            let got = self
                .call(&[CORE, CONTACTS], vec![("ContactCard/get", json!({"accountId": account, "ids": chunk, "properties": PROPERTIES}))])
                .await?;
            cards.extend(list(got.first(), "list").iter().cloned());
        }
        Ok(Some(contacts_sync(&books, &cards, now)))
    }
}

/// The sync the engine merges, from the server's address books and cards.
pub fn contacts_sync(books: &[Value], cards: &[Value], now: i64) -> DavSync {
    let books: Vec<BookInfo> = books
        .iter()
        .filter_map(|b| {
            let id = b.get("id")?.as_str()?.to_string();
            let name = b.get("name").and_then(Value::as_str).filter(|n| !n.trim().is_empty()).unwrap_or("Contacts").to_string();
            Some(BookInfo { id, name, read_only: true, ctag: String::new() })
        })
        .collect();
    let mut stored = Vec::new();
    for v in cards {
        let Some(id) = v.get("id").and_then(Value::as_str) else { continue };
        let card = card_of(v);
        for book in v.get("addressBookIds").and_then(Value::as_object).into_iter().flat_map(|m| m.keys()) {
            if books.iter().any(|b| &b.id == book) {
                stored.push(StoredCard {
                    book: book.clone(),
                    href: format!("{HREF_PREFIX}{id}"),
                    etag: String::new(),
                    card: card.clone(),
                    dirty: false,
                });
            }
        }
    }
    DavSync {
        carddav_home: Some("jmap".into()),
        checked: now,
        synced_books: books.iter().map(|b| b.id.clone()).collect(),
        books: Some(books),
        cards: stored,
        ..Default::default()
    }
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
        extra: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
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

    #[test]
    fn the_cards_are_read_as_many_at_a_time_as_the_server_allows() {
        let session = json!({
            "capabilities": {"urn:ietf:params:jmap:core": {"maxObjectsInGet": 2}, "urn:ietf:params:jmap:mail": {}, CONTACTS: {}},
            "primaryAccounts": {"urn:ietf:params:jmap:mail": "A1"},
            "apiUrl": "/api/", "downloadUrl": "/d/{accountId}/{blobId}", "uploadUrl": "/u/{accountId}/",
        });
        let card = |id: &str| json!({"id": id, "addressBookIds": {"B1": true}, "emails": {"e": {"address": format!("{id}@example.ch")}}});
        let answers = vec![
            session,
            json!({"methodResponses": [
                ["AddressBook/get", {"list": [{"id": "B1", "name": "Neoffice"}]}, "c0"],
                ["ContactCard/query", {"ids": ["C1", "C2", "C3"]}, "c1"]
            ]}),
            json!({"methodResponses": [["ContactCard/get", {"list": [card("C1"), card("C2")]}, "c0"]]}),
            json!({"methodResponses": [["ContactCard/get", {"list": [card("C3")]}, "c0"]]}),
        ];
        let server = Canned(answers.into(), Vec::new().into());
        let client = crate::block_on(Client::connect(server, "jmap.example.com", "me", "pw", true)).unwrap();
        let sync = crate::block_on(client.sync_contacts(9)).unwrap().unwrap();
        let hrefs: Vec<&str> = sync.cards.iter().map(|c| c.href.as_str()).collect();
        assert_eq!(hrefs, vec!["jmap:C1", "jmap:C2", "jmap:C3"]);
        let asked = client.http.1.borrow();
        let gets: Vec<usize> = asked.iter().skip(1).filter_map(|r| r["methodCalls"][0][1]["ids"].as_array().map(Vec::len)).collect();
        assert_eq!(gets, vec![2, 1], "two cards at a time, as the server allows");
        assert_eq!(asked[0]["using"], json!(["urn:ietf:params:jmap:core", CONTACTS]));
    }

    #[test]
    fn the_books_and_their_cards_come_back_as_a_sync_to_merge() {
        let books = vec![json!({"id": "B1", "name": "Neoffice"}), json!({"id": "B2", "name": " "}), json!({"name": "no id"})];
        let mut in_two = jeanne();
        in_two["id"] = json!("C3");
        in_two["addressBookIds"] = json!({"B1": true, "B2": true, "B9": true});
        let sync = contacts_sync(&books, &[jeanne(), in_two, json!({"no": "id"})], 7);
        let names: Vec<(&str, &str, bool)> = sync.books.iter().flatten().map(|b| (b.id.as_str(), b.name.as_str(), b.read_only)).collect();
        assert_eq!(names, vec![("B1", "Neoffice", true), ("B2", "Contacts", true)], "read only for now; a book with no name is named");
        assert_eq!(sync.synced_books, vec!["B1", "B2"], "every book is read in full: what is gone from them goes");
        let placed: Vec<(&str, &str)> = sync.cards.iter().map(|c| (c.book.as_str(), c.href.as_str())).collect();
        assert_eq!(
            placed,
            vec![("B1", "jmap:C1"), ("B1", "jmap:C3"), ("B2", "jmap:C3")],
            "a card in two books is in both; an unknown book is left out"
        );
        assert_eq!((sync.carddav_home.as_deref(), sync.checked, sync.caldav_home.as_deref()), (Some("jmap"), 7, None));
    }
}
