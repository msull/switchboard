# Issue #230: a web control surface on the remote port

Worktree `/Users/sbgragent/.dispatch/worktrees/50e56c62`, HEAD `7fe8949` (Release 0.4.1). Facts cited as "facts, <section>" from `facts/1/repo/facts.md`. Schema versions as they are today: `SCHEMA_VERSION` 16 (`src/core/model.rs:12`), `VIEWS_SCHEMA_VERSION` 7 (`src/core/model.rs:612`), `IDENTITY_SCHEMA_VERSION` 1 (`src/ports/remote.rs:17`). None is bumped (see Record).

## Where this departs from the issue

- **The card's one-line excerpt is not on the wire.** The issue asks for the excerpt "the card shows". `SessionView` has `card` (the status word, `control/src/lib.rs:646`) and no excerpt. The card computes its excerpt in the UI from the transcript (`src/ui/cards.rs:558`, `agent_excerpt`). Putting that read on the frame breaks §4.1. The list rows show the status word, `waiting_reason` and `trust_question`. The session view shows the last response, which the API already serves (`/sessions/{id}/last-response` and `/screen`).
- **Scope and host name are not readable after pairing.** The issue wants them in the header. Scope is returned only by the `/pair` reply (`src/app.rs:3662`), and `/info` names the host only (`src/app.rs:3568-3599`). The plan adds `GET /api/v1/device`.
- **"A name from the user agent" needs a header the host does not keep.** `read_request` keeps no `User-Agent` (`src/adapters/remote.rs:444-514`). The page composes the name from `navigator.userAgent`, and the host takes `device_name` as it does today (`src/app.rs:3650`).

## Record

**Identity (`identity.json`, `src/ports/remote.rs` `Identity`).** Add `#[serde(default)] pub certificate_expires: Option<SystemTime>`, `None` for identities made before this change. `IDENTITY_SCHEMA_VERSION` stays 1.

**Devices (`devices.json`, `PairedDevice`, `src/ports/remote.rs:96-107`).** Add `#[serde(default)] pub browser: bool`, `false` for every existing record. `Core::new_device` (`src/core/remote.rs:166`) takes `browser: bool` and stores it. The scope is the window's scope, Read by default, as today. The record stores the token's SHA-256 only, as today.

**Browser-side state.** Two things, neither a record:
- The token, in a session cookie named `__Host-switchboard-<server_id>` (Trust, decision 2). The script never reads it. The `server_id` in the name keeps two instances on one machine apart, since cookies are not port-isolated (facts, Browser protocol).
- `localStorage` key `switchboard.pinned`, value `{server_id, spki_sha256}`. Both are public. It is a per-viewer convenience, read and written in try/catch. Without it the page asks for a new code on every 401.

## Signals

None new. The page uses the events socket as `docs/remote-client.md` §4.4 defines it.

## Actions and effects

No new `AppAction` and no new `Effect`. `IdentityLoaded` and `Paired(device)` keep their shapes (`src/core/action.rs:278`).

**Pool.** `MAX_UNAUTHENTICATED` (`src/adapters/remote.rs:567`) goes from 4 to 8. A page load holds at most six sockets, Chrome's per-host limit (facts, Unresolved), so two remain for `/pair`. Safari and Firefox are unmeasured; Spike 23 checks them. Test: `pairing_succeeds_while_six_page_sockets_are_open` (Tests). **Owner (For the owner, unauthenticated cap).** The owner confirms the §4.1 amendment.

**Static routes (connection thread).** `connection()` (`src/adapters/remote.rs:770-867`) answers `GET /`, `GET /app.js` and `GET /app.css` after `read_request` and before the rate limit and the frame. The table lives in a new `src/adapters/remote_web.rs`, which `include_str!`s `web/index.html`, `web/app.js` and `web/app.css`. Any other method or path goes on as today. The response:
- `Content-Type`: `text/html; charset=utf-8`, `text/javascript; charset=utf-8`, `text/css; charset=utf-8`.
- `Content-Security-Policy: default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self' wss:; img-src data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'`.
- `Referrer-Policy: no-referrer`, with `<meta name="referrer" content="no-referrer">` in the page as well.
- `Cache-Control: no-store`, `X-Content-Type-Options: nosniff`, `Content-Length`, and `Connection: close`.

`Connection: close` keeps the pool short-lived for the page's resources.

**Reply writer.** `respond()` (`src/adapters/remote.rs:530`) gains a `headers: &[(&str, &str)]` parameter. It builds the fixed head from two defaults, `Content-Type: application/json` and `Connection: keep-alive`, and a caller's header with the same name replaces the default. A static answer passes `Content-Type` and `Connection: close` and gets exactly one of each. The four call sites in `connection()` pass `&[]`. `RemoteReply` gains `headers: Vec<(String, String)>`, and the `/pair` reply uses it for `Set-Cookie`.

**Request.** `read_request` (`src/adapters/remote.rs:444-514`) keeps `host` and `origin` as values and never keeps the raw `Cookie` header. It extracts the values of the cookies whose names start with `__Host-switchboard-` into `browser_cookies: Vec<(String, String)>` (name, token), the way it already extracts the bearer. `RemoteRequest` (`:360`) gains `host`, `origin` and `browser_cookies`.

**Pairing (`remote_pair`, `src/app.rs:3606-3673`).** The body may carry `browser: true`. A browser pairing:
- creates the device with `browser: true`;
- replies with `{scope, server_id, device_id}` and no `token`, so the script never holds it;
- adds `Set-Cookie: __Host-switchboard-<server_id>=<token>; Path=/; Secure; HttpOnly; SameSite=Strict`, with no `Max-Age` (a session cookie).

A native pairing is unchanged: `token` in the body, no cookie.

**Authentication (`authenticate`, `src/app.rs:3127-3152`).** Credential order: the bearer header, then a `browser_cookies` entry named `__Host-switchboard-<server_id>`. A cookie-authenticated request that is not a `GET`, or that is a WebSocket upgrade, must carry an `Origin` whose scheme is `https` and whose authority equals the `Host` header. The comparison is on the parsed scheme and authority, with a `:443` suffix ignored on both sides. Refused 403 `origin`: a missing `Origin`, `Origin: null`, and any other scheme or authority. The check covers every state-changing route and the `events` upgrade.

**Device read.** `GET /api/v1/device` (authenticated, any scope) answers `{device_id, name, scope, server_name}` with `envelope(200, body, true)`. It is one arm in `serve_remote` (`src/app.rs:3017`), after the `events` arm (`:3036`). It reads the device record and the identity, not a read model.

**Startup certificate.** `make_identity` (`src/adapters/remote.rs:95`) sets `not_after` to 398 days from now, via `OffsetDateTime::from(SystemTime)` (add `time = "0.3"` to `[dependencies]`; `Cargo.lock:4869` already pins it). The doc comment at `:91-92` changes. The startup block (`src/app.rs:2786-2856`) already holds the key PEM. It reissues when `certificate_expires` is `None` or within 30 days: a new certificate over the same key (`rcgen::KeyPair::from_pem`), the same CN, the same SPKI. The reissue runs only when `self.core.read_only()` is false. The new identity is saved through `save_identity`, then dispatched in `IdentityLoaded`.

**Trust, token and key (decisions).**

1. **Browser trust: the click-through is the trust step.** The browser's own warning is what the owner accepts, and the paired-devices dialog says so. Before pairing, a non-loopback listener serves `/info`, `/pair` and the three page files, and nothing else (`docs/remote-client.md` §5 amended). The page's key check protects against a relay that does not also serve the page.
   - **Owner (For the owner, certificate trust).** If the exception is per session, not per host and port, the page says the browser will ask again on each visit, and the pairing window's trust line says so. The plan builds that wording and no install step.
   - **Hard fail.** If the phone run shows the browser refuses the certificate with no way to proceed, the trust step does not exist, and the gate fails. The plan does not build the browser branch then. The pairing window's line becomes "This browser refused the Mac's certificate and offered no way past it", and the owner decides between a certificate download the host serves at a new static path and dropping the browser surface. Either is a new plan.
   - **No SAN, by decision.** The certificate has no SAN for an address.

2. **Token: cookie, not script memory.** The cookie is `HttpOnly; Secure; SameSite=Strict`, named per `server_id`, and checked against `Origin` on state changes and the events upgrade. The script never holds the token, and `localStorage` never holds it. Spike 23 (Steps, step 0) decides whether the cookie is kept under the click-through. If Spike 23 shows the cookie is not kept, the plan is reopened.
   - **Owner (For the owner, token transport).** Session or persistent? The plan builds a session cookie. A persistent cookie is a `Max-Age` attribute on the same `Set-Cookie`, and the revocation path is the device record.

3. **One URL for the QR, the text and the native paste.** The QR and "Copy invite" carry `https://<address>/?code=<code>&key=<spki_sha256>&id=<server_id>`, built by one function, `invite_url`, in `src/adapters/remote.rs`. `PairingInvite.parse` (`client/Sources/SwitchboardProtocol/Info.swift:114-116`) accepts the URL, which fills `addresses` with its authority, and also the JSON. The CLI's one-argument path calls `parse` (`client/Sources/switchboard-remote/main.swift:111`) and is unchanged. The headset's paste calls `parse` at `client/Sources/SwitchboardUI/BrowserView.swift:883`, and `ConnectionManager.pair(invite:)` (`client/Sources/SwitchboardClientCore/ConnectionManager.swift:482`) takes an already parsed invite, so it needs no change.
   - **Owner (For the owner, one QR or two).** The plan keeps JSON parsing in case the headset's paste or another client is still JSON-bound. The owner runs the visionOS build to check.

4. **No address, no URL.** If `local_address()` is `None`, the pairing window shows the code and the key in text and no QR, as it shows the empty address today (`src/ui/remote.rs:157-164`).

**Client-side logic in the page (outside `src/ui/`).** The page keeps the highest `revision` it has applied per resource and ignores a reply older than that. The envelope carries `revision` on every reply (`src/app.rs:3717-3741`), and `docs/remote-client.md` §4.4 ("Subscribe first, then snapshot") puts the reconcile in the client. The page refetches whole collections on `changed` rather than applying diffs. **Owner (For the owner, reconcile).** The plan keeps it in the page; a Rust projection would be a new read model.

## UI

**Pairing window (`src/ui/remote.rs:141`, `pairing`).** The QR encodes the invite URL (`:155-166` becomes `invite_url(...)`). The text beside the QR is the same URL, in a selectable monospace label, and "Copy invite" copies it (`:199-206`). Two lines are added:
- "Scanning opens the page in the phone's browser. The browser's certificate warning is the trust step for that browser."
- When `identity.listen.lan` is off: "Devices on the local network cannot reach this page. Turn on Allow devices on the local network." **Owner (For the owner, page only with LAN on).** The plan shows the line; the alternative is to show nothing, and a phone then sees a connection error.

**Paired-devices dialog (`src/ui/remote.rs:230`, `devices`).** A browser device gets a "Browser" meta label beside its name. One sentence under the table: "A browser device trusts this Mac's certificate through its own warning. Revoke it to end its access." The Revoke button is unchanged (`RevokeDevice`).

**The page (`web/index.html`, `web/app.js`, `web/app.css`).** Plain HTML, CSS and JS. No inline script, no inline event handler, no `style=` attribute, no external resource, no build step, no dependency. `index.html` carries `<link rel="icon" href="data:,">` so the browser does not request `/favicon.ico`.

- *Startup.* `app.js` reads `code`, `key` and `id` from `location.search`, then calls `history.replaceState(null, '', '/')` before any request. If `code` is present it calls `GET /api/v1/info` and compares `spki_sha256` with `key` and `server_id` with `id`. A mismatch shows "This is not the Mac whose code you scanned. Nothing was sent." and stops. A match posts `POST /api/v1/pair` with `{code, device_name, spki_sha256: key, browser: true}`. On success it writes `switchboard.pinned` in try/catch and loads the view.
- *Device name.* Composed from `navigator.userAgent`: browser (`Edg/`, then `Firefox/`, then `Chrome/` before `Safari/`) and device (`iPhone`, `iPad`, `Macintosh`, `Android`, else none), as `Safari on iPhone`. The host keeps `new_device`'s existing trim to 64 characters (`src/core/remote.rs:169`). A desktop-mode iPad reads as a Mac.
- *No code and no token.* `GET /api/v1/device` returns 401. The page then calls `/api/v1/info` and compares its key with `switchboard.pinned`:
  - no stored entry: "Scan the code on the Mac to pair this browser.";
  - stored entry, key differs: "This Mac's identity changed. Scan a new code on the Mac to pair again.";
  - stored entry, key the same: "This browser's access to the Mac has ended. Scan a new code to pair again.";
  - without storage the first sentence applies to every 401.

  **Owner (For the owner, wording).** The three sentences are the plan's choice. The third covers a revoked device, a closed session and a stale pairing alike, since a session cookie ends on every browser close. The page does not read the 401's message text.
- *Header.* Device name, scope and the host's name from `/api/v1/device` and `/info`. The reply box is shown only when `scope` is `control`. A `403 scope` on send shows "This device reads only now; the Mac lowered its scope."
- *Views.* Projects from `/api/v1/projects`, sessions per project from `/api/v1/projects/{id}/sessions`, and `/api/v1/waiting`. Waiting sessions first, marked with the status word and `waiting_reason`. A session view reads `/api/v1/sessions/{id}`, then `last-response` when the wire `kind` is the Claude Code kind, else `screen?lines=n`. It sends `watch {kind, id}` for `conversation` or `screen` and refetches on the matching `changed`. The `kind` strings are read from the serialized `SessionKind`, not guessed.
- *Live.* `wss://<host>/api/v1/events` opened after the view loads. `hello` sets the epoch; an epoch change refetches everything. `changed` refetches the named collection. `resync_required` and a closed socket reconnect after a delay and refetch. A reconnect that gets 401 goes to the no-token states above.

## Tests

The HTTP tests use the host's own client, as the CLI does: `http()` in `tests/remote.rs:245-298` is extended to return the response headers and to send a `Cookie`, `Origin` and `Host` when asked.

- `tests/remote.rs`:
  - `the_page_answers_without_a_token_with_its_headers_and_nothing_else`: `GET /`, `/app.js`, `/app.css` return 200 with the exact `Content-Type`, `Content-Security-Policy`, `Referrer-Policy`, `Cache-Control`, `X-Content-Type-Options`, and `Connection: close`, each header once; the body is the embedded file. `/index.html`, `/app.css/` and `/` with a non-GET method return the 404.
  - `the_page_asks_only_for_paths_the_host_serves`: parse `href`/`src` in `index.html` and the `/api/v1/` literals in `app.js`. Each one, with a paired control-scope token, must not return the catch-all refusal "not available on this port" (`src/app.rs:3108-3116`).
  - `a_static_answer_closes_its_connection`: the server closes after `GET /` (`read` returns 0), so the slot is released.
  - `pairing_succeeds_while_six_page_sockets_are_open`: open six idle sockets, then `POST /api/v1/pair` on a seventh connection succeeds.
  - `a_browser_pairing_sets_a_cookie_and_returns_no_token`: `POST /api/v1/pair` with `browser: true` inside the window returns the `Set-Cookie` with the exact attributes, no `token` in the body, and the device record with `browser: true`. A native pairing still returns `token` and no cookie.
  - `a_cookie_reads_but_refuses_state_changes_without_its_origin`: with the cookie and no bearer, `GET /api/v1/device` is 200; `POST /api/v1/sessions/{id}/send` without `Origin` is 403 `origin`; with `Origin: https://<Host>` it passes the origin check; the events upgrade with a foreign `Origin` is 403; `Origin: null` is 403.
  - `device_reads_its_scope_and_name`: `/api/v1/device` returns `{device_id, name, scope, server_name}` for a bearer and for a cookie.
  - `a_revoked_cookie_is_refused_on_its_next_request`: revoke the device through the core, the same cookie then gets 401.
  - `the_invite_url_carries_the_code_key_and_id`: `invite_url(...)` produces `https://<addr>/?code=…&key=…&id=…` with 64 hex and 8 code characters.
  - `the_allowlist_refuses_everything_else_with_no_side_effect` (`:682`): cases added for `/`, `/app.js` and `/app.css` as the static exceptions, and for `/api/v1/device` as allowed.
  - `the_client_fixtures_decode_on_both_sides` (`:1829`): `device.json` is added to `client/Fixtures` (28 files, from 27), and checked for its keys on the Rust side.
- `src/adapters/remote.rs` unit tests: `read_request` keeps `host` and `origin`, extracts only `__Host-switchboard-` cookie values and never the raw `Cookie` header; `respond` writes each header once and lets a caller replace the default; `remote_web::asset` returns the three files and nothing else; `make_identity` sets `certificate_expires` about 398 days out; `reissue` keeps the SPKI and changes the certificate bytes.
- `src/app.rs`, startup: `a_read_only_instance_does_not_reissue_the_certificate`: a read-only core leaves `identity.json` unchanged.
- `src/core/tests.rs`: `new_device` with `browser: true` stores it; `src/app.rs:3659` passes the flag from the body, and the two existing core tests (`src/core/tests.rs:11942`, `:11955`) pass `false`.
- `tests/ui.rs`: `remote_devices_dialog_opens_pairing_and_revokes_a_device` (`:7424`) extended: the QR's text label starts with `https://`; "Copy invite" (`:7474`) copies that URL; the LAN-off line appears when `lan` is off; a browser device row shows "Browser".
- `client/Tests/SwitchboardProtocolTests/FixturesTests.swift`: `PairingInvite.parse` of a URL gives `serverID`, `spkiSha256`, `code` and `addresses == [authority]`; the JSON form still parses.

**Not automated:** the page's JavaScript in a real browser. No browser test tool exists in the repo; adding one is an owner decision.

**Gate before merge.** Spike 23 (Steps, step 0) must record, from the owner's phone run: the cookie is kept across a reload under the click-through; the events socket opens; a `send` is accepted; the browser's `Origin` form matches the one the check compares (facts, Unresolved: whether the browser sends `https://<ip>:<port>`); the Referer on `/app.js` is empty; the QR scans at the phone's distance. The run also checks the issue's acceptance: a session that starts waiting moves to the top without a reload, a reply sent from the phone arrives, and a revoked device sees "This browser's access to the Mac has ended" on its next request. A hard fail on the certificate stops the work (Trust, decision 1). The commit does not merge with the cookie unverified.

**Checks.** `cargo test --locked --workspace`, `cargo clippy --locked --workspace --all-targets -- -D warnings`, `cargo fmt --all`, and `cargo clippy --locked -p dispatch --all-targets --target x86_64-unknown-linux-gnu -- -D warnings`.

**Steps, each leaving the app running.**
0. **Spike 23, `spikes/23-browser-session/README.md`.** The owner's phone run, with the questions in the gate and the exact steps to run them. The README records the result before step 3 is built. Steps 1, 2, 4, 5 and 6 do not depend on it.
1. **Fields and their literal sites.** `Identity.certificate_expires` and `PairedDevice.browser` with their defaults, and `Core::new_device`'s parameter. Fix every struct literal the compiler reports, and the two core tests for `new_device` (`src/core/tests.rs:11942`, `:11955`).
2. **Reply writer, request fields, pool, static answers.** `respond` takes headers. `RemoteReply` gains `headers`, and `RemoteRequest` gains `host`, `origin` and `browser_cookies`. Fix every literal the compiler reports. `MAX_UNAUTHENTICATED` becomes 8. `remote_web` and the static answers, with the page files as placeholders, plus the static and pool tests.
3. **Browser pairing, the cookie branch, the `Origin` check, `/api/v1/device`.** Built after step 0 is recorded. With their tests.
4. **Certificate expiry and the startup reissue,** with the read-only gate and its test.
5. **`invite_url`, the pairing window, the devices dialog, the Swift `parse`,** with their tests.
6. **The page files.**
7. **Docs and status.**

## Docs and status

- `docs/remote-client.md`:
  - §4.1, line 125: the unauthenticated bound is 8, not 4. The pairing bound is the `/pair` rate window and the window's counter, not the pool.
  - §4.3, line 201 and line 207: the card's one-line excerpt is not on the port. Line 207's sentence ("that is `last_message` from the `Stop` hook on the record … and moves with `changed {session}`") and line 201's "card excerpt" are amended: the excerpt is a later addition through the content worker, and the page shows the status word and the last response. The `session.last_response` projection on line 207 is unchanged, because the plan ships that read.
  - §4.3: rows for `GET /`, `/app.js`, `/app.css` (no auth, before pairing) and `GET /api/v1/device` (read); the allowlist sentence names the three page files.
  - §5: before pairing, `/info`, `/pair` and the page files; the QR and text form are the URL; a browser pairing gets a cookie and no token in the reply; the click-through is the trust step; the Origin check.
- `docs/design.md`, "Remote client (2026-10-12)" (`:3107`): one bullet in "Built so far" for the web control surface (#230), and the known gap "the card's excerpt is not on the wire; the page shows the status word and the last response".
- `README.md`: Layout table gets `web/index.html`, `web/app.js`, `web/app.css`, and `src/adapters/remote_web.rs`; `src/adapters/remote.rs`'s line (`:383`) gains the static answers, the pool of 8, and the cookie; the `switchboard-remote pair` line (`:277`) takes the URL.
- `CHANGELOG.md`, under `## [Unreleased]` (`:9`): one line for the user-visible change.

## Owner items

Items with no inline home. Each other owner question is an **Owner** note in the section where it is decided.

1. Device name from the page (default) or from the host's User-Agent parse, which needs the header kept (Actions and effects, Request).
2. A browser test tool as a dev dependency (default: none).
3. Yearly reissue over the same key (default: yes. If no, the startup block skips the reissue and browsers keep the year-4096 certificate, with the Safari risk the facts file leaves open).
4. Restart survival with the signed bundle (`scripts/bundle.sh`): the owner runs it, or the plan marks restart survival unverified.
5. The phone run, Spike 23 (Gate).

**Known costs.** A yearly reissue may ask the browser for the click again, if the browser keys its exception on the certificate and not the key (unresolved; facts). Each browser start is a pairing, so each adds one device record to `devices.json` and one row in the dialog (`new_device` mints a fresh id each call, `src/core/remote.rs:168`). The host cannot tell that two records are one browser, so this cut does not clean them up; the owner revokes them, and a later issue can name the rule for a cleanup. A code seen in browser history stays usable until its first pair or five minutes, whichever comes first (single use, `docs/remote-client.md` §11; window, `src/core/action.rs:972`).

## Contract additions

For the implementer to copy. Additive only; no rename or removal.

- `ports::remote::PairedDevice.browser: bool`, `#[serde(default)]`, `false` for existing records.
- `ports::remote::Identity.certificate_expires: Option<SystemTime>`, `#[serde(default)]`, `None` for existing identities.
- Wire: `GET /` `GET /app.js` `GET /app.css`, unauthenticated, served before pairing; `Content-Type` and the headers above.
- Wire: `GET /api/v1/device`, any scope: `{device_id, name, scope, server_name}`.
- Wire: `POST /api/v1/pair` body may carry `browser: true`; a browser pairing replies without `token` and sets `__Host-switchboard-<server_id>`.
- Wire: a cookie is a credential for every route the bearer reaches; a cookie-authenticated non-`GET` or `events` upgrade must carry an `https` `Origin` whose authority is the `Host`, else 403 `origin` (a new error code).
- Wire: the pairing invite is `https://<address>/?code=…&key=…&id=…`; `PairingInvite.parse` takes it and still takes JSON.

**Commit message:** `Serve a web control page on the remote port, paired from a URL`. The commit body is a copy of this Contract additions list.
