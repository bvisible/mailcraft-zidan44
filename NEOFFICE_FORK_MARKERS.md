<!-- //// Neoffice — added file (no upstream equivalent) -->
# Neoffice fork markers

Map of **our** divergence from upstream (`ZIDAN44/mailcraft`) in this fork (`bvisible/mailcraft`, branch
`version-15`). Everything that can carry a comment carries a `//// Neoffice — …` marker in place:
`// //// Neoffice — …` in Rust (a bare `////` is an error for clippy), `# //// Neoffice — …` in TOML
and YAML, `<!-- //// Neoffice — … -->` in Markdown. This file holds the base of the divergence and
what cannot carry a comment.

At the next upstream merge, `grep -rn "////"` in the source plus this file give the complete
picture of what is ours and why.

- **Upstream**: `ZIDAN44/mailcraft`, branch `main`.
- **Base of the divergence**: `27e59b974c42664ec6a1eb3b9516fe5143361ccf`, upstream's `main` and its
  tag `v0.10.0-test.4`, where `version-15` was cut on 2026-10-16. Move it at each upstream merge. The
  check works it out itself (the common ancestor with upstream's `main`), so this line is for the
  reader.

## What the fork adds

Three workflow files and one feature, all marked in place. The workflow files are new files that carry
their own `//// Neoffice — added file` header.

- `.github/workflows/neoffice-web.yml`: builds the browser version on a GitHub runner, started by
  hand, with upstream's own packaging script (`packaging/web/package.sh`); with `release` ticked, the
  zip and its `SHA256SUMS.txt` become a GitHub release (`web-<version>-<short sha>`). Upstream's
  workflows are disabled on this fork: they need upstream's signing secrets.
- `.github/workflows/neoffice-markers.yml`: fails a push to `version-15` when a change to upstream's
  code since the common ancestor carries no marker.
- `.github/workflows/neoffice-ci.yml`: runs upstream's own quality gate, `cargo xtask ci`, on a
  GitHub runner (by hand, or on a push that touches code).
- **« Reply with Nora »** (Nora is the assistant of the Neoffice server). Only when the page that
  hosts the browser build asks for it (`?nora` or `?nora=<origin>`): a « Nora » button beside Reply,
  Reply all and Forward opens « Reply with Nora » and « Reply all with Nora ». The reply opens as
  usual, the server writes its text from the received message (`nora.api.nora_webmail.generate_email`,
  which runs its injection guard on that message first), and the draft goes in above the signature
  and the quoted message, as one step Undo takes back. Nothing is sent. Every other page, and the
  desktop app, behave exactly as upstream.
  - **« Translate »** (the same menu, French, German or English): the server translates the open message
    (`nora.api.nora_webmail.translate`, injection guard first) and it is shown in place of the message,
    under « Translated by Nora » and its « Show the original ».
  - **The band of the message being written** (under the subject, same condition): « Correct », « Improve »,
    « Translate ». Only the person's own text is sent (above the signature `-- ` and the quoted message's
    rule), and the answer goes in only over that same text, as one step Undo takes back.
  - New file `crates/ui-egui/src/nora.rs` (the button and its menu, the band, the calls, the answers, the
    insertions, the translated view, tests).
  - Marked changes in `crates/ui-egui/src/lib.rs` (the module, `Services::nora_available`,
    `Services::nora_send`, `Services::nora_answers` with defaults that say Nora isn't there,
    `Action::Nora`, the state in `UiState`, a line in the frame), `crates/ui-egui/src/reading.rs`
    (the button, the translated view, the band) and `crates/ui-egui/src/rich_edit.rs` (`insert_at_start`,
    `replace_start`, through the editor's history).
  - Her strings are translated outside upstream's catalogs, in the new file
    `crates/ui-egui/src/i18n/neoffice.rs` (the twelve languages), which every language's catalog adds
    to its own entries (marked lines in `crates/ui-egui/src/i18n/mod.rs`); upstream's coverage test
    reads `nora.rs` too (a marked line in `crates/ui-egui/src/i18n/tests.rs`).
  - The browser build reaches the server with the hosting page's own session: new file
    `apps/mailcraft-web/src/nora.rs` (a `fetch` with the page's Frappe CSRF token, or the framing
    page's when it is of the same origin, and `window.mailcraftNora` for the host's settings); marked
    changes in `apps/mailcraft-web/src/main.rs` and `apps/mailcraft-web/src/web.rs`.

## What cannot carry a comment

Nothing yet: no change to `Cargo.lock`, to an asset or to a catalog (`*.tsv`).
