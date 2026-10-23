# Plan: issue #230, a web control surface on the remote port

The host serves a small page from the remote port it already has; a
phone's camera opens it from the pairing window's QR, pairs, and lists
the sessions with the waiting ones first. Everything is read from the
worktree at HEAD `7fe8949` (branch `dispatch/230-...`); "Settled",
"Unresolved" and "For the owner" below name sections of the facts file.

## Where this departs from the issue

- **"The browser accepts the certificate warning."** The certificate has
  no subject alternative name and a common name only
  (`CertificateParams::new(Vec::new())`, `src/adapters/remote.rs:101-107`),
  and Chrome has ignored the common name since 58 (Settled, The world).
  Whether a browser offers a click-through at all for that case is
  Unresolved. The plan issues the served certificate with the host's
  address as an IP SAN over the same key, so the SPKI hash and every pin
  are unchanged (Settled, The tree: `src/adapters/remote.rs:113`), and
  builds the rest to work if the click-through is offered.
- **"Open `/api/v1/events` directly."** WebKit does not apply a
  clicked-through exception to `wss` (Settled, The world: WebKit bug
  41419, and the iOS 26 forum report). The page's live path by default
  is a poll of `/api/v1/waiting` and the open session; the socket is an
  upgrade that fails quietly. "A session that starts waiting moves to
  the top without a reload" holds by the poll in every browser.
- **"The one-line excerpt the card shows."** `SessionView` has no
  excerpt field (Settled, The tree: `control/src/lib.rs:637-690`); the
  card's excerpt is `last_message`, not on the wire. The page shows
  `card`, `liveness` and `waiting_reason`, and the last response in the
  session view. No new field is added to the wire crate.
- **"The device's scope in the header."** Nothing returns the scope after
  pairing: it is in the `/pair` reply only (`src/app.rs:3662`), and with
  an `HttpOnly` cookie the page holds no copy to remember. The page
  cannot do without it (it hides the reply box on it, and the host
  raises or lowers a scope later), so one small read is added,
  `GET /api/v1/me`.
- **"Sessions grouped by project."** There is no all-sessions listing:
  `sessions` is per project and `waiting` is a membership list
  (`src/app.rs:3040-3050`; `docs/remote-client.md:169`). The page reads
  `projects`, then each project's `sessions`, and orders by the
  session's `waiting`. No new read model.
- **"One QR carrying addresses."** The JSON's `addresses` is already a
  one-element list (`src/ui/remote.rs:156-166`), and with no network
  `local_address()` is `None` (`src/adapters/remote.rs:958-970`), so a
  URL has no host to carry. The window shows the code and key as text
  and says there is no address to scan.
- **"Compare the key in the URL with the one `/info` presents."** Kept,
  but it proves less than the issue's wording: `/info`'s key is a JSON
  value from whoever answered, not the key of the TLS certificate the
  browser accepted. It catches a wrong Switchboard at the address, not
  an impostor. The dialog says so (decision 1).

## Decisions the issue asks the plan to settle

1. **Trust in the certificate: the click-through is the trust step.**
   Not a certificate download. A download means installing a profile on
   iOS and trusting a self-signed leaf as a root, a second path served
   before pairing, and a mechanism none of the facts verify; the
   click-through needs nothing new on the host beyond the SAN. What it
   buys: the transport is encrypted and the owner sees a warning
   naming the address. What it does not: a browser cannot pin, so an
   active interceptor on the local network whom the owner clicks
   through gets the code and the token. The native clients pin and are
   not affected. The paired-devices dialog says this next to the QR
   ("A browser cannot check the host's key. Pair a browser only on a
   network you trust; the invite is spent once used"), and the pairing
   scope still defaults to read. §5's rule stands and widens by one
   case: no non-loopback bind serves anything before pairing but
   `/info`, `/pair` and the three page files. The owner question for
   whether a click-through is offered (For the owner 1) does not change
   the plan: the SAN is added either way. If it is offered only after
   the owner trusts the certificate on the device, the download is a
   follow-up (end of plan).
2. **Token transport: an `HttpOnly; Secure; SameSite=Strict` cookie,
   and only that.** Where the token lives is a security decision: a
   cookie script cannot read, over `localStorage` any script on the
   origin can. The cookie is `sb_token=<token>; Max-Age=31536000;
   Path=/api/v1/; Secure; HttpOnly; SameSite=Strict`. `Strict` costs
   nothing here: the page itself needs no cookie to load, and `fetch`
   and the socket are same-origin (Settled, The world: SameSite).
   `Path=/api/v1/` keeps it off the page files. Cookies are not scoped by
   port, so another listener on the same address would receive it; the
   dialog's sentence covers it. A cookie cannot be cleared by the page,
   so there is no "sign out" in this cut: revoke in the dialog.
   Because the cookie now authenticates, the host adds an `Origin`
   check (below); a subprotocol or a first-message token is not built.
   **Whether a clicked-through origin stores a `Secure` cookie and
   sends it on `fetch` and on the upgrade is Unresolved, and it is the
   first thing the implementer tests** (spike 23, step 1 below). If a
   real Safari or Chrome does not store it, stop and report: the
   fallback (a header from `sessionStorage`, a first-message socket
   auth) is a follow-up for the owner, not a second transport built
   now. The socket needing the cookie is not on the critical path: the
   poll uses `fetch` only.
3. **One QR.** The URL `https://<ip>:<port>/?code=<code>&key=<spki>&id=<server_id>`,
   one QR and one text. Nothing needs the JSON form's list of
   addresses, so the JSON is no longer produced; the native text path
   (`PairingInvite.parse`, `client/Sources/SwitchboardProtocol/Info.swift:94-117`,
   which the CLI and the headset's paste field both call) accepts the
   URL as well and still accepts the JSON, so an invite already copied
   and the existing test (`testInviteParsesTheQRText`,
   `client/Tests/SwitchboardProtocolTests/FixturesTests.swift:157-165`)
   stand. The code, key and id are Crockford, hex and a UUID: nothing
   to percent-encode.
4. **Static paths.** Exactly `GET /` (`text/html; charset=utf-8`),
   `GET /app.css` (`text/css; charset=utf-8`), `GET /app.js`
   (`text/javascript; charset=utf-8`). Every page response carries
   `Cache-Control: no-store` (right whether or not the browser caches
   a certificate-error origin; Unresolved, so never rely on a
   conditional request), `X-Content-Type-Options: nosniff`,
   `Referrer-Policy: no-referrer` (the code is in the first URL, and
   `replaceState` cannot beat the script's own request; Settled,
   Referrer) and
   `Content-Security-Policy: default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'`.
   `connect-src 'self'` covers `wss` in Safari 16 and later and current
   Chrome (Settled, CSP); the plan targets those (For the owner 4). The
   page links `rel="icon" href="data:,"` so no `/favicon.ico` is
   asked. Any other method on those paths is 405; any other path stays
   the 404 `not_found` JSON it is today (`src/app.rs:3023`). Without a
   code and without a pairing the page says "This device is not paired.
   On the Mac: Switchboard, Remote devices, Allow pairing, then scan
   the QR code." The page is answered on the connection thread, not
   the frame, so it costs the frame nothing and needs no drain slot.
   Pool and rate: see the record section.
5. **Where it lives and how it is tested.** `web/index.html`,
   `web/app.css`, `web/app.js`, embedded with `include_str!` (the tree
   has none yet; assets are `include_bytes!`, `src/ui/theme.rs:203-211`).
   Tests in the last section.
6. **The device record.** A browser is a `PairedDevice` with a name the
   page derives and sends as `device_name`; no field is added.

## The record

No record changes and no schema bump. The page is code in the binary.
`SCHEMA_VERSION` (`src/core/model.rs:12`, 16 today),
`VIEWS_SCHEMA_VERSION` (`:612`, 7) and `IDENTITY_SCHEMA_VERSION`
(`src/ports/remote.rs:17`, 1) stay; had one been needed it would be the
value in the file at implementation time plus one.

- **Device.** `PairedDevice` (`src/ports/remote.rs:96`) is unchanged:
  the cookie's token hashes to the same `token_sha256`, so revocation
  (`RevokeDevice`, `src/app.rs:486`, which drops the device's queues and
  closes its socket) and key rotation (deletes `devices.json`) work
  unchanged. The name comes from the page: "Safari on iPhone",
  "Safari on iPad" (a Macintosh user agent with `maxTouchPoints > 0`;
  a heuristic, Settled, The world), "Chrome on Android", else
  "Browser". `userAgentData` is not used (Chromium-only).
- **Identity.** `identity.json` is unchanged and its `certificate` field
  stays the original SAN-less one. The certificate the listener serves
  is derived at bind from the Keychain key, the server id and the
  addresses (below), in memory only, so nothing about reissue touches
  disk or the pin.
- **Browser side, not rehydrated by the host.** The page remembers
  `{server_id, spki_sha256, server_name}` in `localStorage` under a try
  block (a per-viewer convenience; it is not a credential). It is what
  lets a 401 read "revoked" rather than "never paired", and a changed
  `/info` key read "identity changed, pair again". Without it the page
  still works and says "not paired" for both.
- **Connection pools** (`src/adapters/remote.rs:567-568`): today 4
  unauthenticated and 12 paired, every accept counted in the 4
  (`:700-730`). Unresolved: how many connections a browser opens and
  whether it retries a close. The plan builds to the safe branch: a
  third pool, `MAX_PAGE = 6`, for connections whose first request is a
  page file, so a page load never competes with `/pair` and `/info` for
  the four, and an unauthenticated flood of `GET /` fills only its own
  six and never the paired twelve. The total becomes 22. A connection
  in the page pool idles out at 10 s, not 30. A page request when the
  six are full is answered 503 `busy` (browsers and people retry). A
  page connection that later makes an authenticated request moves to
  the paired pool exactly as an unauthenticated one does. Page files
  have a rate window of their own, 120 a minute per listener, 429 past
  it (`Limits::limited` returns false for any unlisted path today,
  `:583-640`). `/info` stays 60 and `/pair` 6 a minute. §4.1 is amended
  to say all of this.

## The signals

Nothing new on the wire and no new event. The page speaks §4.4:

- **Live path, default: polling.** Every 3 s the page reads
  `/api/v1/waiting`; the reply envelope carries `revision`, and when it
  differs from the held one the page refetches `projects`, each
  project's `sessions`, and the open session's content, applying a
  reply only if its revision is at or past the held one (§4.2). This is
  the whole feature in a browser whose socket fails.
- **Live path, enhancement: the socket.** The page opens
  `wss://<host>/api/v1/events` once the first load has succeeded.
  On `hello` the poll slows to every 30 s as a safety net; on a
  `changed`, `removed` or `resync_required` frame the page refetches
  (debounced 250 ms); on a close the poll returns to 3 s and the socket
  retries with backoff. Following §4.4 the page sends
  `{"type":"watch","resource":{"kind":"conversation"|"screen","id":<session id>}}`
  for the open session (frame shape from `src/app.rs:1704-1740`) and
  `unwatch` when it closes. If the browser refuses the socket
  (Safari today), the page never says so; it keeps polling.
- **Content.** A Claude Code session: `last-response`. On `400
  no_conversation` (Settled, The tree: `src/app.rs:1869-1886`, which is
  also what a non-Claude session and a Claude session with no handle
  answer), `screen?lines=60`. Text is placed with `textContent`, never
  as markup.
- **Errors in the page's words.** A 401 from `/me` or any read: "revoked
  or never paired" per the remembered record (above). `/info` key differs
  from the remembered one: "This host's identity changed. Pair again."
  and nothing is sent, the cookie included. The key in the URL differs
  from `/info`'s: "This is not the host this invite was made for." and
  `/pair` is not posted. 403 `pairing_closed`, `wrong_code`,
  `wrong_key`, 429 and 503 each have a sentence.

## The actions and effects

No `AppAction` and no `Effect` variant. The page is not state in the
core, the pairing window is the existing `OpenPairing`/`ClosePairing`
(`src/core/remote.rs:46`), and nothing is saved that was not saved.
One pure function is added to the core, because the invite's text
must not be built under `ui/`:

- `invite_url(address: &str, key: &str, code: &str, server_id: &Uuid) -> String`
  in `src/core/remote.rs`, `https://{address}/?code={code}&key={key}&id={server_id}`.
  `ui/remote.rs` calls it (replacing the JSON built at `:156-166`).

The rest is in the adapter and the frame's request path:

- **Certificate.** `tls_config_for(identity, key_pem, addresses: &[IpAddr])`
  in `src/adapters/remote.rs`; `tls_config` keeps its signature and
  delegates with no addresses, so existing callers and tests compile.
  It makes the certificate with the same common name, the same key, and
  `SanType::IpAddress` for `127.0.0.1` and each address given, and its
  SPKI hash must equal `identity.spki_sha256`. `bind_remote`
  (`src/app.rs:2926-2985`) passes `local_address()` when
  `listen.lan` and remembers it as `remote_cert_ip`. It is reissued, by
  a rebind, when the pairing window opens and `local_address()` differs
  from `remote_cert_ip`: in `SwitchboardApp::dispatch`
  (`src/app.rs:452`), before the action runs, an `OpenPairing` calls
  `refresh_remote_certificate()`. A rebind drops paired devices'
  sockets once, and they reconnect. The pairing UI reads the address
  from the app (`DrawCtx` gains `remote_lan_ip` beside `remote_port`,
  `src/ui/mod.rs:401,420`), so the QR carries exactly the address the
  certificate was issued for and the draw no longer opens a UDP socket
  per frame.
- **Request.** `RemoteRequest` (`:360-385`) gains `cookie` (the value of
  `sb_token` only, taken out of the headers as the bearer is, so it is
  never logged), `origin` and `host`; `read_request` (`:444-517`) reads
  `cookie`, `origin`, `host`. The 8 KiB head (`MAX_HEAD`) is checked
  with a real browser's request in the tests.
- **Reply.** `RemoteReply` (`:389`) gains `set_token: Option<String>`.
  It is a struct literal at 30 sites (`grep -n "RemoteReply {"` over
  `src/` and `tests/`); each gets `set_token: None`. `respond`
  (`:530-552`) becomes `respond_with(w, status, content_type, headers, body)`
  with `respond` delegating, and every response now carries
  `Cache-Control: no-store` and `X-Content-Type-Options: nosniff`;
  `set_token` becomes the `Set-Cookie` above. The reason table gains
  nothing (no 304 is ever sent).
- **Page.** In `connection` (`:770-875`), before the rate check, a
  `GET` of one of the three paths is answered by `respond_page` from a
  const table of `(path, content_type, include_str!(...))`. It is
  outside the frame and outside `INBOX`.
- **Authenticate.** `authenticate` (`src/app.rs:3128-3160`) takes
  `request.bearer` else `request.cookie`. When the token came from the
  cookie and the request is an upgrade or not a `GET`, `Origin` must be
  `https://` plus the `Host` header, else 403 `origin` with
  `authenticated: false`. Native clients send a bearer and are not
  checked. This is defence in depth beside `SameSite=Strict`, and it is
  what stops a cross-site WebSocket hijack on any browser that does not
  apply `SameSite` to the handshake.
- **`/pair`.** `remote_pair` (`:3606-3665`) reads an optional boolean
  `cookie`. When true the reply sets `set_token` and its body omits
  `token` (`scope`, `server_id`, `device_id` remain), so the page never
  sees the token. Absent or false is today's reply, byte for byte; the
  Swift client and `pair` in `tests/remote.rs:406` are untouched. The
  `device_name` is cut at 64 characters here (the page sends fewer),
  because a browser, unlike the native clients, is not our code.
- **`GET /api/v1/me`.** Read scope. `{device_id, name, scope}` for the
  authenticated device. One arm in `serve_remote` (`:3017-3127`) and a
  row in `docs/remote-client.md` §4.3; `info`'s capabilities gain
  `"web"` (additive) so the native clients can tell.

## The UI

`src/ui/remote.rs`, the pairing window (`pairing`, `:141-227`):

- The QR encodes `invite_url(...)`; `Copy invite` copies the URL. The
  text under the QR gains the URL's address line and the sentence from
  decision 1, and "Scan, or type the key and the code" stays for the
  headset.
- With no address (`remote_lan_ip` is `None`, or the port is
  loopback-only) there is no QR: the dialog shows the code and key as
  text and "No network address to put in a QR code."
- The invite is spent once used, and the window already closes on the
  first success; the dialog says so in one sentence.

The page (`web/`), no framework, no build, no dependency:

- `index.html`: `<meta name="viewport">`, `<meta name="referrer"
  content="no-referrer">`, the icon link, a header (host name and the
  device's scope), `#waiting`, `#projects`, a session pane with
  `<pre>` content and the reply `<form>`, and a status line. No inline
  script or style.
- `app.js`: the first statement reads `location.search` and calls
  `history.replaceState(null, "", location.pathname)`; then the flow
  `/info` → compare key and id → `POST /pair {code, spki_sha256,
  device_name, cookie: true}` → `GET /me` → lists. Reply box shown only
  when `/me` says `control`; it posts `/sessions/{id}/send` with
  `{op, text}`, `op` made from `crypto.getRandomValues` (not
  `randomUUID`, so a context the browser rates as not secure still
  works). A failed or timed-out send is followed by `GET /ops/{op}`,
  never a blind resend (§4.5): `unknown` allows the repeat, `interrupted`
  does not. Sessions are ordered waiting first, then by
  `last_activity_ms`, each marked and showing `kind`, `liveness`,
  `card` and `waiting_reason`; sessions are grouped under their
  project's name.
- `app.css`: system fonts, `prefers-color-scheme` light and dark, 16 px
  gutters, usable at phone width with no horizontal scroll, tap targets
  at least 44 px.

## The tests

First, before any other code, **spike 23** (`spikes/23-web-cookie/README.md`,
and add it to the `spikes/` line of the README Layout table): a
`python3` `http.server` over TLS with a certificate made by `openssl`
carrying an IP SAN, a path that sets the cookie above and a path that
echoes the `Cookie` header, run on the LAN and opened from a real
iPhone or iPad Safari and Chrome. The README records the commands and
what was seen: is a click-through offered with the SAN (and without),
is the cookie stored and sent by `fetch` after the click-through, does
it survive a reload and a second tab. Per CLAUDE.md a risky mechanism
has a spike before the build; a negative on the cookie stops the build
as decision 2 says. The `wss` and exception results are recorded as
they are seen and change nothing in the plan.

Core (`src/core/tests.rs`): `invite_url` for a known input equals the
exact string the Swift test uses.

Adapter (`src/adapters/remote.rs` tests): `read_request` extracts the
cookie value only for `sb_token`, `origin` and `host`, and rejects a
head over `MAX_HEAD` that includes a long cookie; `respond_with` writes
the headers; `tls_config_for` serves a certificate whose DER holds the
SAN extension and the given octets and whose SPKI equals the identity's;
a page request moves a connection into the page pool, the seventh is
503, the unauthenticated pool still takes `/info` meanwhile, and the
121st page read in a minute is 429.

Integration (`tests/remote.rs`; a helper returning response headers is
added beside `http` at `:245`, which drops them today):

- the three paths answer 200 with the exact `Content-Type`, CSP,
  `Cache-Control`, `Referrer-Policy`, `nosniff`; `POST /` is 405;
  `GET /x` is still 404 `not_found`; `GET /` is answered while the
  test never polls the frame (extend
  `the_allowlist_refuses_everything_else_with_no_side_effect`, `:682`);
- the embedded page references only served paths: every `href`/`src` in
  `web/index.html` is one of the three paths or `data:`, and every
  `/api/v1/...` literal in `web/app.js` is an allowlisted route (the
  test requests it with a token and asserts the reply is not
  `not_found`);
- pairing end to end as the CLI does, over the host's own client, with
  `cookie: true`: the reply has no `token`, `Set-Cookie` has each
  attribute exactly, `GET /me` with only `Cookie` answers the scope,
  the same with a bearer still works, a cookie plus a foreign `Origin`
  on a `POST` and on an upgrade is 403 `origin`, a revoked device's
  cookie is 401, and `pair` without `cookie` is unchanged;
- the existing pairing, rate and revocation tests pass untouched
  (`:426`, `:582`, `:658`, `:1406`).

UI (`tests/ui.rs`): with `remote_lan_ip` seeded the pairing window
shows the URL text and the browser sentence; with it `None` it shows
"No network address". `Copy invite` dispatches nothing to the core.

Swift (`client/Tests/SwitchboardProtocolTests/FixturesTests.swift`):
`testInviteParsesTheURL` (the same literal as the core test, fields
checked), a URL with a non-`https` scheme or a missing `code` throws,
the JSON test unchanged. `swift test` in `client/` if the toolchain is
present; the Rust four commands are the gate otherwise. No fixture
file changes (no view changes).

The acceptance steps need a device and a tap and are not a test: run
`remote-listen lan <port>` and `pairing <code>` from `SWITCHBOARD_SCRIPT`
(no new script line: both already reach the state, `src/script.rs:729-737`),
scan the QR, click through, see the sessions, start a waiting session
and watch it move up, send a reply, revoke in the dialog. The result
goes in design.md's status section as seen, including whether the socket
opened.

## Docs and order

One commit; the app runs at every step. Order: spike 23 and its result;
`RemoteRequest`/`RemoteReply`/`respond_with`; the page pool and rate
window; the certificate; `authenticate`, `/pair`, `/me`; `web/`;
`invite_url` and the dialog; Swift `parse`; tests; docs. Docs:
`docs/remote-client.md` §4.1 (22 connections, the page pool, its rate
and idle), §4.3 (`/me`, the page files), §5 (the QR is a URL; the
cookie for browsers beside the bearer; browser trust paragraph;
"anything but `/info`, `/pair` and the page files"; the key in the URL
and its limit) and §11 (the new failure cases); `docs/design.md`, the
remote status section ("Remote client (2026-10-12)", line 3107): a
bullet for the page and its known gaps (Safari's `wss`, no sign-out, no
excerpt, the unresolved items as seen); README Layout (`web/`, and the
changed rows for `src/adapters/remote.rs`, `src/ui/remote.rs`,
`tests/remote.rs`, and the spikes line) and the dev-aid note at `:277`
(the clipboard text is now a URL, `switchboard-remote pair "$(pbpaste)"`
still pairs it); `CHANGELOG.md` Unreleased.

## Follow-ups for the owner

- If no browser offers a click-through even with the SAN: serve the
  certificate for download so the owner can trust it on the device first
  (For the owner 1). Needs a decision on the trust path, not built here.
- If the `Secure` cookie is not stored from a clicked-through origin
  (spike 23): the fallback token store (`sessionStorage` header plus a
  first-message socket auth) is the owner's call on the weaker store.
- A sign-out that clears the cookie (a `POST` the host answers with an
  expired `Set-Cookie`).
- Dispatch tickets and decisions, the full conversation, drafts and
  dictation on the page: later issues, as the issue says.

## Commit message

Serve a web control surface from the remote port: a browser pairs from the QR's URL, holds an HttpOnly cookie, and lists, reads and answers sessions

## Contract additions

- `RemoteRequest`: fields `cookie: Option<String>`, `origin: Option<String>`, `host: Option<String>`.
- `RemoteReply`: field `set_token: Option<String>` (30 literal sites get `None`).
- `adapters::remote`: `tls_config_for(identity, key_pem, addresses)`, `respond_with`, the page table and `MAX_PAGE` pool; `tls_config` unchanged.
- `core::remote::invite_url`.
- Wire (HTTP only, no `model.rs` or `switchboard-control` change, no schema bump): `GET /api/v1/me`; optional `cookie` boolean in the `POST /api/v1/pair` body, with `token` omitted from the reply and `Set-Cookie` set when true; `"web"` in `info`'s capabilities; a 403 `origin` refusal for cookie-authenticated upgrades and non-`GET` requests; `Cache-Control: no-store` and `X-Content-Type-Options: nosniff` on every response.
- `DrawCtx`: field `remote_lan_ip: Option<IpAddr>`.
- `PairingInvite.parse` (Swift): accepts the URL form beside the JSON.
- New files: `web/index.html`, `web/app.css`, `web/app.js`, `spikes/23-web-cookie/README.md`.
