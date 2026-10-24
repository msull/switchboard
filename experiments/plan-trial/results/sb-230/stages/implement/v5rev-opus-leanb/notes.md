# Implement notes: sb-230, v5rev-opus-leanb

Branch `stage/sb-230-implement-v5rev-opus-leanb`, one commit as the plan
says:

- `4b3a20c` Serve a web control surface from the remote port, paired from
  a URL whose fragment carries the code (the plan's message, with the
  plan's contract list in the body). Not pushed, no PR.

Before the commit: `cargo fmt --all`, `cargo clippy --locked --workspace
--all-targets -- -D warnings` (clean), `cargo test --locked --workspace`
(all pass; `tests/remote.rs` 52, lib 536, ui 205). The pre-commit hook ran
too. `swift test --filter FixturesTests` passes (5 tests, the new URL
parse among them); the rest of the Swift suite was not run. No
`cfg(target_os)` arm was touched, so the Linux cross-clippy of `dispatch`
was not needed. `git status` is empty after the commit.

## What was built

- **Transport (`src/adapters/remote.rs`).** `RemoteRequest` gains
  `cookie`, `origin`, `host` (read in `read_request`) and a
  `cookie(name)` lookup by exact name. `RemoteReply` gains
  `headers: Vec<(&'static str, String)>` and `RemoteReply::new`; every
  struct literal in `app.rs` and `connection` now goes through `new`.
  `respond` writes the extra headers plus `Cache-Control: no-store` and
  `X-Content-Type-Options: nosniff` on every JSON reply. `Limits.page`
  (120 a minute). In `connection`, an exact `GET` for a page path is
  answered there (`respond_page`: CSP, no-store, nosniff, no-referrer,
  `Connection: close`), followed by a TLS close_notify and the end of
  the connection, so the slot frees at once.
- **`src/adapters/web.rs`** (new): the `PAGES` table (path, type,
  `include_str!`), `CONTENT_SECURITY_POLICY`, `PAGE_HEADERS`, `page()`;
  unit tests for the exact match, the page referencing only its own
  files or `data:`, no inline script/style/handler, no
  `innerHTML`/`eval`/`document.write` in the script, and a runner for
  `node --test web/app.test.js` that skips when node is absent (checked
  that it is not vacuous by breaking an assertion: the Rust test failed).
- **Host (`src/app.rs`).** `authenticate` takes the bearer, else the
  cookie `__Host-sb-<server_id>`; a cookie-only non-GET or upgrade needs
  `Origin == https://<Host>`, else 403 `origin` (refused unauthenticated,
  before `DeviceSeen`). `/pair` with `"transport": "cookie"` omits
  `token` from the body and sets
  `Set-Cookie: __Host-sb-<id>=<token>; Max-Age=34560000; Path=/; Secure; HttpOnly; SameSite=Strict`.
  `GET /api/v1/me` answers `{device_id, name, scope, server_id,
  server_name}`; not added to `capabilities`.
- **Core.** `core::remote::invite_url(identity, window, Option<IpAddr>,
  port)`: `https://<addr>:<port>/#code=…&key=…&id=…`, the loopback when
  `listen.lan` is off or there is no address, IPv6 bracketed. Core test.
- **Dialog (`src/ui/remote.rs`).** QR and "Copy invite" carry the URL;
  a Mac-only port shows "The port answers this Mac only; allow devices
  on the local network to pair a phone" in place of the address; the
  browser-trust sentence beside the redaction one. UI test rewritten.
- **Swift.** `PairingInvite.parse` accepts the URL (authority read as
  text so an IPv6 bracket survives) and still the JSON; a URL without
  `code`, `key` or `id` throws. Test added.
- **Page (`web/`).** `index.html`, `app.css`, `app.js`, `app.test.js` as
  the plan's UI section describes: fragment parse then `replaceState`,
  `/info` comparison, cookie pairing, `/me` on boot, the 401 inference
  from `/info`, list grouped and waiting first, last response for
  `kind: claude` else screen, reply box under `control` with no
  `prompt_refusal`, random-hex op kept across a retry, 3 s poll of
  `/waiting` (20 s while the socket is up), socket retry 5 s doubling to
  60 s, device name from the user agent. All routes in one `API` table
  of `'METHOD /path'` strings that `tests/remote.rs` reads.
- **Host tests (`tests/remote.rs`).** Page files and headers to an
  unpaired client, the JSON 404 for `/favicon.ico`, `POST /`, `/api`, a
  query not echoed; every route in the page's table answered with
  something other than `not_found`, and every `/api/v1/` in `app.js` in
  the table; cookie pairing end to end; two hosts' cookies kept apart and
  the bare `__Host-sb` refused; the `Origin` rule on send and on the
  upgrade (`subscribe_at` refactored over a new `subscribe_with` that
  takes headers); `/me` read → control → 401 after revoke with `/info`
  unchanged; rotation test now also checks `/info` gives the same id
  with another key; a page load freeing its slot; a page request failing
  at accept behind four stalled strangers; the 121st page request 429
  while `/info` and `/pair` are unaffected.
- **Spike `spikes/23-web-page/`** (README only) and its row in
  `spikes/README.md`.
- **Docs.** `docs/remote-client.md` §0 dated paragraph, §4.1 bound,
  §4.3 rows and the "anything else is `not_found`" sentence, the excerpt
  sentence at the old line 207 corrected, §5 (pre-pairing surface, QR as
  URL, token wording, a new "A browser" bullet with the cookie, `Origin`
  rule and the weaker trust, the spike-only line); `docs/design.md` dated
  bullet at the end of "Remote client" with its known gaps; `README.md`
  Layout rows and the dev-aid sentence; `client/README.md` invite
  wording; `CHANGELOG.md` Unreleased with the plan's line.

## Unresolved facts: built to the plan's fallback, not settled

- **Unresolved 1 (click-through for a SAN-less certificate).** Built
  the plan's default: certificate untouched, click-through is the trust
  step, said in the dialog without promising which browsers allow it.
  Note: facts.md's Settled "Certificate trust" says the plan "must build
  to" an IP SAN; the finalized plan explicitly chose not to (owner's
  choice) and I followed the plan.
- **Unresolved 2 (`wss://` on iOS).** Polling is the base; the socket
  only hastens it.
- **Unresolved 3 (cookie kept on a self-signed IP host).** Cookie
  transport built; the token-in-page fallback is not built (owner's
  call). Not tested in a browser.
- **Unresolved 4 (`replaceState` and history).** The page clears the
  fragment first thing; the code's single use is the rest.
- **Unresolved 6 (browser connection count).** Measured the host side
  only (below); the browser's own count is not measured.

## Spike

The browser questions (cookie storage and send, two ports' cookies, CSP
and `wss:`) were **not run**: no display session could be driven and the
repo forbids keystroke injection. The README gives the commands for the
owner. Five parallel connections against the test harness, five rounds,
were run with a temporary test (removed before the commit): every round
four got 200 and the fifth was reset at accept.

## What the plan got wrong or left out

- **Parallel requests from the page.** The plan's list step ("`/projects`,
  then `/projects/{id}/sessions` each") did not say sequentially, and
  the spike shows a fan-out would break: connections count as
  unauthenticated until their first authenticated reply, and a fifth at
  once is closed. `app.js` reads the sessions one project at a time on
  its kept connection; recorded in the spike and §4.1.
- **"About twenty literals".** There were 11 `RemoteReply` struct
  literals (10 in `app.rs`, 1 in `connection`), not about twenty.
- **`core::remote` was private.** `mod remote;` in `src/core/mod.rs` had
  to become `pub mod remote;` for the plan's `core::remote::invite_url`
  path.
- **`serve_remote` length.** Adding the `/me` arm took it to 101 lines,
  over pedantic `too_many_lines`; added a scoped
  `#[allow(clippy::too_many_lines)] // one arm per route`, the same
  pattern as `core/control.rs`.
- **`invite_url` signature.** The plan wrote `(identity, window,
  address, port)`; I made the address `Option<IpAddr>` and put the
  `listen.lan` choice inside it, so the UI holds no logic and the
  loopback case is testable in the core, as the plan's tests ask.
- **`subscribe_at` "takes extra headers".** Kept its signature and added
  `subscribe_with(port, headers, path) -> Result<Subscriber, String>`
  under it, so the refused upgrade can be asserted as an `Err`.
- **Node in this stage's shell.** `node --test` could not be run directly
  here (permission), but it runs through `cargo test`; verified that a
  failing JS assertion fails the Rust test.
