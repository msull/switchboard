# A web control surface on the remote port (issue #230)

A page the host serves from the remote port, opened in a browser on the
local network from the pairing window's QR code, that lists the sessions
with the waiting ones first, shows one session's last response or screen
live, and sends a reply under `control` scope. Same port, same protocol,
no build step: three files embedded at compile time.

## Where this departs from the issue

- **The page cannot learn its scope after the first load.** The issue
  has the header show "the device's scope", and spec §5 says "the host
  answers with the device's scope" on every request. No reply carries it
  today: `remote_pair` (`src/app.rs:3660`) is the one body with a
  `scope`, and the envelope (`src/app.rs:3730`) has `epoch`, `revision`
  and `body` only. A scope changed in the dialog therefore never reaches
  a page that was paired before it. The plan adds one additive field:
  `/info` answered to a request carrying a valid credential adds
  `device: {id, name, scope}`; without one it is what it is today. The
  page reads `/info` on every load, so a changed scope shows and hides
  the reply box on the next load, and a 403 `scope` on a send hides it
  at once.
- **There is no "one-line excerpt" on the wire.** The issue lists each
  session "with its kind, state and the one-line excerpt the card
  shows". `SessionView` (`control/src/lib.rs:637`) carries `card` (the
  state's word), `waiting`, `waiting_reason`, `trust_question` and
  `last_stop_at_ms`, and no field holds the last message; the Mac's card
  reads its excerpt from the conversation cache, which is content, read
  on the worker, never on the frame. The list shows the kind, the
  card's word, the waiting reason and the time of the last stop; the
  excerpt is the session view, one tap away. No new read model.
- **The page's files are answered on the connection thread, not on the
  frame.** The issue places them beside the API's allowlist in
  `serve_remote`. They are compile-time constants with no state behind
  them, so `connection` (`src/adapters/remote.rs:770`) answers them
  before the request reaches the channel, the way it answers 429 and
  503. `serve_remote`'s rule at `src/app.rs:3023`, everything outside
  `/api/v1/` refused, stands unchanged, and a page load costs no frame
  and no channel slot.
- **One QR, a URL; the JSON stays as accepted input only.** The headset
  has no camera and pairs from the Bonjour record and a typed code
  (`ConnectionManager.pair(_:code:)`), and its paste field and the CLI
  go through `PairingInvite.parse`. `parse` learns the URL; it keeps
  decoding the JSON because the Swift tests and the fixture test use
  that form, but nothing emits it any more: the QR and "Copy invite" are
  the URL.
- **Browser trust.** Decided below: the first visit's certificate
  warning is the trust step, said in the dialog. The certificate is not
  offered for download: `make_identity` (`src/adapters/remote.rs:95`)
  issues it with no subject alternative name, which iOS refuses to trust
  as a server certificate even when installed as a profile, and a
  rotated key would need every device to install it again.
- Nothing else; the device record, the pairing state and the events are
  used as they are.

## The record

No record change. A browser is a `PairedDevice` (`src/ports/remote.rs:96`)
like any other; its `name` comes from the `User-Agent` header when the
pair body has no `device_name` (`Safari on iPhone`, `Chrome on Mac`,
`Browser` when nothing is recognised), made by a pure function
`web::device_name(user_agent)` and passed to `AppCore::new_device`
(`src/core/remote.rs:166`) unchanged. `identity.json` and `devices.json`
keep their shapes: `IDENTITY_SCHEMA_VERSION` stays `1`, and nothing in
`src/core/model.rs` changes, so no `SCHEMA_VERSION` bump.

## The decisions the issue asked for

1. **Trust in the certificate: the click-through.** The native client
   pins the key; a browser cannot, and the certificate cannot be made
   trustable on the device without a SAN and a profile install that a
   key rotation would undo. So the first visit shows the browser's
   warning, the owner accepts it, and the browser keeps that exception
   for the address. What this buys: after the exception, a different
   certificate at the same address is a new warning, which is the
   browser's form of "identity changed"; and the `key` in the URL lets a
   genuine page refuse to post the code to a host whose `/info` key
   differs, so a URL relayed to the wrong host sends nothing. What it
   does not buy: a page served by an intermediary at scan time is not a
   genuine page, and the owner cannot tell. The exposure is a
   five-minute window, a single-use code, and an attacker already on the
   local network and in the path at that moment; the dialog says so in
   one sentence under the QR. §5's rule is amended to: before pairing a
   non-loopback bind serves `/info`, `/pair` and the page's three files,
   nothing else.
2. **The token reaches the WebSocket by a cookie.** A pair request whose
   body carries `"credential": "cookie"` is answered with
   `Set-Cookie: switchboard_token=<token>; Path=/api/v1; HttpOnly;
   Secure; SameSite=Strict; Max-Age=31536000` and a body without
   `token`; the script never sees it, `fetch` and `new WebSocket` on the
   same origin both carry it, and nothing on the device but the
   browser's cookie jar holds it. The bearer header stays for the
   native clients. `Sec-WebSocket-Protocol` was the alternative; it
   would put the token where the script reads it and so in
   `localStorage`. A cookie-authenticated request that carries an
   `Origin` header naming another origin than `https://<Host>` is
   refused 403 `origin` (`SameSite=Strict` already keeps the cookie off
   a cross-site request; the check is one string compare and covers an
   older browser). `localStorage` keeps only what is not secret: the
   `server_id`, the key hash, the host's name and the scope, so the
   page can tell "revoked" from "never paired" and "identity changed"
   from either.
3. **One QR.** `https://<address>:<port>/?code=<code>&key=<spki_sha256>&id=<server_id>`.
4. **Static paths.** Exactly three, `GET` only, any query ignored:
   `/` as `text/html; charset=utf-8`, `/app.css` as `text/css;
   charset=utf-8`, `/app.js` as `text/javascript; charset=utf-8`. Every
   one answers with `Cache-Control: no-store` (no caching across
   versions, and the whole page is under 40 KiB), `X-Content-Type-Options:
   nosniff`, `Referrer-Policy: no-referrer`, `Connection: close`, and
   on `/` the policy `default-src 'none'; script-src 'self'; style-src
   'self'; connect-src 'self' wss://<Host>; img-src 'self'; base-uri
   'none'; form-action 'none'; frame-ancestors 'none'` (the `Host`
   header's value, since Safari does not take `'self'` for a socket).
   `Connection: close` is what keeps a page load out of the pools: a
   browser opens its files in parallel, and a connection that closes
   with its reply never holds one of the four unauthenticated slots
   while idle; the later `/info`, `/pair` and API calls open fresh
   connections and move to the paired pool on their first authenticated
   reply as today. The three paths share one rate window of their own,
   `PAGE_PER_MINUTE = 60` per listener, beside `/pair`'s six and
   `/info`'s sixty (`src/adapters/remote.rs:583`). `/favicon.ico` is
   not served; the page declares `<link rel="icon" href="data:,">` so
   no browser asks. Opened with no code and no token, the page reads
   `/info` and shows the host's name and "Not paired. Open Remote
   devices on the Mac and scan the code."; while `pairing_open` is true
   it also shows the key's short form and a code field, as the
   headset's typed path does, and pins the key `/info` presented for
   that one post.
5. **Where it lives.** `web/index.html`, `web/app.css`, `web/app.js`,
   embedded by `src/web.rs` with `include_str!`; tests below.
6. **The device record.** As above: named from the user agent, scope
   from the pairing window, listed and revocable in the dialog as any
   device.

## The signals

No new signal. The page consumes the events of spec §4.4 as the headset
does: `hello`, `changed` for `projects`, `sessions` (per project),
`session`, `waiting`, `conversation` and `screen`, `removed`, and
`resync_required`; it sends `watch`/`unwatch` for the open session's
`conversation` (a Claude Code session) or `screen` (anything else). A
revocation closes the socket as today (`src/app.rs:486`), and the page's
next fetch answers 401, which it shows as "revoked".

## The actions and effects

No core change: no action, no effect, `AppCore::new_device` as it is.
The work is in the adapter, `app.rs`, the new `web` module and the UI.

- **`src/adapters/remote.rs`.** `RemoteRequest` (`:360`) gains `host`,
  `origin` and `user_agent` (`Option<String>` each) and
  `bearer_from_cookie: bool`; `read_request` (`:444`) fills `bearer`
  from the `Authorization` header or, when that is absent, from the
  `switchboard_token` cookie, setting the flag. `RemoteReply` (`:389`)
  gains `headers: Vec<(String, String)>` and `respond` (`:530`) writes
  them after `Content-Type`; a `Set-Cookie` arrives this way. A static
  path is answered in `connection` (`:770`) right after the rate check,
  from `web::asset(path)`, with the headers of decision 4 and the
  function returning after the write (`Connection: close`); `Limits`
  (`:623`) gets the page window. `MAX_HEAD` stays 8 KiB; a phone's
  cookie and user agent fit.
- **`src/web.rs`.** `Asset { body: &'static str, content_type:
  &'static str }`, `asset(path) -> Option<Asset>`, `headers(host) ->
  Vec<(String, String)>`, `device_name(user_agent) -> String`, and
  `const PATHS: [&str; 3]`. Pure, unit-tested, listed in the README.
- **`src/app.rs`.** `remote_pair` (`:3606`) reads `credential` from the
  body; for `cookie` it answers the `Set-Cookie` header and a body of
  `{scope, server_id, device_id}`; an empty `device_name` falls back to
  `web::device_name(request.user_agent)`. `remote_info` (`:3568`) takes
  the request, and when `authenticate` succeeds adds `device` to the
  body (the refusal is not returned: `/info` stays answered to anyone).
  `authenticate` (`:3127`) refuses a cookie credential whose `origin`
  is present and is not `https://<host>`. `serve_remote` is otherwise
  unchanged.
- **`src/ui/remote.rs`.** The QR payload (`:160`) becomes the URL; so
  does "Copy invite" (`:201`). With no local address the QR is not
  drawn and the line says "This Mac has no network address." When
  `listen.lan` is off the line says the page answers only on this Mac
  (the address in the URL is where the port would be, not where it
  is). A sentence under the code: "A phone's browser warns about the
  certificate on the first visit; accepting it is what trusts this Mac
  from that device. Pair on a network you trust."
- **The client.** `PairingInvite.parse` (`Info.swift:114`) accepts a
  `https://` URL: `code`, `key` and `id` from the query, the authority
  as the one address; a string that starts with `{` decodes as before.
  `main.swift:12` says "the QR code's URL or text"; `client/README.md`'s
  pair line likewise. No other Swift change.
- **The page** (`web/app.js`, plain ES2020, no framework). On load:
  read the query, `history.replaceState` to `/` before any request, then
  `GET /api/v1/info`. With `code`, `key` and `id`: compare `key` and
  `id` with `/info`, stop with "This is not the Mac whose code was
  scanned" on a mismatch, else `POST /api/v1/pair {code, spki_sha256:
  key, credential: "cookie"}`, keep `{server_id, key, name, scope}` in
  `localStorage`, and go on as paired. Without them: if `/info` carries
  `device`, paired; if `localStorage` names this `server_id` but
  `/info`'s key differs, "Identity changed; pair again"; if it names it
  and `device` is absent, "This device was revoked on the Mac; pair
  again"; else the not-paired text of decision 4. Paired: open
  `wss://<origin>/api/v1/events`, wait for `hello`, then `GET
  /projects`, `GET /waiting`, and `GET /projects/{id}/sessions` per
  project, applying every reply by `revision` per key as §4.2 says and
  refetching on `changed`. The list: projects as groups, within each the
  waiting sessions first then by `card` rank then name, each row the
  name, the kind, the card's word, the waiting reason and the last stop
  time. A tapped session: `GET /sessions/{id}/last-response` for a
  Claude Code session, else `GET /sessions/{id}/screen?lines=40`,
  `watch` on the matching key, refetch on its `changed`; a reply box
  posting `POST /sessions/{id}/send {op: crypto.randomUUID(), text}`
  shown only under `control`, disabled while a post is in flight, a
  `timeout` followed by `GET /ops/{op}` never a resend; a `Trust this
  folder` button when `trust_question` is set. The header: the host's
  name, the device's name and scope, a dot for the socket's state. A
  401 anywhere after pairing: the revoked text; `resync_required` or a
  closed socket: reconnect with backoff and refetch; a new `epoch`:
  refetch everything.

## The UI

Covered above: the pairing dialog's QR, invite and two sentences; the
page itself. Nothing else on the Mac changes.

## The tests

- **`src/web.rs` unit tests.** Every path in `PATHS` answers an asset
  and `/x`, `/index.html`, `/app.js/` do not; the HTML references only
  `/app.css`, `/app.js` and `/api/v1/` paths (every `href`, `src` and
  string literal starting with `/` in the three files is in `PATHS` or
  starts with `/api/v1/`), holds no `<script>` body, no `on*=`
  attribute and no `http://`; `device_name` on an iPhone Safari, a Mac
  Chrome, an iPad and an empty string.
- **`tests/remote.rs`**, with `http` (`:245`) growing optional extra
  headers and a header map in its result, and a `subscribe_with`
  variant of `subscribe_at` (`:1960`) that sends a `Cookie` header
  instead of the bearer:
  - unpaired, `GET /`, `/app.css` and `/app.js` answer 200 with the
    content types and headers of decision 4 and a `Connection: close`;
    `GET /other` is still 404 `not_found` (`:464` already asserts it);
    a sixty-first page request in a minute is 429;
  - the pairing flow end to end as the page does it: open the window,
    `GET /info` and compare the key, `POST /pair` with
    `credential: "cookie"` and no `device_name` under a `User-Agent` of
    an iPhone: the reply has `Set-Cookie` with `HttpOnly`, `Secure`,
    `SameSite=Strict` and `Path=/api/v1`, no `token` in the body, and
    the device is listed as `Safari on iPhone` with the window's scope;
    then `GET /projects` with only the cookie reads, `GET /info` with
    it carries `device` with that scope, and the events socket opened
    with the cookie says `hello` and `changed` when a session is
    renamed;
  - a cookie request with `Origin: https://elsewhere` is 403 `origin`;
    the same with the bearer header is answered;
  - after `RevokeDevice` the cookie's next request is 401 and its socket
    closes (the bearer variant at `:657` is the model);
  - the fixture test (`:1829`) gains `pair_cookie.json` (the cookie
    reply's body) and an `invite.txt` holding the URL the dialog emits,
    which `FixturesTests.swift` parses beside its JSON case.
- **`tests/ui.rs:7424`.** The invite copied is a URL whose query holds
  the code, the key and the id, and the page-trust sentence is on
  screen.
- **Swift.** `testInviteParsesTheQRText` keeps the JSON case and adds
  the URL, asserting the address came from the authority.
- **By hand, once, before the commit** (no UI test drives a browser):
  the acceptance list in the issue on a phone against a dev instance
  (`SWITCHBOARD_REMOTE_PORT`, `remote-listen lan`, `pairing <code>`),
  and the result noted in the design status.

## Docs

- `docs/remote-client.md`: §4.1 the page window and `Connection:
  close`; §4.3 the three static paths above the table and `credential`
  on `/pair`, `device` on `/info`; §5 the amended rule, the cookie and
  the click-through paragraph.
- `docs/design.md`, "Remote client (2026-10-12)": a bullet "The web
  control surface (2026-10-22)" after "A session opened from the
  headset", and the known-gaps bullet gains what the page leaves out
  (the conversation beyond the last response, drafts, tickets, runs,
  files, a "forget this host" that clears the cookie).
- `README.md`: Layout lines for `src/web.rs` and `web/`, the
  `tests/remote.rs` and `src/ui/remote.rs` lines, and the
  `SWITCHBOARD_REMOTE_PORT` paragraph saying the port serves the page
  at `/`.
- `CHANGELOG.md` under Unreleased: "The remote port serves a page: scan
  the pairing QR with a phone, accept the certificate once, and see the
  sessions with the waiting ones first, a session's last response or
  screen live, and a reply box under the control scope."

## Commit

`The remote port serves a browser page paired from the QR code's URL`

## Contract additions

- `adapters::remote::RemoteRequest`: `host: Option<String>`,
  `origin: Option<String>`, `user_agent: Option<String>`,
  `bearer_from_cookie: bool`.
- `adapters::remote::RemoteReply`: `headers: Vec<(String, String)>`.
- `adapters::remote`: `PAGE_PER_MINUTE`.
- `web` (new module): `Asset`, `asset`, `headers`, `device_name`,
  `PATHS`.
- Wire: `POST /api/v1/pair` body field `credential` (`"cookie"`), its
  reply without `token` and with `Set-Cookie`; `GET /api/v1/info` body
  field `device {id, name, scope}` when the request is authenticated;
  `GET /`, `GET /app.css`, `GET /app.js` before pairing.
- `SwitchboardProtocol.PairingInvite.parse`: accepts the pairing URL.
- No `AppAction`, `Effect`, `model.rs` or `ports/` change; no schema
  bump.
