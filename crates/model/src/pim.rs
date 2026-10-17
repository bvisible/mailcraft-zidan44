//! What an account keeps of its calendars and address books: the server's (CalDAV, CardDAV)
//! with what's needed to sync them, or ones kept on this computer only.

use serde::{Deserialize, Serialize};

use crate::ical::CalEvent;
use crate::vcard::Card;

/// The id of a calendar or address book that lives on this computer only.
pub const LOCAL: &str = "local";

/// Where an account's calendars and address books are kept in its cache.
pub const PIM_DOC: &str = "pim.json";

/// A calendar.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarInfo {
    /// The collection's address on the server, or [`LOCAL`].
    pub id: String,
    pub name: String,
    /// An index into the UI's palette.
    #[serde(default)]
    pub color: u8,
    #[serde(default)]
    pub read_only: bool,
    /// The collection's version at the last sync (`getctag` / sync token).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ctag: String,
}

/// An event with where it lives.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredEvent {
    /// The calendar's id.
    pub calendar: String,
    /// The resource's address on the server (empty until stored there).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub href: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub etag: String,
    pub event: CalEvent,
    /// Changed here and not yet on the server.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dirty: bool,
}

/// An address book.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookInfo {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ctag: String,
}

/// A card with where it lives.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredCard {
    pub book: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub href: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub etag: String,
    pub card: Card,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dirty: bool,
}

/// The id of someone else's calendar opened by their address.
pub fn shared_calendar_id(email: &str) -> String {
    format!("shared:{}", email.trim().to_ascii_lowercase())
}

/// Whose calendar a shared calendar's id is (`shared:<address>`).
pub fn shared_calendar_owner(id: &str) -> Option<&str> {
    id.strip_prefix("shared:").filter(|e| !e.is_empty())
}

/// A note, as Outlook's Notes keeps them: text, and a colour.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    pub uid: String,
    pub text: String,
    /// Outlook's note colours: 0 blue, 1 green, 2 pink, 3 yellow (the usual), 4 white.
    #[serde(default = "yellow")]
    pub color: u8,
    /// When it was last changed (Unix seconds).
    #[serde(default)]
    pub modified: i64,
}

fn yellow() -> u8 {
    NOTE_YELLOW
}

/// The colour a new note takes, as in Outlook.
pub const NOTE_YELLOW: u8 = 3;
/// How many note colours there are.
pub const NOTE_COLORS: u8 = 5;

impl Note {
    /// Its first line: what a list of notes shows as its title.
    pub fn title(&self) -> &str {
        self.text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("")
    }
}

/// A note with where it lives: `href` is its item on the server (Exchange's Notes folder),
/// empty for a note kept on this computer only.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredNote {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub href: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub etag: String,
    pub note: Note,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dirty: bool,
}

/// Something deleted here that is still to be deleted on the server.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tombstone {
    pub href: String,
    #[serde(default)]
    pub etag: String,
    /// The calendar it was in, when that decides where it's deleted (someone else's calendar).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub calendar: String,
}

/// An account's calendars and address books.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pim {
    #[serde(default)]
    pub calendars: Vec<CalendarInfo>,
    #[serde(default)]
    pub events: Vec<StoredEvent>,
    #[serde(default)]
    pub books: Vec<BookInfo>,
    #[serde(default)]
    pub cards: Vec<StoredCard>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deleted: Vec<Tombstone>,
    /// Other people's calendars opened from this account (their addresses, lower-case), as
    /// Outlook's Open Calendar › From Address Book: read from Exchange with the rest.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shared: Vec<String>,
    /// Notes (Outlook's Notes): the server's (Exchange) and those kept here.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<StoredNote>,
    /// Where the server keeps the user's calendars and address books (found once); empty when
    /// it has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caldav_home: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub carddav_home: Option<String>,
    /// When the server was last asked where they are.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dav_checked: Option<i64>,
    // //// Neoffice — the items of a calendar or book read only in part (DavSync::partial_*) that the last reads in a row
    // didn't bring (by their address on the server): one missed PARTIAL_MISSES times goes, as it would from one read whole.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub partial_misses: std::collections::BTreeMap<String, u8>,
    // //// Neoffice — the server's books whose cards the person may not delete (JMAP `myRights.mayDelete` false, as
    // Neoffice's facade says it from the person's rights), as the last read of the books said: the engine refuses it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub no_delete_books: Vec<String>,
}

// //// Neoffice — see `Pim::partial_misses`.
/// How many partial reads in a row may miss an item before it goes.
pub const PARTIAL_MISSES: u8 = 3;

/// What a CalDAV / CardDAV sync found, for the engine to merge into its [`Pim`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DavSync {
    /// The homes found (`None`: the server offers none).
    pub caldav_home: Option<String>,
    pub carddav_home: Option<String>,
    pub checked: i64,
    /// The server's calendars (`None`: no CalDAV).
    pub calendars: Option<Vec<CalendarInfo>>,
    /// The events of the calendars read in full (`synced_calendars`), except those unchanged
    /// (`kept`, by href).
    pub events: Vec<StoredEvent>,
    pub synced_calendars: Vec<String>,
    /// Calendars that didn't change: what the engine has stays.
    pub unchanged_calendars: Vec<String>,
    pub books: Option<Vec<BookInfo>>,
    pub cards: Vec<StoredCard>,
    pub synced_books: Vec<String>,
    pub unchanged_books: Vec<String>,
    /// Items read again unchanged (their etag matched).
    pub kept: Vec<String>,
    /// Changes stored on the server, with their new addresses and etags.
    pub pushed_events: Vec<StoredEvent>,
    pub pushed_cards: Vec<StoredCard>,
    /// The server's notes, read in full (`None`: it keeps none, so notes stay on this
    /// computer), and the notes stored there this time.
    #[serde(default)]
    pub notes: Option<Vec<StoredNote>>,
    #[serde(default)]
    pub pushed_notes: Vec<StoredNote>,
    /// Deletions done (by href).
    pub deleted: Vec<String>,
    /// What couldn't be stored or deleted (tried again next time).
    pub errors: Vec<String>,
    // //// Neoffice — calendars and books read only in part (a JMAP server's past the cap of one sync): their items read
    // are added or updated, and nothing of them goes, where one read in full (`synced_*`) is replaced.
    /// Calendars read in part: their `events` are merged without removing any.
    #[serde(default)]
    pub partial_calendars: Vec<String>,
    /// Books read in part: their `cards` are merged without removing any.
    #[serde(default)]
    pub partial_books: Vec<String>,
    /// The span of time the calendars were read for (from, to), when they were read for one (JMAP's window around
    /// today): what a calendar read in part holds wholly outside it can't come back, and goes.
    #[serde(default)]
    pub window: Option<(i64, i64)>,
    /// Of `books`, those whose cards the person may not delete (`Pim::no_delete_books`).
    #[serde(default)]
    pub no_delete_books: Vec<String>,
}

impl Pim {
    /// Make sure there is somewhere to put events and people.
    pub fn ensure_defaults(&mut self) {
        // //// Neoffice — somewhere they can be added: when every calendar or book is read-only (a JMAP server's, as
        // NeoMail's are), the one on this computer, also when a sync dropped it before.
        if !self.calendars.iter().any(|c| !c.read_only) {
            match self.calendars.iter_mut().find(|c| c.id == LOCAL) {
                Some(local) => local.read_only = false,
                None => {
                    self.calendars.push(CalendarInfo { id: LOCAL.into(), name: "Calendar".into(), color: 0, read_only: false, ctag: String::new() })
                }
            }
        }
        // //// Neoffice — likewise for the address books (« New contact », « Add sender to contacts »).
        if !self.books.iter().any(|b| !b.read_only) {
            match self.books.iter_mut().find(|b| b.id == LOCAL) {
                Some(local) => local.read_only = false,
                None => self.books.push(BookInfo { id: LOCAL.into(), name: "Contacts".into(), read_only: false, ctag: String::new() }),
            }
        }
    }

    // //// Neoffice — what the engine may not change (session/pim.rs, session/invite.rs): the items of a read-only
    // calendar or book stay as the server has them.
    /// Whether the calendar with this id is listed read-only.
    pub fn calendar_read_only(&self, id: &str) -> bool {
        self.calendars.iter().any(|c| c.id == id && c.read_only)
    }

    /// Whether the address book with this id is listed read-only.
    pub fn book_read_only(&self, id: &str) -> bool {
        self.books.iter().any(|b| b.id == id && b.read_only)
    }

    /// The first calendar events can be added to.
    pub fn default_calendar(&self) -> Option<&CalendarInfo> {
        self.calendars.iter().find(|c| !c.read_only)
    }

    pub fn default_book(&self) -> Option<&BookInfo> {
        self.books.iter().find(|b| !b.read_only)
    }

    /// Every event and instance with this uid.
    pub fn event_indices(&self, uid: &str) -> Vec<usize> {
        self.events.iter().enumerate().filter(|(_, e)| e.event.uid == uid).map(|(i, _)| i).collect()
    }
}
