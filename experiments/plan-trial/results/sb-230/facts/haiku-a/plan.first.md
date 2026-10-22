# Issue #230: a web control surface on the remote port

Worktree `/Users/sbgragent/.dispatch/worktrees/50e56c62`, HEAD `7fe8949` (Release 0.4.1). Read against the tree and the facts file (`facts/1/repo/facts.md`). Facts cited as "facts, <section>". Schema versions as they are today: `SCHEMA_VERSION` 16 (`src/core/model.rs:12`), `VIEWS_SCHEMA_VERSION` 7 (`src/core/model.rs:612`), `IDENTITY_SCHEMA_VERSION` 1 (`src/ports/remote.rs:17`). None is bumped (see Record).

## Where this departs from the issue

- **The card's one-line excerpt is not on the wire.** The issue asks for the excerpt "the card shows". `SessionView` has `card` (the status word, `control/src/lib.rs:646`) and no excerpt. The card computes its excerpt in the UI from the transcript (`src/ui/cards.rs:558`, `agent_excerpt`). Putting that read on the frame breaks §4.1. The plan's list rows show the status word, `waiting_reason` and `trust_question`. The session view shows the last response, which the API already serves (`/sessions/{id}/last-response` and `/screen`, the content arm of `serve_remote`). The excerpt is a later addition through the content worker.
- **Scope and host name are not readable after pairing.** The issue wants them in the header. Scope is returned only by the `/pair` reply (`src/app.rs:3662`). `/info` is unauthenticated and names the host only (`src/app.rs:3568-3599`). The plan adds one read, `GET /api/v1/device` (Actions and effects).
- **"Revoked" and "identity changed" look like "never paired" on the wire.** All three are the 401 at `src/app.rs:3142-3148`, and key rotation clears `devices` (`src/app.rs:2816`). The plan adds no host error code. The page keeps a per-browser flag and the pinned key (Record) and tells the states apart itself.
- **The issue assumes the `/pair` reply can set a cookie.** It cannot. `respond()` hard-codes `Content-Type: application/json` and has no header parameter (`src/adapters/remote.rs:530-546`), and `RemoteReply` has no headers field (`:389-393`). The plan adds both.
- **"A name from the user agent" needs a header the host does not keep.** `read_request` keeps no `User-Agent` (`src/adapters/remote.rs:444-514`). The page composes the name from `navigator.userAgent` and the host takes `device_name` as it does today (`src/app.rs:3650`).
- **The issue's test plan skips two steps of CLAUDE.md's procedure.** It names `tests/remote.rs` only. The plan also adds a UI test in `tests/ui.rs` (CLAUDE.md, "Adding a feature", step 3). It adds no `SWITCHBOARD_SCRIPT` line, since the pairing window is already reachable through `pairing <code>|close` and the page holds no Rust state to script. It adds no spike directory, which CLAUDE.md asks for before building on a risky mechanism. The browser questions need a device (facts, Spike decision), so the gate is the owner's phone run before the commit merges (Tests).
- **Browser questions the issue leaves open.** Its acceptance accepts "the certificate warning" without saying what the warning is. The certificate has no SAN (`src/adapters/remote.rs:102-103`) and runs to the rcgen default year 4096 (`:91-92`). The plan changes validity (Actions and effects, "Startup certificate").

Confirmed as the issue says: `GET /` returns 404 today (`src/app.rs:3023-3025`); `/api/v1/*` is the only prefix served; the pairing window is 5 minutes and 3 attempts (`src/core/action.rs:972-973`); the QR is JSON with one address (`src/ui/remote.rs:155-166`); `switchboard-remote pair` takes one argument (`client/Sources/switchboard-remote/main.swift:109-111`); the headset pairs by Bonjour and a typed code (`ConnectionManager.lookup` and `pair(_:code:)`, `docs/design.md`, remote client status, "Pairing by the typed code (2026-10-14)"); the screen, last-response and `send` routes exist (`src/app.rs:3044-3117`, `:3159`).

## Record

**Identity (`identity.json`, `src/ports/remote.rs` `Identity`).** Add `#[serde(default)] pub certificate_expires: Option<SystemTime>`. `None` for identities made before this change. The field is additive and read with a default, as `last_seen` is (`src/ports/remote.rs:105`). `IDENTITY_SCHEMA_VERSION` stays 1: an older build ignores the field and never reissues. No bump: CLAUDE.md's `SCHEMA_VERSION` rule covers `model.rs`, and `ports/remote.rs` types are outside it (facts, Tree).

**Devices (`devices.json`, `PairedDevice`, `src/ports/remote.rs:96-107`).** Add `#[serde(default)] pub browser: bool`. It is `false` for every existing record. `Core::new_device` (`src/core/remote.rs:166`) takes `browser: bool` and stores it. The device's scope is the window's scope, Read by default, as today. The record stores the token's SHA-256 only, as today.

**Browser-side state.** Two things, neither a record:
- The token, in a session cookie named `__Host-switchboard-<server_id>` (Trust, token and key). The script never reads it. The `server_id` in the name keeps two instances on one machine apart, since cookies are not port-isolated (facts, Browser protocol, first bullet).
- `localStorage` key `switchboard.pinned`, value `{server_id, spki_sha256}`. Both are public. It is a per-viewer convenience: it lets the page say "revoked" and "identity changed" (UI). Every read and write is in try/catch, and without it the page says "scan the code" (UI).

**Nothing else changes on disk.** `SCHEMA_VERSION` 16 and `VIEWS_SCHEMA_VERSION` 7 are untouched. No `model.rs` type changes. The page files are compiled in and are not records.

## Signals

None new. The page uses the events socket as it is: `hello {epoch, revision}`, `changed` for the `waiting`, `sessions`, `session`, `conversation` and `screen` kinds, `watching`, `removed`, `resync_required` (`docs/remote-client.md` §4.4). A session that starts waiting moves to the top because the `waiting` collection is refetched on its `changed`. No new event kind and no new diff.

## Actions and effects

No new `AppAction` and no new `Effect`. `IdentityLoaded` and `Paired(device)` keep their shapes (`src/core/action.rs:278`).

**Static routes (connection thread).** `connection()` (`src/adapters/remote.rs:770-867`) answers `GET /`, `GET /app.js` and `GET /app.css` after `read_request` and before the rate limit and the frame. The table lives in a new `src/adapters/remote_web.rs`, which `include_str!`s `web/index.html`, `web/app.js` and `web/app.css` (paths relative to the file: `../../web/...`). Any other method or path goes on as today. The response:
- `Content-Type`: `text/html; charset=utf-8`, `text/javascript; charset=utf-8`, `text/css; charset=utf-8`.
- `Content-Security-Policy: default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self' wss:; img-src data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'`. `wss:` is named because `connect-src 'self'` does not cover websocket schemes in all browsers (facts, Browser protocol, CSP bullet).
- `Referrer-Policy: no-referrer` (the page also carries `<meta name="referrer" content="no-referrer">`, since whether the header covers its own subresource loads is unresolved; facts, Unresolved).
- `Cache-Control: no-store`, `X-Content-Type-Options: nosniff`, `Content-Length`, and `Connection: close`.

`Connection: close` is what keeps the pools right. The socket is counted as unauthenticated at accept (`src/adapters/remote.rs:701-707`) and released only when the spawned thread that runs `connection()` returns (`src/adapters/remote.rs:701-720`). A static answer returns at once, so each page resource holds a slot for one round trip, not until the 30 s idle timeout (`:573`). Static answers take no rate window: `Limits::limited` matches only `/pair` and `/info` (`:623-639`), and the four-slot pool bounds concurrency. Static answers never reach the frame, so the 4 ms budget is untouched (facts, Browser protocol; `docs/remote-client.md:404`). The static route never logs the query: `refuse` logs method and path only (`src/app.rs:3762-3766`), and the static path logs nothing.

**Reply writer.** `respond()` takes `headers: &[(String, String)]` and writes them after the fixed head. `RemoteReply` gains `headers: Vec<(String, String)>`, empty for every existing reply. That is a mechanical change at 30 literal sites in `src` and `tests` (grep `RemoteReply {`). `read_request` (`:444-514`) keeps three more headers: `host`, `origin` and `cookie`. `RemoteRequest` (`:360`) gains the same three fields; its 3 literal sites are updated.

**Pairing (`remote_pair`, `src/app.rs:3606-3673`).** The body may carry `browser: true`. A browser pairing:
- creates the device with `browser: true`;
- replies with `{scope, server_id, device_id}` and no `token`, so the script never holds it;
- adds `Set-Cookie: __Host-switchboard-<server_id>=<token>; Path=/; Secure; HttpOnly; SameSite=Strict`, with no `Max-Age` (a session cookie).

A native pairing is unchanged: `token` in the body, no cookie. The SPKI check and the code check are unchanged (`src/app.rs:3636`, `:3646`).

**Authentication (`authenticate`, `src/app.rs:3127-3152`).** Credential order: the bearer header, then the cookie. A cookie is read from the `Cookie` header and matched by the name above. A cookie-authenticated request that is not a `GET`, or that is a WebSocket upgrade, must carry `Origin` equal to `https://` plus the request's `Host`. Otherwise it is refused 403 `origin`. The check covers every state-changing route and the `events` upgrade. Without it, a same-site page on another port of the same IP would ride a `Strict` cookie, since same-site ignores the port (facts, Browser protocol, second bullet), and its WebSocket could read the events, since WebSockets have no CORS read gate. Native clients send no `Origin` and no cookie, so they are unaffected.

**Device read.** `GET /api/v1/device` (authenticated, any scope) answers `{device_id, name, scope, server_name}` with `envelope(200, body, true)`. It is one arm in `serve_remote` (`src/app.rs:3017`), after the `events` arm (`:3036`). It reads the device record and the identity, not a read model.

**Startup certificate.** `make_identity` (`src/adapters/remote.rs:95`) sets `not_after` to 398 days from now, via `OffsetDateTime::from(SystemTime)` (the `time` crate; add `time = "0.3"` to `[dependencies]`, the version `Cargo.lock:4869` already pins). The doc comment at `:91-92` changes. The startup block (`src/app.rs:2786-2856`) already holds the key PEM, and it reissues when `certificate_expires` is `None` or within 30 days: a new certificate over the same key (`rcgen::KeyPair::from_pem`), the same CN, the same SPKI. The clock is read in the app, as the rotation branch already does. The new identity is saved through `save_identity`, then dispatched in `IdentityLoaded`. A reissue keeps the SPKI, so native pins still verify (`docs/remote-client.md:308`). Native verification ignores validity (`src/adapters/remote.rs:189`, `verify_server_cert` takes `_now` and checks only the pin), so the reissue is for browsers alone.

**Trust, token and key (decisions).**

1. **Browser trust: the click-through is the trust step.** The browser's own warning is what the owner accepts, and the paired-devices dialog says so. Reasons: the issue asks for nothing to install; a downloaded profile on iOS is an install step; and browsers cannot pin a site's key, since Chrome removed HPKP "for these reasons" (facts, Browser protocol). What this buys: a genuine page refuses to send the code when the `key` or `id` in the URL differs from `/info`. What it does not buy: an impostor serving its own page answers `/info` and runs its own script, so it passes. The page's key check protects against a relay that does not also serve the page, which is the only case the spec's §5 ordering covers. Before pairing, a non-loopback listener serves `/info`, `/pair` and the three page files, and nothing else (`docs/remote-client.md` §5 amended). **Owner (For the owner, certificate trust):** if the exception is per session, not per host and port, the page says the browser will ask again on each visit, and the pairing window's trust line says so. The plan builds that wording as the fallback and adds no install step.

2. **Token: cookie, not script memory.** The cookie is `HttpOnly; Secure; SameSite=Strict`, named per `server_id`, and checked against `Origin` on state changes and the events upgrade (Actions and effects). The script never holds the token, and `localStorage` never holds it. **Owner (For the owner, token transport):** session or persistent? The plan builds a session cookie, since the spec says tokens do not expire and a session cookie adds no lifetime. A persistent cookie is a `Max-Age` attribute on the same `Set-Cookie`, and the revocation path is the device record, which already exists. **Fallback, if the phone run shows the cookie is not kept under the click-through (unresolved, facts Unresolved):** the token stays in page memory, the page sends it as a bearer on `fetch`, and the WebSocket carries it as a `Sec-WebSocket-Protocol` value. The host reads that header in `read_request`. It may omit the echo, since RFC 6455 §4.2.2 allows it (facts, Browser protocol, third bullet). The Origin check stays. A reload then loses the pairing, and the page says so.

3. **One URL for the QR, the text and the native paste.** The QR and "Copy invite" carry `https://<address>/?code=<code>&key=<spki_sha256>&id=<server_id>`, built by one function, `invite_url`, in `src/adapters/remote.rs`. It is 153 bytes, version 9 at level M (facts, QR), smaller than the JSON form's version 10. `PairingInvite.parse` (`client/Sources/SwitchboardProtocol/Info.swift:114-116`) accepts the URL, which fills `addresses` with its authority, and also the JSON, so text copied from an older build still pastes. The CLI and the headset's paste use `parse` unchanged (`main.swift:109-111`, `ConnectionManager.swift:482-487`). **Owner (For the owner, one QR or two):** the plan keeps JSON parsing as the fallback in case the headset paste is still JSON-bound (unknown; `BrowserView.swift:88-105` is the dev aid, and the owner runs the visionOS build).

4. **No address, no URL.** If `local_address()` is `None`, the pairing window shows the code and the key in text and no QR, as it shows the empty address today (`src/ui/remote.rs:157-164`).

**Client-side logic in the page (not `src/ui/`).** The page reconciles revisions in JavaScript: it keeps the highest `revision` it applied per resource and ignores a reply older than that (`docs/remote-client.md` §4.4, "Subscribe first, then snapshot"). It refetches whole collections on `changed` rather than applying diffs, so the logic is small. This departs from CLAUDE.md's "logic moves to the core" for the page only. **Owner (For the owner, reconcile):** the plan keeps it in the page. If the owner wants it in Rust, the fallback is a projection read, which is a new read model and is allowed only if the page cannot do without it (the issue's rule). The page can do without it.

## UI

**Pairing window (`src/ui/remote.rs:141`, `pairing`).** The QR encodes the invite URL (`:155-166` becomes `invite_url(...)`). The text beside the QR is the same URL, in a selectable monospace label, and "Copy invite" copies it (`:199-206`). Two lines are added:
- "Scanning opens the page in the phone's browser. The browser's certificate warning is the trust step for that browser." (Trust, decision 1.)
- When `identity.listen.lan` is off: "Devices on the local network cannot reach this page. Turn on Allow devices on the local network." **Owner (For the owner, page only with LAN on):** the plan shows the line; the alternative is to show nothing, and a phone then sees a connection error.

**Paired-devices dialog (`src/ui/remote.rs:230`, `devices`).** A browser device gets a "Browser" meta label beside its name. One sentence under the table: "A browser device trusts this Mac's certificate through its own warning. Revoke it to end its access." The Revoke button is unchanged (`RevokeDevice`).

**The page (`web/index.html`, `web/app.js`, `web/app.css`).** Plain HTML, CSS and JS. No inline script, no inline event handler, no `style=` attribute (the CSP forbids them), no external resource, no build step, no dependency. `index.html` carries `<link rel="icon" href="data:,">` so the browser does not request `/favicon.ico`.

- *Startup.* `app.js` reads `code`, `key` and `id` from `location.search`, then calls `history.replaceState(null, '', '/')` before any request. If `code` is present it calls `GET /api/v1/info` and compares `spki_sha256` with `key` and `server_id` with `id`. A mismatch shows "This is not the Mac whose code you scanned. Nothing was sent." and stops. A match posts `POST /api/v1/pair` with `{code, device_name, spki_sha256: key, browser: true}`. On success it writes `switchboard.pinned` in try/catch and loads the view.
- *Device name.* Composed from `navigator.userAgent`: browser (`Edg/`, then `Firefox/`, then `Chrome/` before `Safari/`) and device (`iPhone`, `iPad`, `Macintosh`, `Android`, else none), as `Safari on iPhone`. The host trims it to 64 characters. Known limit: a desktop-mode iPad reports as a Mac (facts, Unresolved), so it is named "Mac"; the owner accepts that or asks for host-side parsing (Owner items).
- *No code and no token.* `GET /api/v1/device` returns 401. The page then calls `/api/v1/info` and compares its key with `switchboard.pinned`:
  - no stored entry: "Scan the code on the Mac to pair this browser.";
  - stored entry, key differs: "This Mac's identity changed. Scan a new code on the Mac to pair again.";
  - stored entry, key the same: "This browser was revoked on the Mac. Scan a new code to pair again.";
  - without storage the first sentence applies to every 401.

  **Owner (For the owner, wording):** one sentence for "no code and no token" and the first state above is the plan's choice.
- *Header.* Device name, scope and the host's name from `/api/v1/device` and `/info`. The reply box is shown only when `scope` is `control`. A `403 scope` on send shows "This device reads only now; the Mac lowered its scope."
- *Views.* Projects from `/api/v1/projects`, sessions per project from `/api/v1/projects/{id}/sessions`, and `/api/v1/waiting`. Waiting sessions first, marked with the status word and `waiting_reason`. A session view reads `/api/v1/sessions/{id}`, then `last-response` when the wire `kind` is the Claude Code kind, else `screen?lines=n`. It sends `watch {kind, id}` for `conversation` or `screen` and refetches on the matching `changed`. The `kind` strings are read from the serialized `SessionKind`, not guessed.
- *Live.* `wss://<host>/api/v1/events` opened after the view loads. `hello` sets the epoch; an epoch change refetches everything. `changed` refetches the named collection. `resync_required` and a closed socket reconnect after a delay and refetch. A reconnect that gets 401 goes to the no-token states above, and that is how revocation shows "on its next request".

**Pairing states and copy.** Owned by the page, as listed above. The text is short and says what to do next.

## Tests

The HTTP tests use the host's own client, as the CLI does: `http()` in `tests/remote.rs:245-298` is extended to return the response headers and to send a `Cookie`, `Origin` and `Host` when asked. Existing helpers, `call()` and the fixture reader, stay as they are.

- `tests/remote.rs`:
  - `the_page_answers_without_a_token_with_its_headers_and_nothing_else`: `GET /`, `/app.js`, `/app.css` return 200 with the exact `Content-Type`, `Content-Security-Policy`, `Referrer-Policy`, `Cache-Control`, `X-Content-Type-Options`, and `Connection: close`; the body is the embedded file. `/index.html`, `/app.css/` and `/` with a non-GET method return the 404.
  - `the_page_asks_only_for_paths_the_host_serves`: parse `href`/`src` in `index.html` and the `/api/v1/` literals in `app.js` (`${...}` replaced by a placeholder). Each one, with a paired control-scope token, must not return the catch-all refusal "not available on this port" (`src/app.rs:3108-3116`).
  - `a_static_answer_closes_its_connection`: the server closes after `GET /` (`read` returns 0), and the unauthenticated count is back to zero, shown by the next accept succeeding at 4 open sockets.
  - `a_browser_pairing_sets_a_cookie_and_returns_no_token`: `POST /api/v1/pair` with `browser: true` inside the window returns the `Set-Cookie` with the exact attributes, no `token` in the body, and the device record with `browser: true`. A native pairing still returns `token` and no cookie.
  - `a_cookie_reads_but_refuses_state_changes_without_its_origin`: with the cookie and no bearer, `GET /api/v1/device` is 200; `POST /api/v1/sessions/{id}/send` without `Origin` is 403 `origin`; with `Origin: https://<Host>` it passes the origin check; the events upgrade with a foreign `Origin` is 403.
  - `device_reads_its_scope_and_name`: `/api/v1/device` returns `{device_id, name, scope, server_name}` for a bearer and for a cookie.
  - `a_revoked_cookie_is_refused_on_its_next_request`: revoke the device through the core, the same cookie then gets 401.
  - `the_invite_url_carries_the_code_key_and_id`: `invite_url(...)` produces `https://<addr>/?code=…&key=…&id=…` with 64 hex and 8 code characters.
  - `the_allowlist_refuses_everything_else_with_no_side_effect` (`:682`): cases added for `/`, `/app.js` and `/app.css` as the static exceptions, and for `/api/v1/device` as allowed.
  - `the_client_fixtures_decode_on_both_sides` (`:1829`): `device.json` is added to `client/Fixtures` (28 files, from 27), and checked for its keys on the Rust side. No Swift type reads it.
- `src/adapters/remote.rs` unit tests: `read_request` keeps `host`, `origin` and `cookie`; `respond` writes headers; the cookie name parses; `remote_web::asset` returns the three files and nothing else; `make_identity` sets `certificate_expires` about 398 days out; `reissue` keeps the SPKI and changes the certificate bytes.
- `src/core/tests.rs`: `new_device` with `browser: true` stores it; `src/app.rs:3659` passes the flag from the body, and the two existing core tests (`src/core/tests.rs:11942`, `:11955`) pass `false`.
- `tests/ui.rs`: `remote_devices_dialog_opens_pairing_and_revokes_a_device` (`:7424`) extended: the QR's text label starts with `https://`; "Copy invite" (`:7474`) copies that URL; the LAN-off line appears when `lan` is off; a browser device row shows "Browser".
- `client/Tests/SwitchboardProtocolTests/FixturesTests.swift`: `PairingInvite.parse` of a URL gives `serverID`, `spkiSha256`, `code` and `addresses == [authority]`; the JSON form still parses. Run with `swift test` in `client/` where the toolchain exists; otherwise the owner runs it.

**Not automated:** the page's JavaScript in a real browser. No browser test tool exists in the repo, and adding one is an owner decision (For the owner, browser test tool). The plan's default is no dependency. The HTTP tests above check everything the page calls and every header it depends on. The rest is the owner's phone run in the acceptance below.

**Gate before merge.** The owner's phone run must show: the click-through holds across a reload (a cookie is kept, the events socket opens, a `send` is accepted); the browser's `Origin` form matches the one the check compares (facts, Unresolved: whether the browser sends `https://<ip>:<port>`); the Referer on `/app.js` is empty; the QR scans at the phone's distance. If the cookie is not kept, the fallback in Trust, decision 2 replaces the cookie branch before merge. The commit does not merge with the cookie unverified.

**Checks.** `cargo test --locked --workspace`, `cargo clippy --locked --workspace --all-targets -- -D warnings`, `cargo fmt --all`, and the Linux arm: `cargo clippy --locked -p dispatch --all-targets --target x86_64-unknown-linux-gnu -- -D warnings` (CLAUDE.md). The new adapter code has no macOS-only path, so the Linux arm should compile unchanged.

**Steps, each leaving the app running.** (1) `Identity.certificate_expires` and `PairedDevice.browser` with their defaults, `new_device`'s parameter, and the core test. (2) The reply writer's headers, `read_request`'s three fields, `remote_web` and the static answers, with the page files as placeholders, plus the static tests. (3) `browser` pairing, the cookie branch, the `Origin` check, and `/api/v1/device`, with their tests. (4) Certificate expiry and the startup reissue, with its tests. (5) `invite_url`, the pairing window and the devices dialog, the Swift `parse`, with their tests. (6) The page files. (7) Docs and status.

## Docs and status

- `docs/remote-client.md` (the spec amendments the plan makes):
  - §4.1: static answers close their connection and take no pool-counted round trip beyond it; the four-slot unauthenticated pool is still the pairing bound.
  - §4.3: rows for `GET /`, `/app.js`, `/app.css` (no auth, before pairing) and `GET /api/v1/device` (read); the allowlist sentence names the three page files.
  - §5: before pairing, `/info`, `/pair` and the page files; the QR and text form are the URL; a browser pairing gets a cookie and no token in the reply; the click-through is the trust step; the Origin check; "identity changed" and "revoked" are page states.
  - §11: the new test names.
- `docs/design.md`, "Remote client (2026-10-12)" (`:3107`): one bullet in "Built so far" for the web control surface (#230), and the known gap "the card's excerpt is not on the wire; the page shows the status word and the last response". If the trust-boundary paragraph (about `:240-256`) lists the pre-pairing paths, it adds the page files.
- `README.md`: Layout table gets `web/index.html`, `web/app.js`, `web/app.css`, and `src/adapters/remote_web.rs`; `src/adapters/remote.rs`'s line (`:383`) gains the static answers and the cookie; the `switchboard-remote pair` line (`:277`) takes the URL.
- `CHANGELOG.md`, under `## [Unreleased]` (`:9`): one line for the user-visible change.

## Owner items

Each has the plan's default in the section where it is decided. These need an answer or a device:

1. Certificate exception per host and port, or per session (Trust, decision 1).
2. Session or persistent cookie (Trust, decision 2).
3. Whether the headset's paste still needs JSON (Trust, decision 3). The plan keeps JSON parsing.
4. Page only with LAN on, with the line in the pairing window (UI).
5. Wording for "no code and no token" (UI).
6. Device name from the page (default) or from the host's User-Agent parse, which needs the header kept (Actions and effects, device name).
7. Reconcile in JavaScript (default) or a Rust projection (UI).
8. A browser test tool as a dev dependency (default: none).
9. Yearly reissue over the same key (default: yes. If no, the startup block skips the reissue and browsers keep the year-4096 certificate, with the Safari risk the facts file leaves open).
10. Restart survival with the signed bundle (`scripts/bundle.sh`): the owner runs it, or the plan marks restart survival unverified.
11. The acceptance run on a phone, below.

**Acceptance (owner, phone on the LAN).** Open the pairing window with LAN on; scan the QR; accept the warning; see the sessions; a session that starts waiting moves to the top without a reload; reply from the phone and see it arrive; revoke the phone in the dialog and see "revoked" on the next request. Items the plan cannot verify here: the cookie under the click-through, the `Origin` form, the Referer, the QR at distance, the socket count a browser opens, and whether a refused socket is retried.

**Known costs.** A yearly reissue may ask the browser for the click again, if the browser keys its exception on the certificate and not the key (unresolved; facts). A `localStorage` clear drops the "revoked" and "identity changed" wording to "scan the code" without losing pairing.

## Contract additions

For the implementer to copy. Additive only; no rename or removal.

- `ports::remote::PairedDevice.browser: bool`, `#[serde(default)]`, `false` for existing records.
- `ports::remote::Identity.certificate_expires: Option<SystemTime>`, `#[serde(default)]`, `None` for existing identities.
- Wire: `GET /` `GET /app.js` `GET /app.css`, unauthenticated, served before pairing; `Content-Type` and the headers above.
- Wire: `GET /api/v1/device`, any scope: `{device_id, name, scope, server_name}`.
- Wire: `POST /api/v1/pair` body may carry `browser: true`; a browser pairing replies without `token` and sets `__Host-switchboard-<server_id>`.
- Wire: a cookie is a credential for every route the bearer reaches; a cookie-authenticated non-`GET` or `events` upgrade must carry `Origin: https://<Host>`, else 403 `origin` (a new error code).
- Wire: the pairing invite is `https://<address>/?code=…&key=…&id=…`; `PairingInvite.parse` takes it and still takes JSON.
- Internal, not contract: `RemoteReply.headers`; `RemoteRequest` gains `host`, `origin`, `cookie`; `Core::new_device` gains `browser`; `make_identity` sets expiry; `adapters::remote_web::asset`; `invite_url`; a direct `time = "0.3"` dependency; `client/Fixtures/device.json`.

**Commit message:** `Serve a web control page on the remote port, paired from a URL`

No attribution trailer goes on the commit: CLAUDE.md forbids tool attribution in anything that reaches GitHub, and CLAUDE.md overrides the harness reminder.
