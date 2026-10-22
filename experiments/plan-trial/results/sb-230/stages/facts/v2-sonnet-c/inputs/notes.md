# Issue #230: a browser page on the remote port, investigation notes

Read: CLAUDE.md, docs/lessons.md (84 lines, nothing about the remote port;
its lesson is the action/effect core, a fake for every port, and "spike
risky technology first"), docs/remote-client.md §§0–5, design.md "Trust
boundary" (l.219) and the start of "Remote client (2026-10-12)" (l.3107),
plus the code below. Line numbers are as they are in this worktree at
7fe8949. There is no `web/` directory yet.

## The record first: what is on disk, and how it comes back

- **No new record kind and no `model.rs` change is needed.** A browser is
  one more `PairedDevice` (`src/ports/remote.rs:96-107`: `id`, `name`,
  `scope`, `token_sha256`, `created`, `last_seen`) in
  `<data dir>/devices.json`. That file is a bare `Vec<PairedDevice>`
  with no schema version (`RemoteFiles::load_devices`/`save_devices`,
  `src/adapters/remote.rs:80-86`), written atomically by
  `Effect::SaveDevices` (`src/core/remote.rs:121-125`). The name comes
  from `/pair`'s `device_name` (`src/app.rs:3650-3655`), cut to 64 chars
  in `AppCore::new_device` (`src/core/remote.rs:166-175`). So "Safari on
  iPhone" can be the page's own `device_name` with no host change. If
  the plan wants a `kind` (browser or native) on the device, it is an
  additive `#[serde(default)]` field in a file that has no version.
- **Rehydration** is unchanged: `IdentityLoaded { identity, devices,
  rotated }` (`src/core/remote.rs:22-30`) puts the devices back at
  launch. A token is checked by hash against them on every request
  (`authenticate`, `src/app.rs:3125-3153`; `device_by_token`,
  `src/core/remote.rs:145-147`). Revoke (`RevokeDevice`,
  `src/core/remote.rs:86-92`) and scope change (`:93-100`) apply on the
  next request, and revoke closes the device's socket (test
  `revoke_closes_the_devices_socket`, `tests/remote.rs:2892`).
- **The pairing window is not durable** (`PairingWindow` in core state:
  code, scope, `until`, failures; `OpenPairing`/`ClosePairing`/
  `PairingFailed`/`Paired`, `src/core/remote.rs:46-73`). The QR is built
  in the UI from it (`src/ui/remote.rs:155-173`). Changing JSON to a URL
  is a change to that `json!` call and to the "Copy invite" text (`:199-206`),
  not to any record.
- **The page itself is not a record.** It is static bytes compiled in
  (`include_str!`). On the device, what persists is the token, plus
  whatever the page keeps to say "identity changed". Decision 2 picks
  where that is. The browser also keeps its certificate exception per
  host. The host keeps nothing new per browser beyond the device record.
- **Identity** (`identity.json`, `Identity` at `src/ports/remote.rs:26-42`,
  `IDENTITY_SCHEMA_VERSION = 1` at `:17`). It holds the certificate
  (base64 DER) and `spki_sha256`. The certificate is made by
  `make_identity` (`src/adapters/remote.rs:95-127`) with
  `CertificateParams::new(Vec::<String>::new())`: **no subjectAltName at
  all**, a CN of `Switchboard <server_id>`, rcgen's default validity
  (the comment says `not_after` is the year 4096), and no EKU set
  explicitly. The native clients pin the SPKI and ignore names, so this
  never mattered. A browser checks names and validity. Changing the
  certificate (adding SANs for the LAN address or a `.local` name, or a
  bounded validity) changes nothing the pin sees if the key is reused
  (the SPKI is the same, §5). But the certificate is stored in
  `identity.json`, so reissuing it means writing that file, and an
  address that moves would need another reissue.

## What the code does today, piece by piece

### The listener (`src/adapters/remote.rs`)

- `read_request` (`:444-513`) keeps the method, the path, the query
  (undecoded), the body, `bearer` (`Authorization: Bearer`, `:487-491`)
  and `upgrade` (`Sec-WebSocket-Key` when `Upgrade: websocket`,
  `:492-495`). It throws away every other header. **It does not read
  `Cookie`, `Origin`, `User-Agent`, `Sec-WebSocket-Protocol`, `Host`,
  `If-None-Match` or `Accept`.** Either token transport in decision 2
  needs a new `RemoteRequest` field. That struct is in
  `src/adapters/remote.rs:359-372`, which is not under `ports/`, but
  `tests/remote.rs` builds requests and should be checked.
- `respond` (`:530-552`) always writes `Content-Type: application/json`
  and `Connection: keep-alive`, nothing else. It has no CSP, no
  Cache-Control, no Set-Cookie and no other content type. `RemoteReply`
  (`:388-393`) is `{status, body, authenticated}`. Static files and a
  cookie on `/pair` therefore need `RemoteReply` (or a separate
  connection-thread path) to carry headers. `respond_upgrade`
  (`:519-527`) writes no `Sec-WebSocket-Protocol`. A browser that offered
  a subprotocol fails the handshake if the server does not echo one
  (that is a question below, not checked).
- Pools and windows: `MAX_UNAUTHENTICATED = 4`, `MAX_PAIRED = 12`
  (`:567-568`). A stream past the 4 is dropped at accept (`:699-706`). A
  connection moves pools only when a reply says `authenticated: true`
  (`:834-849`). `HANDSHAKE_TIMEOUT` is 5 s (`:570`), and the first
  request line has the same deadline. `IDLE_TIMEOUT` is 30 s (`:573`).
  The rate windows exist only for `/api/v1/pair` (6/min) and
  `/api/v1/info` (60/min) (`:583-587`, `Limits::limited` `:629-640`).
  Any other path, a static one included, is unlimited and goes to the
  frame. What this means for a page load: a browser opens several
  parallel connections and keeps them alive. Static GETs leave a
  connection in the unauthenticated pool for up to 30 s idle. A
  failed TLS handshake (the certificate warning) also holds a slot until
  it ends. **The pool of 4 is what an unpaired page load competes for.**
  Once paired, each fetch with the cookie or token moves its connection
  to the paired pool. The test `stalled_strangers_fill_their_own_pool_and_never_a_paired_clients`
  (`tests/remote.rs:3226`) is the existing model for that behaviour.
- The frame drain: `port.poll(DRAIN_PER_PASS)` then `serve_remote` for
  each (`src/app.rs:1585-1597`). The connection thread answers 429/503/504
  without the frame (`src/adapters/remote.rs:787-833`). Static bytes
  could be answered on the connection thread the same way (they need no
  core state), or on the frame through `serve_remote`. §4.1 "the frame
  does core state only" leans toward the connection thread. Not decided
  here.

### The router (`src/app.rs`)

- `serve_remote` (`:3017-3121`) refuses any path without the `/api/v1/`
  prefix with 404 `not_found` (`:3023-3025`). Then it answers `info` and
  `pair` unauthenticated (`:3027-3031`), authenticates, and handles
  `events` (`:3036-3038`) and the allowlist. The static paths would be a
  new arm before the prefix match, and they are the third thing served
  before pairing (§5's rule, `docs/remote-client.md` l.390 in the
  section, "Require pairing before anything but `/info` and `/pair`").
- `authenticate` (`:3125-3153`) knows only `request.bearer`. Both its
  refusals have the code `unauthorized`, with different messages: "pair
  this device first" (no token) and "the token is not known; pair again"
  (revoked, rotated, or never valid). **A page cannot tell "revoked"
  from "never paired" by the code.** With an HttpOnly cookie, the page
  cannot even see whether it holds a token. The "revoked" state in the
  issue needs either a distinct code (additive) or a non-secret marker
  the page keeps itself (for example, "I paired with server X as device
  Y").
- `remote_subscribe` (`:2723-2758`) requires `request.upgrade` and a
  device. It does not check `Origin`. Neither does anything else.
- `remote_pair` (`:3606-3674`) compares the body's `spki_sha256` with the
  host's own, then the code, uppercased. It counts a failure on either
  (`PairingFailed`). It answers `{token, scope, server_id, device_id}` in
  the envelope. A cookie would be set here.
- `remote_info` (`:3568-3600`) carries `spki_sha256`, `server_name`,
  `capabilities`, `version`, `pairing_open`, but not the device's scope
  (it is unauthenticated). The header's "scope" comes from the `/pair`
  reply, and after that from nowhere: **no endpoint today tells a device
  its current scope**. A scope change in the dialog is invisible until a
  `control` command is refused. The Swift client has the same gap.
- `refuse` (`:3754-3772`) logs method and path, never the query. A
  `GET /?code=…` that reached the router would not log the code. Any
  new static handler must keep that true.

### The session list and the excerpt

- `SessionView` (`control/src/lib.rs:636-688`) has `kind`, `card` (the
  card's word), `waiting`, `waiting_reason`, `liveness`,
  `trust_question`, `prompt_refusal`, `last_stop_at_ms`,
  `last_activity_ms`. **It has no excerpt or last-message field.** The
  §4.3 text (l.291 of the spec section) calls the card's one-line excerpt
  "`last_message` from the `Stop` hook on the record, a field of the
  session's metadata". That field is not on the wire, and `grep` finds no
  `last_message` in `control/src/lib.rs` or `src/app.rs`. The headset
  fills its compact card by fetching `sessions/{id}/last-response` per
  session (`client/Sources/SwitchboardUI/BrowserView.swift:149-163`). So
  either the page does the same (N requests and N watches, against a
  watch set capped at 128), or the issue's "No new read model unless the
  page cannot do without one" is tested right here.
- Grouped by project: `GET projects` then `GET projects/{id}/sessions`
  each, or `GET waiting` for the waiting set. That is what the headset
  does. Worker records are hidden unless `?scope=all` (`hide_workers`,
  `:3116-3118`).
- The session view: `last-response` for a Claude session, `screen`
  otherwise. Both are content keys that need `watch` frames on the
  socket for `changed` (§4.4 "Watch frames"). The page must speak
  `watch`/`unwatch`, wait for `watching`, then fetch.
- Reply box: `POST sessions/{id}/send` with `{op, text}` under `control`
  (`remote_command`, `:3159`). The page makes the op (needs
  `crypto.randomUUID`, which only exists in a secure context; see
  questions) and follows §4.5's `/ops/{op}` rule on a timeout.

### Pairing UI and the text paths

- QR payload: `src/ui/remote.rs:160-166`, a JSON object with
  `addresses` holding at most one `ip:port` from `remote::local_address()`
  (`src/adapters/remote.rs:962-970`, the outward route's source
  address). An IPv6 result would need brackets in a URL. The address is
  whatever the route picks, not necessarily the one the phone reaches.
- Swift: `PairingInvite.parse` decodes JSON only
  (`client/Sources/SwitchboardProtocol/Info.swift:93-116`).
  `switchboard-remote pair '<invite json>'` or `pair <host:port> <code>
  <spki>` (`client/Sources/switchboard-remote/main.swift:12-13,
  107-125`). `PairingInvite` is used by `ConnectionManager.swift`,
  `BrowserView.swift` and eight test files under `client/Tests`. Taking a
  URL means `parse` tries URL first (`URLComponents`, `code`/`key`/`id`)
  and JSON second. `addresses` comes from the URL's host and port.
- The README dev-aid text (l.265-278) documents "Copy invite" as the QR
  text and `switchboard-remote pair "$(pbpaste)"`. It must change with
  the payload.
- The script line `pairing <code>` (README l.278, `src/script.rs`) opens
  a window with a known code. That is enough to drive the page by hand
  against a test data dir.

### Tests that exist and what extends

- `tests/remote.rs` speaks raw HTTP/1.1 over a pinned rustls client
  (`http`, `:245-296`). It reads `Content-Length` only and keeps no other
  header, so header assertions (CSP, Cache-Control, Set-Cookie,
  Content-Type) need it to return headers. The relevant neighbours are
  `info_is_served_to_anyone_and_every_other_read_is_refused_unpaired`
  (`:426`), `the_allowlist_refuses_everything_else_with_no_side_effect`
  (`:682`), `pair_is_rate_limited_per_listener` (`:1406`),
  `subscribe_at` (`:1960`) and `the_client_fixtures_decode_on_both_sides`
  (`:1829`, `client/Fixtures`).
- "Every path the page references is served" can be a test over the
  embedded strings. The page is plain JS with no runner in the tree, so
  its logic (query parse, `replaceState`, revision reconcile) is
  untested unless the plan adds a JS test harness, which the issue rules
  out as a dependency. The page's protocol logic is therefore checked
  only by hand.
- CI runs on Linux. `include_str!` of `web/*` is portable.

## Things the issue asserts that the tree contradicts or leaves open

- "Any path outside `/api/v1/` is refused": true (`src/app.rs:3023`), but
  on the frame, after the request was queued (so it costs inbox space),
  not at the connection thread.
- "the one-line excerpt the card shows" is not on the wire (above).
- "the device's scope in the header": no endpoint returns it after
  pairing (above).
- "'revoked' said plainly": the refusal code does not distinguish it
  (above).
- The `key` check in the page compares the URL's key with what `/info`
  says. A page cannot read the TLS certificate it is talking over, so
  this proves only that the host *claims* that key. A relay forwards
  `/info` faithfully, and the host's own `spki_sha256` check in
  `remote_pair` (`:3639-3647`) is fed by the page's body, which is the
  impostor's page under a relay. What it does catch is a genuine page on
  the wrong genuine host (a second installation at that address or
  port, or a rotated key). The issue says roughly this. The tree adds
  that §5's sentence "the host refuses a pairing whose `spki_sha256` is
  not its own, so a relayed request … fails" does not hold for a
  browser, and the spec text must say so.
- Cookies are scoped by host, not by port. Two Switchboard instances on
  one Mac (`SWITCHBOARD_REMOTE_PORT`, design §3.1) and any other HTTPS
  service on another port of that host would all receive a
  `Secure` cookie set by one of them. A cookie name per `server_id`
  avoids the clash between instances. It does not stop another service
  on that host from receiving it. `localStorage` is per origin
  (scheme, host and port), so it does not have this problem, but any
  script on the origin can read it.

## Open questions

- [owner] Decision 1: is the click-through on the local network the trust step, said in the dialog, or does the host offer its certificate (or a profile) for download to install first?
- [source] Does iOS/iPadOS Safari let the owner proceed past a self-signed certificate that has no subjectAltName and only a CN, served on a bare IP? Same question for Chrome, Firefox and macOS Safari.
- [source] After a click-through, does each browser (WebKit on iOS especially) apply the per-host exception to `wss://` WebSocket connections and to `fetch` from the page, or only to top-level navigation?
- [run] On a real iPhone, does the page load, `fetch` and `new WebSocket` work end to end against the current certificate after the warning is accepted?
- [source] Does a page whose certificate was accepted by click-through count as a secure context, so that `crypto.randomUUID`, `Secure` cookies and `SameSite` behave as on a valid HTTPS origin?
- [source] If the certificate is to be installed on the device instead: what does iOS require of a manually trusted TLS server certificate (SAN required, EKU `serverAuth`, a maximum validity such as 825 days, the root-trust toggle), and would a year-4096 `not_after` be refused?
- [owner] Should the identity's certificate be reissued over the same key with SANs (the LAN IP, a `.local` name) and a bounded validity? Who reissues it when the address changes?
- [tree] Can the certificate be reissued over the existing Keychain key without changing `spki_sha256`, through the existing `identity.json` atomic save? (`make_identity` always makes a new key today.)
- [source] Does a browser remember the click-through exception across launches, and for how long (Safari, Chrome, Firefox, iOS)? Is the exception lost when the host's IP changes?
- [owner] Decision 2: HttpOnly `Secure; SameSite=Strict` cookie, or the token as a `Sec-WebSocket-Protocol` value with the token in page storage?
- [source] Are cookies set with `SameSite=Strict` sent on a same-origin `new WebSocket()` handshake in Safari, Chrome and Firefox? Does `Secure` work for an IP-address host?
- [source] With a subprotocol token, must the server echo a `Sec-WebSocket-Protocol` in the 101 for the browser to accept the connection, and do browsers restrict the characters (base64url) in that value?
- [owner] Should the host check `Origin` on the WebSocket upgrade and on POSTs (against cross-site WebSocket hijacking and CSRF), and what exact origin does it accept, given the host can be reached by IP, `.local` name or loopback?
- [owner] Cookies are shared across ports on one host: should the cookie name carry the `server_id`, and is a token cookie that other HTTPS services on the same Mac also receive acceptable?
- [tree] Does anything in `tests/remote.rs` or `src/app.rs` construct `RemoteRequest` or `RemoteReply` literally, so that adding `cookie`, `origin`, `user_agent` and response-header fields touches them? Is `RemoteRequest` part of the "shared contract" the CLAUDE.md additivity rule covers? (It lives in `adapters/`, not `ports/`.)
- [owner] How does the page tell "revoked" from "never paired" from "cookie lost": a new refusal code from `authenticate`, a non-secret marker the page keeps, or both?
- [owner] Where does the header's scope come from after pairing: a new authenticated endpoint (for example `GET /api/v1/device` or a field on an authenticated `info`), or the `/pair` reply only, accepting that it goes stale?
- [owner] The card excerpt: add an excerpt field to `SessionView` (additive in `control/`, a fixture change on both sides), or have the page fetch `last-response` per listed session as the headset does?
- [tree] Does the record carry a `last_message` from the `Stop` hook that could become that field, or is the card's excerpt computed from the transcript cache (`refresh_captions`, `src/app.rs:1093`)?
- [run] With the watch set capped at 128 keys, does a page watching `conversation` for every listed Claude session evict the headset's watches on a host with many sessions?
- [owner] Decision 3: one QR holding the URL, with the JSON dropped? Is anything left (the visionOS paste field, `BrowserView`, the CLI, fixtures) that needs the JSON form?
- [tree] Which Swift call sites and tests under `client/` construct or parse `PairingInvite`, and do any of them depend on `addresses` being a list?
- [owner] In the URL, the host part comes from `local_address()`: the IP, a `.local` hostname, or both (two QRs)? What should happen when the route-chosen address is not the one the phone can reach?
- [source] When a phone's camera opens the URL and the certificate interstitial shows first, is the URL with the code recorded in browser history, the tab list or iCloud tab sync before the page's `replaceState` can run?
- [source] Does `history.replaceState` remove the query from Safari's history entry and from "recently closed" on iOS, or only from the address bar?
- [owner] Decision 4: the exact static paths (`/`, `/app.js`, `/app.css`, a favicon?) and their Content-Type and CSP. Should static bytes be answered on the connection thread or the frame, and with which cache policy (no-store, or an ETag from `version`)?
- [owner] Should static paths get a rate window of their own, and do they count against the unauthenticated pool of 4 as today?
- [run] Does a normal page load (parallel connections, keep-alive, `/favicon.ico`, the failed handshake behind the warning) exhaust the pool of 4 and get connections dropped at accept on iOS Safari?
- [source] How many parallel HTTP/1.1 connections per host does iOS Safari open, and how long does it keep an idle one?
- [owner] Does the static-path arm amend §5's "before pairing" rule in the spec and the design.md remote status section, and in what words?
- [owner] What does the page show when opened with neither a code nor a token: "open the pairing window and scan", a code-entry form, or both?
- [owner] Decision 6: should the device record gain an additive `kind` (browser or native) or a user-agent field, or is the name enough?
- [owner] Should the page be HTML, CSS and JS under `web/` served by `include_str!` with no JS test harness, which leaves the page's reconcile logic untested by CI?
- [run] Is the end-to-end pairing flow over the host's own HTTP client (as `tests/remote.rs` does) enough for acceptance, or does the owner need a manual phone run recorded?
- [owner] On the page, a reply to a session sitting at the folder-trust question, `prompt_refusal`, and resume: in scope for the first cut, or hidden?
- [source] Does the Bonjour `.local` name resolve from iOS Safari, and would it be a better URL host than the IP?

What I did not check: no browser behaviour, iOS rule or crate API was looked up, and nothing was run. I did not read `src/script.rs`, the Swift `ConnectionManager`, `client/Fixtures`, design.md past the first part of the remote status section, or spike 22's README. The `last_message` grep covered only `control/src/lib.rs`, `src/app.rs` and the Swift protocol files.
