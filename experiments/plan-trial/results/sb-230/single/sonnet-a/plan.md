# Plan: a web control surface on the remote port (issue #230)

The host serves a small page from the remote port; a browser on the local
network pairs from a URL in the pairing window's QR, lists the sessions
with the waiting ones first, shows a session's last response or screen,
and sends a reply. Three files under `web/`, embedded with `include_str!`;
no build step, no dependency. The page speaks the protocol the headset
speaks.

## Where this departs from the issue

- **"The one-line excerpt the card shows" is not on the wire.**
  `docs/remote-client.md` §4.3 says the excerpt is `last_message` from the
  `Stop` hook, "a field of the session's metadata". `SessionView`
  (`control/src/lib.rs:637`) has no such field, and the record does not
  keep the hook's `last_message` (it exists only on the hook event,
  `src/adapters/hooks.rs:71`). The Mac derives an agent card's excerpt in
  the UI from the cached conversation (`agent_excerpt`,
  `src/ui/cards.rs:558`) and a shell's from pane captions held in
  `UiState`. Putting an excerpt on `SessionView` means masking it on the
  frame, which §4.1 keeps off the frame. So the list shows the card word
  (`card`), kind, liveness and `waiting_reason`, and the page fetches
  `last-response` for **waiting Claude Code sessions only**, taking the
  first non-empty line with `agent_excerpt`'s rule (strip leading
  `#-*>`, 160 characters). The fetch runs when a session becomes waiting
  or its `last_stop_at_ms` moves, and the answer is masked by the port
  like any other. An excerpt on every row waits for a host-side field in
  a later issue. §4.3's sentence is corrected in this commit.
- **No endpoint tells a paired device its own scope.** §5 says "the host
  answers with the device's scope" on later requests; `app.rs` does not
  (`authenticate`, `src/app.rs:3127`, returns the device and nothing
  reaches the reply). The headset keeps the scope from the `/pair` reply
  (`ServerConnection.swift:232`). A page loaded from a cookie has no pair
  reply, and a scope changed in the dialog would never reach it. The page
  cannot do without one, so this adds `GET /api/v1/device`
  (`{device: {id, name, scope}}`) and corrects §5.
- **A bearer-only `RemoteRequest` cannot carry a browser.** The request
  keeps only the bearer, query and body (`src/adapters/remote.rs:360`). A
  cookie, an `Origin` check and a name from the user agent need four more
  headers read, and a `Set-Cookie` needs a field on `RemoteReply`.
- **The certificate has no subject alternative names**
  (`make_identity`, `src/adapters/remote.rs:102`, an empty name list and a
  common name only). A browser's warning is therefore a name mismatch as
  well as an unknown issuer, whatever address is used. This decides the
  trust question below.
- **The pairing window is loopback-only by default.** The QR carries
  `local_address()` whether or not the owner has allowed the local network
  (`src/ui/remote.rs:171`), so a phone scans an address that cannot
  answer. The URL uses the LAN address only when `identity.listen.lan`,
  and `127.0.0.1` otherwise, with a sentence saying a phone cannot reach it
  (the Mac's own browser can).
- **The browser-trust claims are not verified.** That a click-through
  carries to a WebSocket and to a `Secure` cookie, and for how long, differs
  by browser and I did not run one. CLAUDE.md requires a spike before a
  risky mechanism: step 1 below is that spike, and the page has a fallback
  for the case it fails.

Nothing else in the issue contradicts the code: only `/api/v1/` is served
(`src/app.rs:3023`), the pools are 4 and 12 (`src/adapters/remote.rs:567`),
and the QR is the JSON the issue names.

## The decisions

**1. Browser trust: the click-through is the trust step; the host does not
offer its certificate.** A downloaded certificate trusted on the device
would still fail the name check, because the certificate has no names and
an address that DHCP changes cannot be put in one without reissuing the
certificate on every change. And the download would arrive over the same
unauthenticated connection, so it would carry no more than the click.
What the click buys: the owner reached the address from a QR on the host's
own screen, and the browser then holds that certificate for that address.
What the `key` in the URL buys: a genuine page refuses to send the code to
a host whose `/info` key differs, which catches a stale QR, a rotated key
and an address reassigned to another Switchboard, and does not burn a
wrong-code attempt. What nothing buys: authentication of the host. The
browser cannot read the certificate, so the page compares the URL against
what `/info` *says*, and a page served by an impostor is not a genuine
page. An impostor on the path at first visit gets the code and the token;
the defence is the network, and the dialog says so. The page shows the
host's name and short key (`KeyFingerprint.short` form) during pairing for
a comparison by eye, as the headset does. §5's rule becomes: before
pairing a non-loopback bind serves `/info`, `/pair` and the three page
files, nothing else.

Dialog sentence, under the pairing section: "A browser cannot check this
Mac's key as the headset does, so it asks you to accept this Mac's
certificate once. Accept it only on a page you reached by scanning this
window's code, on your own network. A certificate warning at a later visit
means the key changed or something else answers at that address: do not
accept it. A paired browser reads, and with Control scope types into, your
sessions until you revoke it here."

**2. The token reaches the WebSocket as a cookie.** `/pair` answers with
`Set-Cookie: __Host-sb-<first 8 hex of server_id>=<token>; Path=/; Secure;
HttpOnly; SameSite=Strict; Max-Age=34560000` (400 days, the longest a
browser keeps). `fetch` and `new WebSocket` carry it; the page's script
never holds it, and it is not in `localStorage`, which WebKit also evicts
after seven days without a visit. The name carries the server id because a
cookie is scoped by host, not port, and two instances on one machine would
otherwise overwrite each other. Costs, stated: the cookie is sent to any
other HTTPS service on that address; `SameSite=Strict` does not stop a
page on another port of the same address, so a cookie-authenticated `POST`
or upgrade must carry `Origin: https://<Host header>` or is refused 403
`origin`; and the token is not stored, so the cookie cannot be refreshed
and the device re-pairs after 400 days. A bearer header still
authenticates, wins when both are present, and needs no `Origin`. When the
body of `/pair` carries `"cookie": true` the reply omits `token` and sets
the cookie; without it the reply is today's, so the native clients are
unchanged. `Sec-WebSocket-Protocol` is rejected: the script would have to
hold the token, so it needs script-readable storage.

An unknown cookie token answers 401 `revoked`; no credential answers 401
`unauthorized`; the bearer path keeps `unauthorized` for both.

**3. One URL.** The QR and the text beside it become
`https://<address>:<port>/?code=<code>&key=<spki_sha256>&id=<server_id>`.
The values are hex, a UUID and Crockford, so nothing is percent-encoded.
The host stops producing the JSON. `PairingInvite.parse` (Swift) accepts
the URL, taking `host:port` from the authority as the one address
(brackets kept for IPv6), and still accepts JSON for an invite copied
earlier: no change to `ConnectionManager` or the headset's typed-code
path. The CLI usage line and `client/README.md` say URL.

**4. Static paths.** Exactly `GET /` (`text/html; charset=utf-8`),
`GET /app.js` (`text/javascript; charset=utf-8`) and `GET /app.css`
(`text/css; charset=utf-8`), matched on the whole path with the query
dropped (the page load carries the code in its query). Other methods on
those paths answer 405 with `Allow: GET`; every other path outside
`/api/v1/` keeps today's 404 from the frame. They are answered **on the
connection thread**, before the rate check and the channel, so a page load
costs the frame nothing and works while the frame is busy. Each carries:

```
Content-Security-Policy: default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'
Cache-Control: no-store
X-Content-Type-Options: nosniff
Referrer-Policy: no-referrer
Connection: close
```

`no-store` rather than validators, so a page never outlives its binary
and there is no 304 path to write; the files are tens of kilobytes on a
local network. `Connection: close` frees the unauthenticated slot at once,
since a browser opens up to six connections and the pool holds four (the
extra accepts are closed and the browser retries). A page load counts
against the **unauthenticated pool** (a connection moves to the paired
pool only on its first authenticated API reply, as now) and a new rate
window, `PAGE_PER_MINUTE = 120` per listener, answered 429 beyond it
(about four loads' worth of requests each, away from `/info`'s 60 and
`/pair`'s 6). The HTML has no inline script or style, no `<form>`, and a
`data:,` icon so there is no favicon request. `connect-src 'self'` covers
the `wss:` socket in current browsers; an older Safari that does not is
the case the polling fallback exists for. The JSON replies gain
`Cache-Control: no-store` and `X-Content-Type-Options: nosniff`.

Opened with no code and no cookie, the page reads `/info` and shows the
host's name, its short key, "This browser is not paired", and, when
`pairing_open`, a code field: the owner compares the key with the window
by eye and types the code, which is the headset's path. It sends nothing
else.

**5. Where it lives:** `web/index.html`, `web/app.css`, `web/app.js`,
embedded by `src/adapters/web.rs` (below). Tests below.

**6. The device record.** A browser is a `PairedDevice` like any other.
Its name is `device_label(user_agent)` ("Safari on iPhone", "Chrome on
Mac", "Firefox on Windows", "Browser" when nothing matches; iPadOS Safari
presents as a Mac and is labelled so), unless the body names one. Scope is
the window's, as today. The dialog lists and revokes it with no change.

## The record

No change: `PairedDevice` (`src/ports/remote.rs:96`) and `devices.json`
keep their shape, so `SCHEMA_VERSION` (16 in `src/core/model.rs:12`),
`VIEWS_SCHEMA_VERSION` (7, `model.rs:612`) and `IDENTITY_SCHEMA_VERSION`
(1, `src/ports/remote.rs:17`) stay. Should a ticket landing first have
changed one of them, this commit still bumps nothing.

## The signals

No new event. The page uses `hello`, `changed {session|sessions|projects|
conversation|screen}`, `removed`, `watching`, `resync_required` and the
`watch`/`unwatch` frames as §4.4 has them, and the per-key held revision
of §4.2. A scope change in the dialog emits nothing, so the page re-reads
`/device` on a 403 `scope`, on every reconnect and when the tab becomes
visible.

## Actions and effects

No `AppAction` and no `Effect`: pairing already goes through `Paired`, and
a cookie-authenticated request dispatches `DeviceSeen` as a bearer one
does. No port trait changes. The work is in the adapter and in
`serve_remote`'s helpers.

- `src/adapters/web.rs` (new): the three assets as `&'static str` from
  `include_str!("../../web/…")`, `asset(method, path) -> Option<Page>`
  with the headers above, and `CSP`. An exact match on three literals, so
  there is no path to traverse. (`include_str!` takes a path relative to
  the file; `include_str!` of a missing file fails the build, which is the
  check that `web/` is complete.)
- `src/adapters/remote.rs`: `read_request` also keeps `cookie`, `origin`,
  `host` and `user_agent` on `RemoteRequest`; `RemoteReply` gains
  `set_cookie: Option<String>`; `respond` takes the extra headers (the
  ~30 `RemoteReply { … }` literals, all in `app.rs` bar one, take the new
  field through a `RemoteReply::new(status, body, authenticated)` so the
  edit is mechanical); `connection` answers a static path before
  `limits.limited`, with `Connection: close`, and returns; `Limits` gains
  the `page` window; `pub fn cookie_name(server_id)`. The raw cookie and
  `set_cookie` are never logged (`refuse` logs method, path and code, and
  the query is already split off the path).
- `src/app.rs`: `authenticate` tries the bearer, then the cookie named by
  `cookie_name`, applying the `Origin` rule (non-GET or upgrade must carry
  an `Origin` equal to `https://` + `Host`); `remote_pair` honours
  `"cookie": true` and takes the name from `device_label`; a `("GET",
  ["device"])` arm beside `projects`; `capabilities` in `remote_info`
  gains `web` and `device`.
- `src/core/remote.rs` (pure, tested in `src/core/tests.rs`):
  `pairing_url(identity, window, addr: SocketAddr) -> String` (a
  `SocketAddr` formats IPv6 with brackets) and `device_label(&str)`.

### The page (`web/app.js`)

Plain script, no modules, DOM built with `createElement` and
`textContent` only: transcript and screen text is agent output and must
never reach `innerHTML`. In order:

1. Read `location.search` for `code`, `key`, `id`, then
   `history.replaceState(null, '', location.pathname)` — the first
   statements, before any `await` or request. The code is single-use and
   dies in five minutes, so a browser that keeps the first URL in its
   history keeps a spent code.
2. `GET /api/v1/info`. With a code: refuse to post unless `spki_sha256`
   equals `key` and `server_id` equals `id`, and say "this address is not
   the Mac the code was made for" without posting. On a match,
   `POST /api/v1/pair {code, spki_sha256: key, cookie: true}`; remember
   `{server_id, spki_sha256}` in `localStorage` (not secret).
3. Without a code: a remembered key that differs from `/info`'s is
   "identity changed, pair again" and the page makes no further request.
   Otherwise `GET /api/v1/device`: 401 `revoked` is "revoked, pair again",
   401 `unauthorized` is the unpaired view; a `/device` that fails right
   after a successful pair says the browser kept no cookie, so the owner
   revokes it in the dialog.
4. Subscribe first, then snapshot (§4.4): open `wss://<host>/api/v1/events`,
   wait for `hello`, fetch `projects` then each project's `sessions` (at
   most six at once), apply buffered events by revision. A reply is applied
   only at or past the held revision of its key; a new `epoch` refetches
   all; `resync_required` or a close reconnects with 1, 2, 4 … 30 s
   backoff, showing the last view marked stale.
5. If the socket fails twice, poll `projects`/`sessions` and the open
   session every 4 s. Replies still apply by revision. This is the answer
   to a browser that will not open `wss:` under a clicked-through
   certificate.
6. List: projects with a waiting session first, then by name; within a
   project waiting first. A waiting row says "waiting on you" in words
   with its reason, not by colour alone; each row has kind, `card` word
   and the excerpt of the first decision above.
7. Session view: a Claude Code session shows `last-response` (`text`,
   truncated mark), anything else `screen?lines=80`; it sends `watch` for
   `conversation` or `screen` (id = record id), fetches after `watching`,
   refetches on `changed` past the held revision and every 30 s, and
   sends `unwatch` on leaving. A Claude session with no conversation yet
   falls back to `screen`. The reply box shows only when `/device` says
   `control`: `POST /sessions/{id}/send {op, text}` with a
   `crypto.getRandomValues` UUID (no secure-context API), then §4.5's
   rule on a timeout — query `/ops/{op}`, resend only on `unknown`. A
   `trust_question` shows "answer the folder question on the Mac".
8. Header: host name, device scope, connection state.

## The UI

The Remote devices dialog (`src/ui/remote.rs`) only draws, so it takes
the URL from `core::remote::pairing_url`: the QR encodes it, "Copy invite"
copies it, and the text beside the QR shows it under the code. The address
is the LAN address when `identity.listen.lan`, else `127.0.0.1` with the
sentence "Listening on this Mac only; a phone cannot reach this address."
The decision-1 sentence goes under the pairing section. No change to the
rest.

## The tests

- `src/core/tests.rs`: `pairing_url` (IPv4, IPv6 with brackets, the three
  parameters); `device_label` over real user-agent strings for Safari on
  iPhone, Chrome on iOS, Edge on Windows, Firefox on Linux, an iPad
  presenting as a Mac, and an empty string.
- `src/adapters/remote.rs` unit tests: `read_request` keeps the cookie,
  origin, host and user agent; the `page` window slides; `cookie_name`
  differs per server id.
- `tests/remote.rs` (helpers extended to return headers and to send
  `Cookie`, `Origin` and `User-Agent`):
  - the three paths answer 200 with the exact content types and the
    headers of decision 4, with no frame serving (`serve_pending` not
    called), `Connection: close`, and the query ignored; `POST /` is 405;
    `/index.html` and `/app.js/` keep the 404; ten sequential loads never
    fill the unauthenticated pool; the 121st in a minute is 429.
  - the page references only what the host serves: the `src`/`href` in
    `index.html` are among the three paths, and every `/api/v1/…` literal in
    `app.js` (with `${…}` replaced by an id) answers something other than
    the "not available on this port" refusal, GET or POST as the call
    site says. Static guards over the three files: no `http://`,
    `https://` or `//host` URL, no inline `<script>` body, no `style=` or
    `on…=` attribute, no `<form>`, and none of `innerHTML`, `outerHTML`,
    `insertAdjacentHTML`, `document.write`, `eval(`, `new Function` in
    `app.js`.
  - pairing end to end through the host's own client, as the CLI does:
    open the window, build the URL with `pairing_url`, parse its query by
    hand, read `/info` and compare the key, `POST /pair` with
    `cookie: true` and a Safari-on-iPhone user agent; the reply has no
    `token`, one `Set-Cookie` with `__Host-sb-…`, `Secure`, `HttpOnly`,
    `SameSite=Strict` and the `Max-Age`; the device is named "Safari on
    iPhone"; `/device`, `projects` and `sessions` read with the cookie
    alone; a spent code, a wrong key and a wrong code behave as today
    (a wrong key counts). A pair without `cookie` still returns the token
    and sets nothing.
  - cookie safety: `POST …/send` with the cookie and no `Origin`, or an
    `Origin` for another port, is 403 `origin` and types nothing; with
    the right `Origin` and a control scope it types once per op; a read
    device is 403 `scope`; the events upgrade needs the same `Origin`; a
    bearer request needs none.
  - revoke: after `RevokeDevice` the cookie's next request is 401
    `revoked` and an open socket closes (the shape of
    `revoke_closes_the_devices_socket`); a made-up cookie is `revoked`, no
    cookie `unauthorized`.
  - `device.json` fixture added to `write_fixtures` and checked in
    `the_client_fixtures_decode_on_both_sides`; `info.json` is rewritten
    for the two capabilities. No Swift type reads `device.json`.
- `tests/ui.rs`: `remote_devices_dialog_opens_pairing_and_revokes_a_device`
  (`:7424`) asserts the copied invite is an `https://127.0.0.1:<port>/`
  URL carrying `code`, `key` and `id`, instead of JSON, and finds the
  loopback sentence and the browser-trust sentence.
- `client/Tests/SwitchboardProtocolTests/FixturesTests.swift:159`: the URL
  form and the JSON form parse to the same invite. Not in the hook or CI;
  run with `swift test` in `client/`.
- No test executes the page's JavaScript. That is a gap, said here and in
  the design status.

Acceptance, run by hand on a throwaway data directory with
`SWITCHBOARD_SCRIPT` lines `remote-listen lan <port>` and `pairing <code>`
and reported as run or not run: scan the QR on an iPhone, accept the
warning, see the sessions; inject a `PermissionRequest` with
`switchboard-hook` and watch the session move to the top without a reload;
send a reply with a Control window; revoke in the dialog and see "revoked"
on the next request. Repeat once on a desktop browser with the socket
blocked, to see the polling fallback.

## Steps (one commit, the app runs at each)

1. `spikes/23-browser-pairing/`: the host's real certificate, a page that
   sets the cookie and opens `wss:` after a click-through; run on Safari
   on iPhone, Safari and Chrome on a Mac; record whether the cookie is
   kept, whether the socket opens, and whether `connect-src 'self'`
   covers it, in the spike's README. If the cookie is refused, switch
   decision 2 to `Sec-WebSocket-Protocol` and `localStorage` before the
   rest; if only the socket fails, the polling fallback is the plan.
2. `core::remote` functions and tests; `RemoteRequest`/`RemoteReply`
   fields, `web.rs`, the static answers, the page window.
3. `authenticate`, `remote_pair`, `/device`, `capabilities`; the
   `tests/remote.rs` cases.
4. `web/*`; the static guards.
5. Dialog, `tests/ui.rs`, Swift `parse`, CLI usage, `client/README.md`.
6. Docs: `docs/remote-client.md` §4.1 (page pool and window), §4.3 (the
   three paths, `/device`, the excerpt sentence), §5 (URL invite, cookie,
   `Origin`, browser trust, the corrected scope sentence, "never in a URL"
   unchanged), §13 rows "Browser trust" and "Browser credential"; a
   "Web page (2026-10-22)" entry in the Remote status section of
   `docs/design.md` listing the gaps (no JS test, excerpt on waiting
   sessions only, no tickets, 400-day cookie, the spike's findings);
   README Layout rows for `web/` and `src/adapters/web.rs`, the
   `tests/remote.rs` row, and the Development paragraph at `:277` (the
   invite is a URL a browser opens); a `CHANGELOG.md` line under
   Unreleased.

Commit message: `Serve a browser page on the remote port, paired from the URL in the pairing window's QR`

## Contract additions

- HTTP: `GET /`, `GET /app.js`, `GET /app.css` (answered by the connection
  thread, no auth); `GET /api/v1/device` → `{device: {id, name, scope}}`
  (any paired device); `POST /api/v1/pair` accepts `"cookie": true`, then
  omits `token` and sets the `__Host-sb-<server id prefix>` cookie;
  `/info` `capabilities` gains `web` and `device`; errors `revoked` (401)
  and `origin` (403); JSON replies carry `Cache-Control: no-store` and
  `X-Content-Type-Options: nosniff`.
- Invite text is the URL `https://<address>:<port>/?code=&key=&id=`; the
  JSON is no longer produced and is still accepted by `PairingInvite.parse`.
- `adapters::remote`: `RemoteRequest.{cookie, origin, host, user_agent}`,
  `RemoteReply.set_cookie`, `RemoteReply::new`, `cookie_name`,
  `PAGE_PER_MINUTE`. `adapters::web` (new): `asset`, `CSP`.
- `core::remote`: `pairing_url`, `device_label`.
- Files: `web/index.html`, `web/app.css`, `web/app.js`,
  `spikes/23-browser-pairing/`, `client/Fixtures/device.json`.
- No change to `model.rs`, `AppAction`, `Effect`, a `ports/` trait, or any
  schema version.
