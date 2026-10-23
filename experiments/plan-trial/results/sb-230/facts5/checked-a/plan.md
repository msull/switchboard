# Plan: issue #230, a web control surface on the remote port

One commit. The app runs at every step. Every line number below was
checked in this worktree on 2026-10-22; the facts file is cited as
"facts" with its own numbering (Settled 1-6, Unresolved U1-U4, owner
O1-O3).

## Where this departs from the issue

- **"Accept the certificate warning, see the sessions" does not work
  with today's certificate.** `make_identity` builds
  `CertificateParams::new(Vec::<String>::new())` with a CommonName only
  (`src/adapters/remote.rs:102-106`): no subjectAltName, no extended key
  usage, validity to year 4096. A browser matches an IP-literal URL
  against the SAN, so the QR's `https://<ip>:<port>/` is a name
  mismatch before any click-through (facts, Settled 5). The plan
  re-issues the certificate over the same key with SANs (step 2), so
  the pinned SPKI and every paired native device are untouched. The
  new certificate is persisted, not re-made per launch: a browser's
  click-through decision is tied to the certificate it was given for
  (Chrome's `SSLHostStateDelegate::AllowCert` records a certificate per
  host; Firefox stores the accepted fingerprint; no Safari source found),
  and `ring`'s ECDSA signing is randomised, so a certificate re-made at
  each launch would bring the warning back each launch.
- **The WebSocket cannot be the page's only live path.** The issue
  treats the events socket as the live channel. Safari on iOS, iPadOS
  and Apple-silicon Macs does not complete `wss://` to a self-signed
  host even after the click-through (facts, Settled 1). The page's base
  is a poll of the same REST reads; the socket, when it opens, only
  makes the refetch immediate.
- **"Any path outside `/api/v1/` is refused" gets three exceptions, and
  they are answered on the connection thread**, not in `serve_remote`.
  `src/app.rs:3023` (`strip_prefix("/api/v1/")` else 404) is
  unchanged. The pages are constants; sending them through the frame
  would spend the 64-deep inbox and a frame pass per file for no state.
- **The card's one-line excerpt is not on the wire.** The issue lists it
  on each row. `docs/remote-client.md:207` says it is `last_message`, "a
  field of the session's metadata", but `SessionView`
  (`control/src/lib.rs:637-688`) has no such field and `SessionRecord`
  has none (`last_message` exists only as the Stop hook event's payload,
  `src/ports/events.rs:24`). The card builds its excerpt in the UI from
  the parsed conversation (`agent_excerpt`, `src/ui/cards.rs:558`). The
  plan's rows show name, kind, the `card` word and `waiting_reason`; the
  excerpt (first line of the last response) appears in the session view
  only. Putting an `excerpt` on `SessionView` is a follow-up. The plan
  also corrects the spec sentence at `:207`.
- **No endpoint tells a device its scope.** The issue wants the scope in
  the header and the reply box hidden without `control`. `/pair`'s
  reply carries it once, but with the token in an HttpOnly cookie the
  script never sees that reply's token and, after a reload, nothing at
  all. The page cannot do without one read, so the plan adds
  `GET /api/v1/device` (step 5). It is also the page's "am I still
  paired" probe.
- **The device name comes from the page, not the host.** The host
  drops `user-agent` today. `/pair` already takes `device_name`
  (`src/app.rs:3641`); the page sends "Safari on iPhone" from
  `navigator.userAgent`. No header capture, no record change.
- **The cookie cannot rest on `SameSite=Strict` alone.** Cookies are
  scoped by host, not port, and "same site" ignores the port. Another
  service on the same address, or a second Switchboard on another port,
  is same-site, and nothing in the tree consults `Origin` (`read_request`
  drops it). The plan adds an `Origin` check for cookie-authenticated
  writes and upgrades, and a per-installation cookie name.
- **"Sessions grouped by project" is not N+1 fetches on load.** Rows come
  from `waiting` first; a project's group fetches
  `projects/{id}/sessions` when it is expanded (groups with a waiting
  session start expanded). This keeps a first load within the paired
  pool and the 8-per-pass drain.

## The decisions the issue asks for

**1. Browser trust: the click-through is the trust step.** The host does
not offer its certificate for download. Reasons: iOS makes the owner
install a profile and then enable full trust in Settings, a flow longer
than the one it replaces; it needs a new pre-pairing endpoint that §5's
rule ("nothing before pairing but `/info` and `/pair`") would have to
admit; and a trusted certificate does not fix Safari's `wss://` anyway
(facts, Settled 1, "even when marked trusted"). What the click-through
buys: an encrypted channel to whoever answered at that address. What it
does not buy: the `key` in the URL lets a genuine page refuse to send
the code to a host whose `/info` key differs, but a page served by an
impostor is not a genuine page, and an impostor that relays the page's
own `/info` and `/pair` calls to the real host passes the host's
`spki_sha256` check, because the browser cannot pin. The trust root is
therefore the QR on the owner's own screen plus the five-minute,
single-use, three-try window, and the browser path is weaker than the
native pin. The pairing window says so in a line (UI below) and keeps
`read` the default scope. The decision this plan does not take: whether
the owner wants browser pairing to grant `control` at all; it is the
existing radio.

§5's rule becomes: a non-loopback bind serves `/info`, `/pair` and the
three page files before pairing, nothing else. The page holds no
secret and no host state.

**2. The token reaches the socket by the cookie, and only by the cookie.**
`/pair` with a body field `delivery: "cookie"` omits `token` from its
reply and sets `__Host-sb-<first 8 hex of server_id>=<token>; Path=/;
Secure; HttpOnly; SameSite=Strict; Max-Age=34560000`. `fetch` and the
WebSocket upgrade both carry it, the script never reads it, and nothing
is in `localStorage` but the public pin. The cookie name is per
installation (the `__Host-` prefix forces `Secure`, `Path=/`, no
`Domain`), so two Switchboards on one address do not overwrite each
other. The native clients keep the bearer header and the `token` in the
reply; a request with a bearer never consults the cookie. No
`Sec-WebSocket-Protocol` token, no first-frame token and no
`sessionStorage` copy: a script-readable store is the weaker place for a
credential and the plan builds one transport.

Whether Safari carries that cookie is Unresolved U1 (facts), so it is
the first thing the implementer tests (step 1), and it splits cleanly:
- *`fetch` carries it* (the ordinary case, and what the page needs for
  its base behaviour, the poll): build on.
- *The WebSocket upgrade does not carry it*: the socket answers 401, the
  page stays on the poll. Nothing breaks, because the poll is already
  the base. This is the answer on iOS whichever way U1 falls.
- *`fetch` does not carry it on Safari*: stop, record it in the spike
  README, and report to the owner. The fallback (a token the script
  holds, sent as `Authorization`) is a follow-up for the owner (O1), not
  a second transport built now.

**3. One QR.** The QR and "Copy invite" carry the URL
`https://<address>:<port>/?code=<code>&key=<spki_sha256>&id=<server_id>`.
The JSON form is no longer produced. The Swift `PairingInvite.parse`
accepts both (a pasted old invite still works) and the URL form fills
`addresses` from the host and port. No camera client needs the JSON.
An empty address (no network) gives `127.0.0.1`, which the certificate
covers, so the same Mac can pair its own browser.

**4. Static paths.**
- Exactly `GET` on `/`, `/app.js`, `/app.css`; any query is
  ignored (so `/?code=` serves the same page). Any other method on those
  paths, `HEAD` and `OPTIONS` included: 405 with `Allow: GET`. Every other path, including
  `/index.html`, `//` and `/favicon.ico`, falls through to the frame and
  its 404 as today.
- Headers on all three: `Content-Type: text/html; charset=utf-8`,
  `text/javascript; charset=utf-8`, `text/css; charset=utf-8`;
  `Cache-Control: no-store` (the page is 20 KB, and an old script
  against a newer API is the worse failure); `X-Content-Type-Options:
  nosniff`; `Referrer-Policy: no-referrer`; and on the HTML,
  `Content-Security-Policy: default-src 'none'; script-src 'self';
  style-src 'self'; connect-src 'self' wss:; img-src data:; base-uri
  'none'; form-action 'none'; frame-ancestors 'none'`. `connect-src
  'self' wss:` rather than `'self'` because WebKit does not resolve
  `'self'` to `wss:` (facts, Settled 3). That sentence in the facts says
  `wss:` adds no external host; it permits any secure socket, but only
  from the one fixed script, which has no injection point (no inline
  script, no `innerHTML`). The page's `<link rel=icon href="data:,">`
  stops the favicon request.
- Pool and rate: the three files are answered on the connection thread,
  a millisecond each, on the same connection a browser then uses for its
  first authenticated `fetch`, which promotes it to the paired pool
  (`src/adapters/remote.rs:843-858`). They count against the 4-slot
  unauthenticated pool at accept (`:701`) like `/info`, which already
  bounds an unpaired client; no rate window is added, and a page load
  never reaches the frame.
- Opened with no code and no token the page shows the host's name (from
  `/info`) and "Open Remote devices on the Mac, allow pairing, and scan
  its QR code."

**5. Where the page lives.** `web/index.html`, `web/app.js`,
`web/app.css`, embedded with `include_str!` by a new
`src/adapters/remote_page.rs` (the routing table, the headers, a pure
`route(method, path) -> Option<Page>`). No bundler, no dependency. Tests
in "Tests" below.

**6. The device record.** A browser is a `PairedDevice` like any other,
listed and revocable in the Remote devices dialog (unchanged). Revoking
it deletes the record, so its next request is 401 and the host clears
the cookie in that refusal.

## The record

One new file, `web-cert.json`, beside `identity.json` in the data
directory (constant `WEB_CERT_FILE` in `src/ports/remote.rs` next to
`IDENTITY_FILE`, `:13`): `{schema_version, addresses, not_after,
certificate}`, the browser-facing certificate (DER, base64, public; the
private key stays in the Keychain) and the address set it covers. It is
written through the same atomic `save` the identity uses
(`src/adapters/remote.rs:77`, owner-readable), never any other way. Its
`schema_version` starts at 1 (`WEB_CERT_SCHEMA_VERSION`). It is derived
data: a missing, unreadable or newer-versioned file is re-made, never an
error, and deleting it costs one repeat warning. `Identity` is not
touched, so `IDENTITY_SCHEMA_VERSION` (`src/ports/remote.rs:17`, 1 today)
and `SCHEMA_VERSION` (`src/core/model.rs:12`, 16 today) do not move;
`identity.json`'s own certificate stays as written for the native
clients. Key rotation (a new identity) deletes `web-cert.json` with the
device records, since the old certificate is over the old key.
`devices.json` and `PairedDevice` (`src/ports/remote.rs:96`) are
unchanged: the cookie holds the same token whose SHA-256 the device
record already has (`token_sha256`, `src/adapters/remote.rs:345`),
looked up by `core.device_by_token` (`src/app.rs:3127`).

The page's own record lives on the device, not on the host: the pin
`{server_id, spki_sha256}` it last paired with, in `localStorage` under
try/catch (public values, so a script-readable store is fine), used only
to say "identity changed" versus "revoked". The page renders with
nothing in it.

## The signals

- `changed`/`removed`/`resync_required` are the existing frames; no new
  event, no new revision rule. The page applies §4.4 as the headset
  does: it keeps one held revision per key (`waiting`, `projects`,
  `sessions:<project>`, `session:<id>`, `conversation:<id>`,
  `screen:<id>`), applies a reply only if its envelope `revision` is at
  or past the held one, and on `epoch` change or `resync_required`
  refetches everything.
- **Live path.** Base: a timer refetches `waiting`, every expanded
  project's sessions and the open session's content, every 5 s (3 s when
  the open session is waiting). When the socket opens and `hello` lands,
  the timer relaxes to 30 s (the spec's own visible-view refetch,
  `docs/remote-client.md:256`) and `changed {waiting}`,
  `{sessions,id}`, `{session,id}` and the watched content kinds trigger
  the matching refetch at once. Metadata kinds need no `watch` frame; the
  open session sends `watch {conversation|screen, id}` and `unwatch` on
  leaving (`docs/remote-client.md:240,256`). "A session that starts
  waiting moves to the top without a reload" is `changed {waiting}` then
  a `waiting` refetch on the socket, and the next poll without it.
- **Failure words**, each one sentence in the page: a `fetch` that fails
  with no HTTP status is "The host is unreachable, or this browser does
  not trust its certificate" (facts, U2: the page never assumes the
  click-through outlives the session); a 401 on `/device` triggers
  `/info`, and a different `spki_sha256` or `server_id` from the pin is
  "Identity changed, pair again" (nothing more is sent), the same key
  with a pin stored is "This device was revoked", and no pin is the
  no-code screen; `pairing_closed`, `wrong_code` and `wrong_key` from
  `/pair` are worded plainly.

## The actions and effects

None in the core: no `AppAction`, no `Effect`, no change under
`src/core/`. The pairing window, the failure counter and `Paired`
already carry everything (`src/app.rs:3606-3665`). The work is in the
adapter and in `serve_remote`'s handlers, in this order:

1. **Spike first, `spikes/23-web-cookie/`.** CLAUDE.md requires one for a
   new risky mechanism. Do steps 2-5 minimally with a 20-line throwaway
   `web/` page that, on a `delivery: "cookie"` pair, does a `fetch` of
   `/device` and opens `new WebSocket` to `/api/v1/events`, and run it in
   Safari and Chrome on the Mac against a LAN bind (accept the warning).
   Record in the README: cookie stored, returned on `fetch`, returned on
   the upgrade, per browser and version; and, with the page already
   trusted, whether restarting the host with the same address (the
   persisted certificate) and then forcing a re-issue over the same key
   (delete `web-cert.json`) each bring the warning back, in Safari and
   in Chrome. The second result says how much the persistence buys; the
   first must be no warning, or step 2 is wrong. The owner repeats it on the
   iPhone (O1); the plan does not wait on that, the result only decides
   whether the socket ever opens there. If `fetch` fails in Safari, stop
   as in decision 2.
2. **Certificate with SANs, persisted.** `RemoteStore` (`:111`) gains
   `load_web_cert`/`save_web_cert` (fake in `adapters/fakes.rs` too).
   `tls_config` (`:144`) gains a parameter, the store and the loaded
   `WebCert`, and serves through a `ResolvesServerCert` holding one
   `CertifiedKey` and the address set it was made for (its call at
   `src/app.rs:2957` changes with it). On each handshake it compares
   `local_address()` (`:962`, a connected UDP socket, no packet sent)
   with the covered set; only when the address is not covered, or
   `not_after` is under 30 days away, does it re-make the certificate
   over the same `rcgen::KeyPair::from_pem(key)` and `save_web_cert`
   it (a failed save is logged and the certificate still serves). A
   restart with the same address loads the same DER, so a browser sees
   the same certificate. The certificate carries SANs `localhost`,
   `127.0.0.1`, `::1` and the address, `ExtendedKeyUsage::ServerAuth`,
   the same CommonName, and a 397-day validity (facts, Settled 5: iOS
   13+ limits). The key is the same, so `spki_sha256` is the same;
   native clients ignore dates and names (`pinned_client`, `:168`). The
   Bonjour `.local` name is not added: the QR carries an IP. What still
   repeats the warning: a change of the Mac's address (a new network, a
   DHCP lease), the yearly re-make, and a key rotation. The pairing
   window's line and §5 say so.
3. **Request and reply fields** (additive, `src/adapters/remote.rs`).
   `RemoteRequest` (`:360`, one literal, in `read_request`) gains
   `cookies: Vec<(String, String)>` (only names starting `__Host-sb-`,
   so other sites' cookies never enter memory), `origin:
   Option<String>`, `host: Option<String>`. `RemoteReply` (`:389`)
   gains `set_cookie: Option<String>`; every literal adds
   `set_cookie: None`. `respond` (`:530`) becomes a
   thin call of a general writer taking content type and extra headers; the status table gains nothing new.
4. **Pages on the connection thread.** In `connection` (`:770`), before
   `limited`, `remote_page::route` answers the three paths (decision 4).
5. **Cookie auth, `/device`, `Origin`.** In `authenticate` (`:3127`):
   a bearer wins; otherwise the cookie named for this installation. A
   cookie-authenticated request that is not a plain `GET`, and every
   upgrade, must carry `Origin: https://<Host header>` or it is 403
   `forbidden_origin`, logged by code. An unknown cookie token is 401 as
   today, with `set_cookie` clearing the cookie (`Max-Age=0`).
   `remote_pair` (`:3606`) reads `delivery`; for `"cookie"` it omits
   `token` and sets the cookie on the reply. `serve_remote` (`:3017`)
   gains, beside `events` and after `authenticate`, `GET device` ->
   `{device_id, name, scope}` through `envelope` (any paired scope). The
   `Referrer-Policy` and the unchanged `refuse` log (path without query)
   keep the code out of the log.
6. **The invite URL.** A pure `remote::invite_url(address:
   Option<IpAddr>, port, server_id, key, code) -> String` in
   `src/adapters/remote.rs` beside `local_address`, using
   `SocketAddr::new(ip, port)` so an IPv6 address is bracketed. No
   percent-encoding: the code is Crockford, the key hex, the id a UUID
   (the undecoded query, facts, tree).
7. **Swift.** `PairingInvite.parse` (`client/Sources/SwitchboardProtocol/
   Info.swift:112`) tries JSON, then a URL (`URLComponents`: `code`, `key`,
   `id`, host and port into `addresses`, a host with a colon rebracketed).
   `switchboard-remote pair` and `BrowserView` call `parse` and need no
   change. Run `swift test` in `client/` on the Mac (not run in
   planning).
8. **The page** (`web/`), plain DOM, `textContent` only (session names
   and agent output are untrusted; the last response is shown as
   `white-space: pre-wrap` text, not markdown). Sessions grouped by
   project, waiting first and marked in text as well as colour; the kind,
   the card word and `waiting_reason`; the session view shows the last
   response for a Claude Code session (falling back to `screen` on
   `no conversation`) and `screen` for every other kind; a reply box that
   posts `send` `{op: <uuid>, text}` only when `/device` says `control`;
   host name and scope in the header. A reply is submitted with a fresh
   `op` and, on a timeout, a status query (`GET ops/{op}`) and never a
   resend (`docs/remote-client.md` §4.5).

## The UI

`src/ui/remote.rs`, the pairing window (`:140-220`): `payload` becomes
`remote::invite_url(...)`; the QR, "Copy invite" and the "Address" line
follow. The window's help text (`:187-195`) gains, after the existing
sentence: "A browser pairs by this QR too: it will warn about the
certificate, which is expected for a host that signs its own, and warns
again when this Mac's address changes. A warning at any other time is
a reason to stop. The browser cannot check the host's key the way the apps do, so pair only
from this screen. The code works once." No new widget and no
`SWITCHBOARD_SCRIPT` line: `pairing <code>` (`src/script.rs:730`)
already puts the window in this state. README's dev-aid list is
unchanged.

## The tests

- **`src/adapters/remote_page.rs` unit tests**: `route` for each path,
  a `POST` and a `HEAD` (405 with `Allow`), `/index.html` and `//` (none).
- **Embedded page, `tests/remote.rs`**: `index.html` references only
  `/app.js`, `/app.css` and `data:`; no `<script>` body, no `style=`
  attribute, no `http(s)://` or `//host` URL anywhere in the three files;
  every `/api/v1/<first segment>` literal in `app.js` is in the set the
  host serves (`info`, `pair`, `device`, `projects`, `sessions`,
  `waiting`, `ops`, `events`).
- **The new paths over the real listener** (`call` at `:300` returns
  status and JSON only; add a sibling returning headers and raw body):
  200 unpaired for all three with the named `Content-Type`,
  `Cache-Control: no-store`, `nosniff`, `Referrer-Policy`, and the CSP on
  the HTML, containing no `unsafe-inline`; a query changes nothing.
- **The pairing flow end to end with the host's own client, as the CLI
  does** (no UI test drives a browser): take `invite_url`, parse its
  query, `GET /info` and compare `key` and `id`, `POST /pair` with
  `delivery: "cookie"` (reply has no `token`; `Set-Cookie` has `Secure`,
  `HttpOnly`, `SameSite=Strict`, `Path=/`, the `__Host-sb-` name), then
  with only the cookie: `GET /device` (scope), `GET /waiting`, `POST
  .../send` with the right `Origin` (control) and with a wrong one (403),
  `GET` of `events` upgrade with the right `Origin` (101, `hello`) and a
  wrong one (403); a bearer request is unchanged and a bearer wins over a
  wrong cookie; a read-scope device's `send` is 403 `scope`; revoke ->
  the next cookie request is 401 and the reply clears the cookie, and the
  subscriber is closed; key rotation -> 401 and `/info` shows the new key.
- **Certificate**: a rustls client with the stock webpki verifier and the
  presented certificate as its only anchor connects by `localhost`,
  `127.0.0.1` and the current `local_address()` (skips cleanly when there
  is no address), and `spki_sha256` is the same before and after a
  re-issue; a second `tls_config` over the saved `WebCert` and the same
  address serves byte-identical DER (no re-make), a different address or
  a `not_after` under 30 days re-makes and saves, and a corrupt
  `web-cert.json` is re-made without error. If the webpki client will not take an end-entity anchor,
  fall back to parsing the SANs from the DER.
- **`invite_url`** unit test: IPv4, IPv6 (bracketed), no address.
- **`tests/ui.rs:7424`** (`remote_devices_dialog_opens_pairing_and_revokes_a_device`):
  the copied invite is now the URL; assert it starts `https://`, has the
  window's code, key and id, and no JSON.
- **Fixtures**: add `client/Fixtures/invite_url.txt`, written by
  `write_fixtures` (`tests/remote.rs:1587`) from `invite_url`, decoded by
  a new `FixturesTests.swift` case through `PairingInvite.parse`
  (extends `the_client_fixtures_decode_on_both_sides`, `:1829`). `/device`
  gets no fixture: no Swift speaker reads it.
- **Not testable here, said plainly**: the page's script runs in no test
  (no browser, no JS runner, and the plan adds neither). Its logic is
  checked by the acceptance steps below, run by hand in a real browser
  on a LAN bind, and kept to small pure functions.
- **Acceptance by hand**: `cargo run --locked`; Remote devices, LAN,
  allow pairing with `control`; scan on a phone, accept the warning,
  see sessions; a session that starts waiting moves up (on the poll if
  the socket does not open); send a reply and watch it arrive in the
  pane; revoke the phone and see "revoked" on its next refresh. Also
  open the same QR text in Safari and Chrome on the Mac.

## Docs, in the same commit

- `docs/remote-client.md`: §4.1 bounds (three page paths, answered on the connection thread); §4.3 table (the three
  `GET` page paths and `GET /api/v1/device`); §5 (the pre-pairing line;
  the token sentence: bearer over the pinned connection **or** the
  host-set HttpOnly cookie for a browser, still never in a URL; the QR
  is the URL; the browser-trust paragraph from decision 1; the Origin
  rule; the browser certificate in `web-cert.json` and when its warning
  returns); `:207` (the excerpt is not a `SessionView` field); §9 untouched.
- `docs/design.md`: a "Web control surface" item in the Remote client
  status section (from `:3107`), with the known gaps: no excerpt on the
  wire, Safari socket unreliable, browser pairing not pinned, the
  warning returning on an address change or the yearly re-make, U1 and
  the re-issue result as measured in the spike.
- `README.md`: Layout table lines for `src/adapters/remote.rs` (`:383`),
  `src/ui/remote.rs` (`:423`), a new `src/adapters/remote_page.rs`, a
  new `web/` entry, and `spikes/23-web-cookie`.
- `CHANGELOG.md`, under Unreleased: "A browser can pair and control the
  host from the pairing window's QR code."

## For the owner, after this lands

- O1: on the iPhone or iPad, state the iOS version, does the cookie
  return on `fetch` and on the socket? If `fetch` fails, a token held by
  the script is the follow-up; if the socket works, the poll relaxes to
  a fallback only.
- Follow-ups named, not built: an `excerpt` on `SessionView`; the
  certificate download if the owner prefers it to the click-through
  (it would change §5's pre-pairing rule); raising the 4-slot pool if the
  spike shows a browser's parallel connections being closed at accept.

## Commit message

`Serve a browser control page from the remote port, paired by a URL QR and an HttpOnly cookie`

## Contract additions

- `RemoteRequest`: `cookies: Vec<(String, String)>`, `origin:
  Option<String>`, `host: Option<String>`.
- `RemoteReply`: `set_cookie: Option<String>`.
- `POST /api/v1/pair`: optional body field `delivery` (`"cookie"`); with
  it the reply omits `token` and sets the `__Host-sb-<id8>` cookie.
- `GET /api/v1/device` -> `{device_id, name, scope}`, any paired scope.
- Cookie authentication for every `/api/v1/` endpoint, with the
  `Origin` rule for non-`GET` requests and upgrades; error code
  `forbidden_origin` (403).
- Unauthenticated `GET` of `/`, `/app.js`, `/app.css`; new module
  `src/adapters/remote_page.rs`; new directory `web/`.
- `remote::invite_url`; the pairing QR and "Copy invite" now carry the
  URL (`PairingInvite.parse` accepts the URL and the JSON).
- `tls_config` serves a certificate re-issued over the same key with
  SANs and a 397-day validity, persisted and reused until the address
  or the expiry moves; it gains a store parameter. `spki_sha256` and
  `identity.json` are unchanged.
- New file `web-cert.json` (`WEB_CERT_FILE`, `WEB_CERT_SCHEMA_VERSION`
  = 1, type `WebCert`); `RemoteStore` gains `load_web_cert` and
  `save_web_cert`, with fakes.
- No change to `model.rs`, `AppAction`, `Effect`, `Identity` or any
  existing schema version.
