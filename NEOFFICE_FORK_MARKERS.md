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
  secret, scheme, signature }], language, product }` on itself (MailCraft reads it there or on the same-origin page
  that frames it). MailCraft then opens those mailboxes without a form: their accounts are put in a
  memory store before the session opens (each starts once), they sign in with the given secret
  (`scheme: "bearer"`: `Authorization: Bearer`), and no mail, account or secret is kept between visits, as with
  the sample mailbox. A message's own window is shown in the page (a new window would have no mailbox). `language`
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
- **What is kept between visits.** The interface's settings, in the browser's storage (the People list's columns and
  order, the calendars hidden or side by side, the months of the side pane, when mail is marked read, how notes are
  arranged…); never the mail, the accounts or their secrets, nor what comes of the mail (the senders whose pictures
  load, the conversations ignored, the follow-up dates, the senders moved in or out of Focused), and what is a
  mailbox's (its calendars hidden, its favourite folders, what is collapsed of it) by its address rather than its place
  (new file `crates/ui-egui/src/hosted.rs`, `saved_settings`, `restored_settings`). Each mailbox's signature is the
  one the page gives (`signature: { html, text }`, optional: its text, else its HTML made text), and so is the name
  shown on its mail; one made or changed in NeoMail is kept with the settings (`mailcraft.signatures`,
  `mailcraft.names`, by address) until NeoMail writes it to Neoffice, as long as the page still gives the one it
  replaced (one changed in Neoffice since is Neoffice's). New file `apps/mailcraft-web/src/kept.rs` (built on every
  target, its tests run natively); marked changes in `apps/mailcraft-web/src/web.rs` (`WebShell::save`, `kept_texts`,
  `save_kept`, `ids`).
- **The account page offers what is the person's**: in Options' Info, no « Add account… », and the account's settings
  are the name shown on their mail (« Your name… », `Dialog::AccountName`, hosted.rs) and their signature, not its
  servers, its sign-in or its removal (the page's); « Automatic replies » isn't offered where the server offers none,
  and what a server says of them is in the interface's language (messages.rs) (marked in
  `crates/ui-egui/src/backstage.rs`; `Session::hosted`).
- **A token refused while the address books and calendars sync** goes the way a refusal of the mail's sync does
  (`backend.rs`, `refused`: what was done before is handed over, they are read again with the next sync of
  everything, and a hosted mailbox asks the page for a new token), rather than leaving People empty until the next
  mail check (`sync_contacts` and `sync_calendars` say it as `Error::Auth`).
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
- **Calendars, read** (draft-ietf-jmap-calendars, `crates/jmap/src/calendars.rs`): the events of a
  window around today (90 days back, a year ahead, or as much of it as the server's
  `maxExpandedQueryDuration` allows, back and ahead in the same proportion), each occurrence of a
  repeating event apart, with its start and end in UTC as the server works them out (`utcStart`,
  `utcEnd`: MailCraft has no time-zone database), who organizes and attends it (`organizerCalendarAddress`,
  `participants`), and, for an occurrence, its series there (`baseEventId`, kept in the event's `extra` as
  `X-MAILCRAFT-SERIES:`, with the series' rule as `X-MAILCRAFT-SERIES-RULE:` when the server gives it; marked
  constants in `crates/model/src/pim.rs`). Its keywords (JSCalendar's `keywords`, where iCalendar's `CATEGORIES` go)
  are its categories, which upstream's category colours paint in the calendar, read only where the server keeps them
  (not Neoffice's). A calendar is writable where the person may write its events (`myRights.mayWriteAll` or
  `mayWriteOwn`). The contacts and the calendars make one sync (`pim_sync`), so that neither home is said to be gone
  when only the other was read.
- **Refusals in words**: a change the server refuses is said with what it is, translated (« not found », « not
  allowed » with the server's reason, « too many changes at once », else « refused by the server » with what it
  said: `refusal_words` in `crates/jmap/src/contacts.rs`), never the server's kind of error as it is.
- **Calendars, written** (the owner's decision: NeoMail does what the desk does). Before reading, what was made,
  changed or deleted here goes with `CalendarEvent/set`, as many at a time as `maxObjectsInSet` allows and at most
  50 (Neoffice's facade answers `rateLimit` beyond). A new event goes whole, with its uid (`alreadyExists` and its
  `existingId` taken as that event); a change as a patch of what the person changed since the copy the change started
  from, as read (`Pim::originals`, kept by `Pim::keep_as_read` before the first change and let go of once it is sent or
  refused, marked in `crates/model/src/pim.rs`, `crates/engine/src/session/pim.rs` and `invite.rs`), made to the
  server's copy read just before (`CalendarEvent/get`): what the person didn't change stays as the server has it, and
  what someone else changed meanwhile (an assistant moving the meeting in the desk) is neither undone nor mailed to the
  guests as undone. The event form sends what the person changed in it only, its time whole where any of it changed
  (`EventForm::opened`, marked in `crates/ui-egui/src/pim_dialogs.rs`), so that a change read while it is open stays
  too. A map goes whole, the person's change made to the server's (the locations: the first named one renamed, taken
  out or added; the guests: those the person added or took out; the person's own answer on their entry), as the
  facade takes no patch of a part of a property; a deletion as a destroy. A time made or changed here is written in the
  browser's zone (`start`, local by the browser's own time-zone rules, and `timeZone`, its IANA name from
  `Intl.DateTimeFormat`, `host.rs`), its length as its days on the wall clock there (`D`, which the server reads as
  days of the calendar: 23 or 25 hours across a change of offset) then the time that passes, an occurrence's in its own
  zone (the server refuses another zone for one occurrence), a day-long event as its days (`showWithoutTime`,
  midnight, `P{n}D`); reading stays on `utcStart`/`utcEnd`. An occurrence is changed or deleted alone by its own id (« This occurrence »),
  or for the whole series through its base event (« The whole series »: moved as the occurrence was, the rule sent
  only when changed), which the event form offers (`crates/ui-egui/src/pim_dialogs.rs`, « Apply to »;
  `Session::update_occurrence`, `delete_occurrences`, `find_occurrence`, marked in
  `crates/engine/src/session/pim.rs`, and the occurrence's change for the series carried by
  `X-MAILCRAFT-SERIES-CHANGE:`); a new repeating event sends its `recurrenceRule`. The server sends the meeting mail
  (`sendSchedulingMessages`) where the person organizes the event with guests (not for a draft), and MailCraft then
  sends none itself (`Session::server_schedules`, marked in `crates/engine/src/cmd.rs`); the person's answer to
  another's meeting is mailed by MailCraft, as for any other calendar, the server's copy taking the answer. The
  organizer, the draft state and a series' overrides are never sent (the server keeps them), nor the keywords to
  Neoffice, which keeps none and refuses a set that has them: there, the event form greys the categories, as they look
  and as they act, saying why (« Neoffice keeps no categories on events »), for a new event too. The categories made at
  first (Important, Work, Personal, To do, Later) show in the interface's language wherever they are shown, as
  upstream's Categorize menu already shows them. A refusal is said once, and the change goes (`DavSync::refused_events`,
  marked in `crates/model/src/pim.rs`: an event never stored there goes, one stored there follows the server again); a
  `rateLimit` waits for the next sync, quietly, and a failure for the time being (`serverFail`, `serverUnavailable`,
  `serverPartialFail`: the facade's when the mail server doesn't answer) is said and waits for it too, for a change, a
  new event or a deletion, of an event or of a contact (`Refusal` in `crates/jmap/src/contacts.rs`); a failed call
  stops neither the others nor the read; what a sync did is kept until the engine has taken it in (`Sent`), as for the
  cards. A read of a calendar leaves alone only the part of an event that has a change waiting here (the event itself
  or the same occurrence of it, by its address there or its uid, and an exception for a date taken out here; one never
  stored there, all of its uid, as upstream: `waiting_change` in `crates/engine/src/session/pim.rs`), so that one
  occurrence changed here leaves the others of its series shown, and a CalDAV event's other exceptions come as the
  server has them and are written back with it (upstream held back every part of the uid, and its next write lost
  them). The participants are known by their calendar address as the server reads them (`calendarAddress`, which the
  Suite keeps for each, with no `sendTo` for one it asks no answer of), and a guest the person takes out is told, the
  last one too. A guest's answer taken in from the mail shows on the copies of a Neoffice calendar and is not sent
  there: the server keeps its guests' answers (`apply_invite`). An invitation answered from the mail for an
  event a Neoffice calendar holds already takes the answer on the server's copy, not the invitation's (which the
  organizer may have changed since): an expanded series' occurrences show it, and it goes once, for the series;
  declined, the series goes once, by its base event, as « Delete the series » does (`respond_to_invite`, marked in
  `crates/engine/src/session/invite.rs`); the reading pane knows such an invitation is in the calendar
  (`Session::invite`, `find_in_calendar`), and takes in its organizer's cancellation (the series once, by its base
  event, or that occurrence alone) and a guest's answer (shown on its occurrences) from the mail (`apply_invite`).
- **Paging** (`crates/jmap/src/lib.rs`, `query_rest`, `query_page`): a `/query` is read page after page
  from where each page ends (RFC 8620 §5.5), asking `calculateTotal`, until the total is reached or a page
  comes back empty, following a smaller `limit` the server returns, at most 5000 cards or occurrences and
  100 pages a sync (no page asked that would not be read); an empty page before the total the server gave
  is a read in part. A collection read whole replaces what the engine had of it; one that could not be
  read whole (past the cap) is handed over as read in part (`DavSync::partial_books`,
  `partial_calendars`, marked in `crates/model/src/pim.rs`): what was read is added or updated, and what
  three such reads in a row didn't bring goes (`Pim::partial_misses`), as do the occurrences wholly
  outside the span read (`DavSync::window`) (`crates/engine/src/session/pim.rs`, `apply_dav`).
- **What the server didn't take, and what changed meanwhile.** A change the server could not take for the time
  being goes again at the next sync the person asks for (Send / Receive, or the sign-in after a refusal of the token),
  and three times, two minutes apart, after each sync (`Task::Resend`, `pim_waiting`, `RESENDS` in
  `apps/mailcraft-web/src/backend.rs`): a resend only sends (`send_contacts`, `send_calendars`: nothing read), says
  no failure the last syncs already said (`pim_said`), and is followed by a read when the server refused something for
  good (`read_after`), so that it shows again as the server has it; a hosted mailbox keeps nothing once the page is
  closed. What a sync did is kept until the engine has taken in the last sync recorded (`Sent::recorded` against
  `Pim::dav_checked`, each sync's time later than the one before), and known by its content too (what was stored or
  refused, as it was sent): a change made since goes, even after a refusal. A card's change the server refuses for good
  goes as an event's does (`DavSync::refused_cards`, marked in `crates/model/src/pim.rs`; `apply_dav`), and a stored
  event refused shows as it was read before the change (`Pim::originals`) until the next read. An occurrence with a
  change of its own waiting takes no change for the other scope, said so (`CHANGE_NOT_SENT`, `update_occurrence`;
  an answer to the series goes with another occurrence, `series_carrier` in `invite.rs`), and a change of the series'
  rule waiting stays one. The event form sends the time it opened with (`was`) when the person changes the time:
  an event moved elsewhere meanwhile, and read since, is not moved back over that (`MOVED_MEANWHILE`,
  `moved_meanwhile` in `crates/engine/src/cmd.rs`, comparing the time as the form shows it), and what the form left
  as it showed it keeps its exact instant (`as_left`). The event and contact forms say the engine's messages in the
  interface language (`messages::translate`).
- Marked changes in `crates/jmap/src/lib.rs` (the modules, the accounts that hold them, `max_set`) and
  `apps/mailcraft-web/src/backend.rs` (`pim`: the account's books as the engine keeps them go to
  `sync_contacts`, the read time is noted only when a part was read; the resends and pacing above).

### Read-only collections and new items

- **The engine changes nothing in a read-only calendar or book** (`crates/engine/src/session/pim.rs`,
  `session/invite.rs`): no event or card is added to, changed in or deleted from one. Only the copy that
  can be changed is: the same meeting in a colleague's read-only shared calendar (Exchange) stays as it
  is while the person's own copy is changed, answered or deleted, whichever is listed first
  (`Session::find_event_to_change`, also what the event form opens). An invitation whose event is only in such
  calendars is answered by mail only, and a guest's answer or a cancellation isn't taken into them. A card
  changed here in a book that became read-only since can't be sent: the change goes, said once, and the
  server's card comes back.
- **The calendar and address book on this computer stay** while none of the server's can be added to
  (`apply_dav`), and come back where an earlier sync dropped them (`Pim::ensure_defaults`, marked in
  `crates/model/src/pim.rs`), so that new items always have somewhere to go in the desktop app.
- **Read-only address books are upstream's** since its Exchange public folders (`Session::person_read_only`,
  the refusals in `update_card` and `delete_card`, People saying « In a shared address book: read-only » in place
  of Edit and Delete): a JMAP book the person may not write is one of them. Ours is folded into it, marked: the
  ribbon's greyed Edit and Delete say why on hover (« Your rights do not let you change this contact »); a card
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
- **Deleting one of Neoffice's contacts asks first**: in the browser build with a hosting page's mailboxes, a card
  of the server's book is the company's, shared by the whole team, and the desk never deletes one without asking. The
  card's Delete and the ribbon's open a question saying the person's card goes from Neoffice's contacts for the whole
  team and that an administrator can restore it; Cancel, or Delete (the danger), Escape being Cancel
  (`Dialog::DeleteContact`, marked in `crates/ui-egui/src/dialogs.rs`). **Likewise an event of a Neoffice calendar**
  (`Dialog::DeleteEvent`, read_only.rs `delete_event`): the event's form asks what is removed from the person's Neoffice
  calendar (the event, this occurrence, or every occurrence of the series, each of the form's two buttons asking its
  own) and says its guests are told where the person organizes it with guests; Cancel goes back to the event. Elsewhere, Delete deletes at once, as
  upstream does. A deletion the server refuses (a contact linked to a document) says the server's reason in a toast:
  the toasts showed none any more once the engine kept its fifty notices (`UiState::seen_newest`, marked in
  `crates/ui-egui/src/lib.rs`).
- **The browser build offers no « Mail kept on this computer »** in Options (marked in
  `crates/ui-egui/src/backstage.rs`): it keeps a window of each folder of its own and nothing between visits, and
  the setting did nothing there.
- New file `crates/ui-egui/src/read_only.rs` (the reasons, the question before a contact is deleted, and the tests);
  marked changes in `crates/ui-egui/src/lib.rs`, `people.rs`, `calendar.rs`, `reading.rs` and `pim_dialogs.rs` (the
  controls).

### Daylight saving: every date by its own offset

Upstream shows every date with today's UTC offset, and reads every local time with it: after a change of offset (the
last Sunday of October in Europe), a meeting at 10:00 showed at 11:00 in the week view and in its form, and a person who
moved it by what they saw moved it wrongly. The clock now carries the local zone's changes of offset from three years
back to ten years ahead (`Clock::zone`, `Zone`, marked in `crates/model/src/dates.rs`; outside that span, the offset at
its nearest end, fixed), worked out once a day from the app's own
time-zone rules: the browser's (`Date`, `apps/mailcraft-web/src/web.rs`), the system's on the desktop
(`chrono::Local`, marked in `apps/mailcraft/src/main.rs` and `apps/mailcraft-cli/src/main.rs`), handed to the session
through `Services::zone` (marked in `crates/ui-egui/src/lib.rs`; `Session::set_zone`, `set_clock` keeps them). Every
date is shown with the offset of its own date (`civil`, and so the message list, the reading pane, the calendar's
week, month and day, its forms, the times to send later or snooze until), and every local time is read with it
(`Clock::utc`: the event form's times, a day's midnight `Session::local_midnight`, the calendar's clicks,
`calendar.events`, `SnoozeUntil::at`, the send-later presets), the same whatever today's offset: a local time the
clock skips (the hour summer time starts) is read as the offset before would have it, so it moves on by the change,
and one the clock shows twice (the hour winter time comes back) as the first. A day-long event is its days, never a
time (`AgendaItem::days`, marked in `crates/engine/src/session/pim.rs`, which the calendar draws it by, `days_of` in
`calendar.rs`; the event form's dates; the reading pane's invitation), so that a zone whose clock changes at midnight
leaves it on its own days. An occurrence an iCalendar rule
repeats keeps the hour of its first (`Session::agenda`); the week view draws a time by the wall clock, the day of a
change (23 or 25 hours long) included (`wall_hours`, marked in `crates/ui-egui/src/calendar.rs`); an invitation's
floating time, or one named in a zone the file doesn't describe, is read by its own date's offset
(`ical::parse_floating`, marked in `crates/model/src/ical.rs`; `Session::read_calendar`). Without the rules (a test, a
service that gives none), today's offset for every date, as before.

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
upstream's coverage test reads `nora.rs`, `read_only.rs` and `messages.rs` too, and the labels of the forms' rows
(`form_row`: the fork's « Apply to » had shown in English), marked lines in `crates/ui-egui/src/i18n/tests.rs`. The messages `messages.rs` looks up whole (« the server refused the
user name or password », the hosting page's answers) are in it too.

Ten of upstream's own French sentences put a name right after « de » (« Propriétés de Archives », « De Anne ») and
are said otherwise in the same file (`UPSTREAM_SAID_OTHERWISE`: the English, upstream's sentence, the fork's), never
in `fr.tsv`, which merging upstream then never touches. A test fails when upstream changes one of those sentences
(look at the fork's again: keep it, or drop it if upstream fixed it), and another refuses any French sentence of
either catalog that puts a name right after a word it would elide (« La fiche de Essai » for « d’Essai »).

## What cannot carry a comment

Nothing yet: no change to `Cargo.lock`, to an asset or to a catalog (`*.tsv`).
