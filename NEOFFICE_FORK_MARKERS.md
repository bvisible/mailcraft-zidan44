<!-- //// Neoffice — added file (no upstream equivalent) -->
# Neoffice fork markers

Map of **our** divergence from upstream (`ZIDAN44/mailcraft`) in this fork (`bvisible/mailcraft`, branch
`version-15`). Everything that can carry a comment carries a `//// Neoffice — …` marker in place:
`// //// Neoffice — …` in Rust (a bare `////` is an error for clippy), `# //// Neoffice — …` in TOML
and YAML, `<!-- //// Neoffice — … -->` in Markdown. This file holds the base of the divergence, what
the fork adds and what cannot carry a comment.

At the next upstream merge, `grep -rn "////"` in the source plus this file give the complete
picture of what is ours and why.

- **Upstream**: `ZIDAN44/mailcraft`, branch `main`.
- **Base of the divergence**: `0e1420faa3726daefec8e3b374f4c63d7ce04998`, upstream's `main` (no `v0.14` tag yet),
  merged into `version-15` on 2026-10-17; before it `b31066a` (`v0.13.0-test.8`), and the fork was cut on 2026-10-16
  from `27e59b9` (`v0.10.0-test.4`). Move it at each upstream merge. The check works it out itself (the common
  ancestor with upstream's `main`), so this line is for the reader.

## What the fork adds

Three workflow files and the features below, all marked in place. The new files carry their own
`//// Neoffice — added file` header. Every feature but the workflows is for the browser build hosted
by Neoffice (the desk's NeoMail page, or the Neodesk desktop app's window): a page that hosts nothing,
and the desktop app, behave as upstream, except where a section says otherwise.

### Workflows

- `.github/workflows/neoffice-web.yml`: builds the browser version on a GitHub runner, started by
  hand, with upstream's own packaging script (`packaging/web/package.sh`); with `release` ticked, the
  zip and its `SHA256SUMS.txt` become a GitHub release (`web-<version>-<short sha>`). Upstream's
  workflows are disabled on this fork: they need upstream's signing secrets.
- `.github/workflows/neoffice-markers.yml`: fails a push to `version-15` when a change to upstream's
  code since the common ancestor carries no marker.
- `.github/workflows/neoffice-ci.yml`: runs upstream's own quality gate, `cargo xtask ci`, on a
  GitHub runner (by hand, or on a push that touches code).

### Nora, the assistant of the Neoffice server

- **« Reply with Nora »**. Only when the page that hosts the browser build asks for it (`?nora` or
  `?nora=<origin>`), or makes Nora's calls itself (below): a « Nora » button beside Reply, Reply all and
  Forward opens « Reply with Nora » and « Reply all with Nora ». The reply opens as usual, the server writes
  its text from the received message (`nora.api.nora_webmail.generate_email`, which runs its injection
  guard on that message first), and the draft goes in above the signature and the quoted message, as one
  step Undo takes back. Nothing is sent.
  - **« Translate »** (the same menu, French, German or English): the server translates the open message
    (`nora.api.nora_webmail.translate`, injection guard first) and it is shown in place of the message,
    under « Translated by Nora » and its « Show the original ».
  - **The band of the message being written** (under the subject, same condition): « Correct », « Improve »,
    « Translate ». Only the person's own text is sent (above the signature `-- ` and the quoted message's
    rule), and the answer goes in only over that same text, as one step Undo takes back.
  - For a mailbox the page handed over, Nora names the open message by its id on the server (the JMAP id
    behind MailCraft's number for it, `mailcraft_jmap::email_id_of`): she reads it there, and no text of
    it leaves the page. She writes in the style of that mailbox and checks the person may open it
    (`Services::nora_mailbox`, `Services::nora_message`).
  - New file `crates/ui-egui/src/nora.rs` (the button and its menu, the band, the calls, the answers, the
    insertions, the translated view, tests).
  - Marked changes in `crates/ui-egui/src/lib.rs` (the module, the `Services::nora_*` with defaults that
    say Nora isn't there, `Action::Nora`, the state in `UiState`, a line in the frame),
    `crates/ui-egui/src/reading.rs` (the button, the translated view, the band) and
    `crates/ui-egui/src/rich_edit.rs` (`insert_at_start`, `replace_start`, through the editor's history).
  - The browser build reaches the server with the hosting page's own session: new file
    `apps/mailcraft-web/src/nora.rs` (a `fetch` with the page's Frappe CSRF token, or the framing
    page's when it is of the same origin, and `window.mailcraftNora` for the host's settings); marked
    changes in `apps/mailcraft-web/src/main.rs` and `apps/mailcraft-web/src/web.rs`.
  - **`window.mailcraftNoraCall(method, args)`**: a host that holds the person's credential itself
    (Neodesk) offers this function, a promise of `{ status, body }` (`args` the JSON of the arguments).
    Nora is then on without `?nora`, and every call goes through it, so the credential reaches neither
    the hosting page nor MailCraft (`apps/mailcraft-web/src/nora.rs`).

### The mailboxes a hosting page hands over (maintenance#1630)

- **The handover.** The page sets `window.mailcraftJmap = { accounts: [{ email, name, session, username,
  secret, scheme }], language, product }` on itself (MailCraft reads it there or on the same-origin page
  that frames it). MailCraft then opens those mailboxes without a form: their accounts are put in a
  memory store before the session opens (each starts once), they sign in with the given secret
  (`scheme: "bearer"`: `Authorization: Bearer`), and nothing is kept between visits, as with the sample
  mailbox. A message's own window is shown in the page (a new window would have no mailbox). `language`
  is the person's language, which MailCraft draws in for « Automatic »; `product` is the name the person
  reads (Neoffice's « NeoMail »), shown wherever MailCraft names itself, the window's title included
  (`crates/ui-egui/src/i18n/mod.rs`, `rename_product`).
- **Renewing a refused token.** When the server refuses a mailbox's token (it expired, or died with the
  session it was bound to), MailCraft asks the page for new ones through `window.mailcraftJmapRenew()`, a
  promise of `{ accounts: [...] }` as above, once per sign-in: mailboxes refused at the same time share
  one request, and the new set replaces the old one. A mailbox the answer leaves out (`{ accounts: [] }`
  leaves out every one) asks for a reload at once. No answer (a promise that fails, or gives no
  `accounts`), or one that names the mailbox without a secret yet (the page's own credential being
  renewed, the person signing in again in Neodesk), is asked again a minute later, for an hour (60
  times) before the reload message. An answer replaces only the secrets of the mailboxes it gives in
  full: the others keep theirs, and stay signed in as hosted ones. Meanwhile the mailbox shows
  « Connecting », not a password to type; a mailbox started again since it asked sees to itself, and a
  retry asks only for the same start of a mailbox still signed out and still waiting. A change the server
  refused with the token waits for the new one and goes then. What each mailbox does with an answer is in
  the new file `apps/mailcraft-web/src/renewal.rs`, built on every target so that its tests run natively
  (`apps/mailcraft-web/src/backend.rs`: `failed`, `ask_renewal`, `renew`, `connect`, `remote`).
- **Pictures through the page.** « Show pictures » loads remote images with the browser's `fetch`, which
  Neodesk's window may not be allowed to do. A page that offers `window.mailcraftFetchUrl(url)`, a promise
  of `{ status, contentType, bytes }` (`bytes` a `Uint8Array`), fetches them instead; its answer is
  measured before it is copied in (8 MB at most). Without it, nothing changes (`host.rs` `fetch_url`,
  `backend.rs` `fetch_url`).
- **The account button.** Neodesk names the signed-in person in the build's address, as for the
  document editors (`?author=Jeanne%20Muster`, read with `URLSearchParams`, trimmed, 128 characters at
  most). MailCraft then shows, at the right end of its top bar, a round button the size of the bar's
  controls: the theme's accent colour, up to two initials (the first letter of each of the name's first
  two words) in the accent's text colour, the name on hover, as WordCraft's account button. MailCraft's
  top bar is itself the accent colour, so a ring in the initials' colour draws the button's edge. New file
  `crates/ui-egui/src/account.rs`; marked changes in `crates/ui-egui/src/lib.rs` (`UiState::signed_in`,
  the call in the top bar), `apps/mailcraft-web/src/host.rs` (`author`), `apps/mailcraft-web/src/web.rs`
  and `apps/mailcraft-web/Cargo.toml` (the `UrlSearchParams` feature of `web-sys`).
- New file `apps/mailcraft-web/src/host.rs`; marked changes in `apps/mailcraft-web/src/main.rs`,
  `apps/mailcraft-web/src/web.rs` (start-up, the services) and `apps/mailcraft-web/src/backend.rs`.

### Address books and calendars over JMAP

The browser build has no CardDAV nor CalDAV. A JMAP server that offers them (Neoffice's mail facade
does) gives its address books and calendars through the same client as the mail, read with the mail
(at most every ten minutes) and when the engine asks, and handed to the engine as the sync it already
merges for CardDAV, CalDAV and Microsoft Graph: the address book, the completion of recipients, the
person card, the calendar and the day's agenda read them unchanged.

- **Contacts, read** (RFC 9610, `crates/jmap/src/contacts.rs`): the books, then the ids of their cards
  and the cards (JSContact, RFC 9553) as many at a time as `maxObjectsInGet` allows. A book is writable
  where the server lets the person write (`myRights.mayWrite`). A card in two books is placed once, in
  the first of them the person may write (the engine keeps a card once, by its uid). A card without a
  uid takes its id.
- **Contacts, written** (the owner's decision: NeoMail does what the desk does, under the person's
  rights). Before reading, the cards made or changed here in the server's books go back with
  `ContactCard/set`, whole properties (name, emails, phones, organization, title, and the note where the
  server keeps notes: Neoffice's contacts have none, so a hosted mailbox sends none and its form greys the
  field), as many at a time as `maxObjectsInSet` allows; the organizations and titles MailCraft doesn't
  show (more than one, or more than a name) are kept from the read and written back as they were. A new
  card carries its own uid, so that the server recognises a create it already made whose answer was lost
  (`alreadyExists` and its `existingId` are taken as that card). A card deleted here is deleted there
  (`destroy`) where the book lets the person delete its cards (`mayWrite`, narrowed by `mayRemoveItems`, as
  below; elsewhere Delete is greyed with the reason);
  a deletion the server refuses (a contact linked to documents) is said once, and the card comes back.
  The ids created come back as the cards' addresses (`jmap:<id>`); a refusal is said for its card, which
  stays to be sent, and a call that fails stops neither the others nor the read; when the read itself
  fails, what was sent is handed over all the same. One sync of the address books runs at a time for a
  mailbox, and what one did on the server is kept until the engine has taken it in (`Sent`), so that a
  card is never made twice nor a deletion sent twice when syncs follow each other. Saving the contact
  form keeps the kinds of the addresses and numbers already there (work, home, mobile), and the given
  name and surname while the name shown is the same (`card_from`, marked in `crates/engine/src/cmd.rs`).
- **Calendars, read only** (draft-ietf-jmap-calendars, `crates/jmap/src/calendars.rs`): the events of a
  window around today (90 days back, a year ahead, or as much of it as the server's
  `maxExpandedQueryDuration` allows, back and ahead in the same proportion), each occurrence of a
  repeating event apart, with its start and end in UTC as the server works them out (`utcStart`,
  `utcEnd`: MailCraft has no time-zone database), and its keywords (JSCalendar's `keywords`, where iCalendar's
  `CATEGORIES` go) as its categories, which upstream's category colours paint in the calendar. The contacts and
  the calendars make one sync (`pim_sync`), so that neither home is said to be gone when only the other was read.
- **Paging** (`crates/jmap/src/lib.rs`, `query_rest`, `query_page`): a `/query` is read page after page
  from where each page ends (RFC 8620 §5.5), asking `calculateTotal`, until the total is reached or a page
  comes back empty, following a smaller `limit` the server returns, at most 5000 cards or occurrences and
  100 pages a sync (no page asked that would not be read); an empty page before the total the server gave
  is a read in part. A collection read whole replaces what the engine had of it; one that could not be
  read whole (past the cap) is handed over as read in part (`DavSync::partial_books`,
  `partial_calendars`, marked in `crates/model/src/pim.rs`): what was read is added or updated, and what
  three such reads in a row didn't bring goes (`Pim::partial_misses`), as do the occurrences wholly
  outside the span read (`DavSync::window`) (`crates/engine/src/session/pim.rs`, `apply_dav`).
- Marked changes in `crates/jmap/src/lib.rs` (the modules, the accounts that hold them, `max_set`) and
  `apps/mailcraft-web/src/backend.rs` (`pim`: the account's books as the engine keeps them go to
  `sync_contacts`, the read time is noted only when a part was read).

### Read-only collections and new items

- **The engine changes nothing in a read-only calendar or book** (`crates/engine/src/session/pim.rs`,
  `session/invite.rs`): no event or card is added to, changed in or deleted from one. Only the copy that
  can be changed is: the same meeting in a colleague's read-only shared calendar (Exchange) stays as it
  is while the person's own copy is answered or deleted. An invitation whose event is only in such
  calendars is answered by mail only, and a guest's answer or a cancellation isn't taken into them. A card
  changed here in a book that became read-only since can't be sent: the change goes, said once, and the
  server's card comes back.
- **The calendar and address book on this computer stay** while none of the server's can be added to
  (`apply_dav`), and come back where an earlier sync dropped them (`Pim::ensure_defaults`, marked in
  `crates/model/src/pim.rs`), so that new items always have somewhere to go in the desktop app.
- **Read-only address books are upstream's** since its Exchange public folders (`Session::person_read_only`,
  the refusals in `update_card` and `delete_card`, People saying « In a shared address book: read-only » in place
  of Edit and Delete): a JMAP book the person may not write is one of them. Ours is folded into it, marked: a card
  of a book whose server doesn't let the person delete its cards (`Pim::no_delete_books`) isn't deleted, and
  People and the ribbon grey Delete with the reason (« Your rights do not let you delete this contact »). A JMAP
  book's cards may be deleted where the person may write it (`myRights.mayWrite`: RFC 9610 puts destroying cards
  there, `mayDelete` being about the book itself), unless the server narrows it with a non-standard
  `mayRemoveItems: false` (Neoffice's facade gives it to someone who may write contacts but not delete them).
- **Nothing kept, nothing offered**: the browser build with a hosting page's mailboxes keeps nothing
  between visits, so the calendar and the book on this computer would lose what is put there at the next
  reload (`Session::keeps_local_pim`, off there, set in `apps/mailcraft-web/src/web.rs`). There, « New
  contact », « Add sender to contacts » and « New event » go to a writable server collection only (a new
  contact to the mailbox that offers one, a Neoffice instance's contacts being offered in one of the
  person's mailboxes), and are greyed otherwise, with the reason on hover (« Your rights do not let you add
  contacts »; « Your rights do not let you add events » where the server's calendars can't be added to,
  « MailCraft cannot add events yet », which reads « NeoMail » under Neoffice, where there is none); an
  accepted invitation with nowhere to go is answered by mail only, and the calendar on this computer,
  empty, isn't offered (`Session::calendar_listed`: neither in the side pane nor in upstream's side-by-side
  columns). The desktop app, a browser build that keeps its mail in the browser, and the sample
  mailbox are unchanged.
- New file `crates/ui-egui/src/read_only.rs` (the reasons and the tests); marked changes in
  `crates/ui-egui/src/lib.rs`, `people.rs`, `calendar.rs` and `reading.rs` (the controls).

### The engine's messages in the interface language

The engine and the network layers write their messages in English, some with values in them. The toasts
and a mailbox's « Disconnected: … » status show them through the new file `crates/ui-egui/src/messages.rs`:
a message the catalogs have whole is translated as it is, and the known shapes of the others (a change the
server refused, the transport's kinds of error, a sign-in refused, a mailbox that needs its password, the
hosting page's answers, the calendars' and contacts' problems) are translated with their values put back.

### Translations

The fork's interface strings are translated outside upstream's catalogs, in the new file
`crates/ui-egui/src/i18n/neoffice.rs` (every language of the catalogs, thirteen besides English), which
every language's catalog adds to its own entries (marked lines in `crates/ui-egui/src/i18n/mod.rs`);
upstream's coverage test reads `nora.rs`, `read_only.rs` and `messages.rs` too (a marked line in
`crates/ui-egui/src/i18n/tests.rs`). The messages `messages.rs` looks up whole (« the server refused the
user name or password », the hosting page's answers) are in it too.

## What cannot carry a comment

Nothing yet: no change to `Cargo.lock`, to an asset or to a catalog (`*.tsv`).
