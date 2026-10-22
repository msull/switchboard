# Plan: a web control surface on the remote port (#230)

The remote port also serves a page: three files compiled into the
binary and answered by the connection thread. The page pairs from a
URL in the pairing window's QR code and then speaks the protocol the
headset speaks. The browser's click-through of the certificate warning
is the trust step. The token stays in the page's `localStorage`. The
page sends it as the bearer header on `fetch` and as a
`Sec-WebSocket-Protocol` value on the events socket. One commit. The
app runs before and after it. Line numbers are from this worktree at
7fe8949.

## Where this departs from the issue

- **"Any path outside `/api/v1/` is refused."** True, but the refusal
  happens on the frame (`src/app.rs:3023-3025`), after the request has
  already taken a slot in the inbox of 64. The plan answers every path
  outside `/api/v1/` on the connection thread instead: the page's three
  files, or the same `404 not_found` body as today, then a close. The
  frame never sees such a request. The prefix arm in `serve_remote`
  stays as a guard.
- **"The one-line excerpt the card shows."** The wire does not carry
  it. The record has no `last_message`, and the card computes the
  excerpt in the UI from the conversation cache or the pane caption
  (Settled: "The record carries no `last_message`";
  `src/ui/cards.rs:176-184,558-574`). Putting transcript text on
  `SessionView` would put content on a metadata view that skips the
  content worker's redaction path (§4.1, "The frame does core state
  only"). In this cut, a waiting Claude Code session's row shows the
  first line of its `last-response`, cut the way `agent_excerpt` cuts
  it (160 characters), for at most the first 8 waiting Claude Code rows
  in the list's order. Every other row shows the card's word and
  `waiting_reason`. Each such read puts its key in the port's watch set
  for a minute, since the set holds "every content key read in the last
  minute" (`docs/remote-client.md:254`; facts For the owner, "Card
  excerpt": the per-session fetch "spends watch-set slots"). So the
  excerpts are fetched only at load and on that session's own
  `changed session`, never on a timer, and the page holds at most 9
  keys of the 128 at once: 8 excerpts and the open view's one. This rests on the For the owner line "Card
  excerpt".
- **"The device's scope … in the header."** No endpoint tells a device
  its scope after pairing. `/info` is unauthenticated
  (`src/app.rs:3568-3600`). The plan adds `GET /api/v1/device` (see
  Actions).
- **"'Revoked' said plainly."** The host cannot say it. Both refusals
  in `authenticate` have the code `unauthorized`
  (`src/app.rs:3127-3153`). A revoked token's hash is gone from
  `devices.json` (`RevokeDevice`, `src/core/remote.rs:86-92`), so the
  host cannot tell a revoked token from one it never issued. The page
  infers it instead: it holds a token for this `server_id`, `/info`
  still shows the same key, and the answer is 401. That page reads
  "This device was revoked on the host." This rests on the For the
  owner line "Revoked versus never paired".
- **"Compares the key in the URL with the one `/info` presents."** This
  proves only what `/info` claims. Whether a page can read its own
  connection's certificate is Unresolved. The host checks only the
  `spki_sha256` in the body (Settled, §5 relay line;
  `src/app.rs:3639-3647`). The check catches a genuine page talking to
  the wrong genuine host. It does not catch an impostor that serves its
  own page. The spec's relay sentence (`docs/remote-client.md:309`) is
  amended to say so for browsers.
- **"The code never stays in the address bar or the history."**
  `replaceState` rewrites only the session-history entry. Whether the
  URL reaches Safari's global history or iCloud Tabs first is
  Unresolved. What actually protects the code is that it is single use
  and lasts five minutes (Settled: "A code is single use"). The spec
  says that, and treats `replaceState` as tidying, not as the
  guarantee.

## The record

No change. A browser is a `PairedDevice` in `devices.json`
(`src/ports/remote.rs:96-107`, an unversioned `Vec`). The page sends
its name as `/pair`'s `device_name`. `SCHEMA_VERSION` stays 16
(`src/core/model.rs:12`) and `IDENTITY_SCHEMA_VERSION` stays 1
(`src/ports/remote.rs:17`). The certificate in `identity.json` is
untouched.

## The actions and effects

There is no new `AppAction` or `Effect`. The only frame-side addition is
one read-only endpoint. Everything else is in the listener, the UI's
payload and the page.

**1. Trust: the click-through is the trust step.** On the first visit
the owner sees a certificate warning and accepts it. The device then
trusts that certificate at that address. The paired-devices dialog says
so (see UI).

- Chrome and Firefox apply the exception to the page's own socket
  (Settled, Chrome and Firefox lines).
- What it buys: the page's key check refuses to send the code to a
  genuine host whose key differs.
- What it does not buy: a page served by an impostor is not our page,
  and the code in the URL reaches whatever host the owner clicked
  through to.
- What rests on Unresolved: whether Safari honours the exception for
  `wss://` (WebKit bug 41419, still NEW). This is the question the plan
  turns on most. The page therefore ships its own fallback. A failed
  handshake tells the script nothing: a non-101 answer fails the
  connection, and the page sees `error` then `close` 1006 with no
  status, the same as a host that is down (WHATWG WebSockets, §2.2 and
  §4, https://websockets.spec.whatwg.org/). So after any socket failure
  or close the page makes one `GET device` before it reconnects or
  counts anything:
  - 401: the revoked state (see UI); no reconnect.
  - A network error: the host is unreachable; back off and reconnect,
    counting no failure.
  - 200: one failure toward the fallback.

  Two such failures before `hello` switch the page to polling
  `projects` and each project's `sessions` every 5 s. While polling it
  tries the socket again once a minute, and a `hello` returns it to
  live, so a fall back after a relaunch does not stick. It refetches the open view's content
  only when that session's `last_activity_ms` or `last_stop_at_ms`
  moves, never on a timer, so polling never parses a transcript for
  nothing. The header says "Live updates unavailable; refreshing every
  5 s". The fallback is built now because the answer only comes from
  the owner's device run at acceptance.

**2. Token transport: the `Sec-WebSocket-Protocol` subprotocol, with the
token in `localStorage`.**

- REST is unchanged. `fetch` sets `Authorization: Bearer` itself, so
  the host keeps reading the bearer header for every client.
- For the socket, the page calls
  `new WebSocket(url, ["switchboard.v1", "bearer." + token])`.
- `read_request` (`src/adapters/remote.rs:444-513`): when `upgrade` is
  set and there is no `Authorization` header, it splits
  `sec-websocket-protocol` on commas. If `switchboard.v1` is among the
  values, the `bearer.`-prefixed value becomes `bearer` and a new field
  `subprotocol: Option<String>` is set to `switchboard.v1`. The token
  stays in the one field that is never logged.
- `connection` clones `subprotocol` next to `upgrade_key` (`:797`).
  `respond_upgrade` (`:519-527`) takes it and writes
  `Sec-WebSocket-Protocol: switchboard.v1`. It echoes the name, never
  the token. The browser fails the handshake without that echo
  (Settled). The token's characters are all `tchar` (Settled).
- A native client offers no subprotocol and gets no echo, so its path
  is unchanged.
- What the subprotocol rests on: `localStorage` is per origin, so two
  instances on two ports never share a token (Settled). It does not
  need a secure context: the HTML standard's `WindowLocalStorage` IDL
  carries no `[SecureContext]`
  (https://html.spec.whatwg.org/multipage/webstorage.html). The same
  section lets the getter throw `SecurityError` when the user agent
  forbids persistence (a private window, blocked site data). The page
  then keeps the token in memory and says "This browser will not keep
  the pairing; it lasts while this tab is open."
- The cost: any script on the origin can read `localStorage`. The
  origin serves only our three files under a CSP with
  `script-src 'self'`, and the page never turns data into markup (a
  test checks this).
- There is no `Origin` check. A browser never attaches a bearer token
  or a subprotocol by itself, so a cross-site page has nothing to
  replay. This answers the For the owner line "Check `Origin`" the way
  that line says the subprotocol choice implies.
- Op ids come from `crypto.getRandomValues` (16 bytes as hex, within
  `OP_CAP` = 64), never from `randomUUID` (Settled: it does not need a
  secure context).

**3. One QR: a URL.**

- The payload is
  `https://<ip>:<port>/?code=<code>&key=<spki_sha256>&id=<server_id>`.
- It is built by a pure function,
  `core::remote::invite_url(identity, code, ip, port)`. The UI calls it
  in place of the `json!` at `src/ui/remote.rs:160-166`.
- The host part is `remote::local_address()`, which is always IPv4, so
  there are no brackets (Settled). When there is no address it falls
  back to `127.0.0.1`. That is honest for a loopback-only listen.
- IP, because `.local` resolution on the device is Unresolved.
- The native text path takes the URL too. `PairingInvite.parse`
  (`client/Sources/SwitchboardProtocol/Info.swift:113-116`) tries
  `URLComponents` first: `https` scheme, `code`, `key`, `id`, and the
  host and port as the one entry in `addresses`. That is enough, since
  only the first address is used (Settled: Swift line).
- It falls back to the JSON as today, so a pasted old invite and the
  fixtures keep working. `switchboard-remote pair` (`main.swift:111`)
  and `BrowserView` (`:106`, `:883`) call `parse` and change only in
  the usage text (`main.swift:12`).

**4. Static paths.**

- `src/adapters/web.rs` holds the embedded files: `PAGE`, an array of
  `PageFile { path, content_type, body }` built with
  `include_str!("../../web/…")`, and `CSP`.
- The three paths:
  - `/`: `index.html`, `text/html; charset=utf-8`
  - `/app.js`: `text/javascript; charset=utf-8`
  - `/app.css`: `text/css; charset=utf-8`
- A query on `/` is ignored and never logged: `RemoteRequest::path`
  holds the path without its query (`src/adapters/remote.rs:362-363`).
- In `connection` (`src/adapters/remote.rs:770`), right after
  `read_request` and before the rate-window check (`:788`), a path
  outside `/api/v1/` is answered there:
  - both answers go through `respond_closing(w, status, content_type,
    extra_headers, body)`, a new private writer beside `respond`
    (`:530-552`) that writes `Connection: close`, so `RemoteReply` and
    its 30 literals are untouched and no reply claims keep-alive before
    a close;
  - a `GET` of a page file gets its bytes, its content type and the
    page headers below;
  - anything else gets 404, `application/json` and
    `error_body("not_found", "no such endpoint")`, with no page
    headers, and is logged as the frame logs it today
    (`src/app.rs:3762-3766`): `log::info!("remote: {method} {path}
    refused: not_found")`, method and path only, never the query, so
    §5's "Refusals are logged by op and kind" (`docs/remote-client.md:318`)
    still holds;
  - then `return Ok(())`.
- Headers on a page file:
  - `Content-Type` as above;
  - `Content-Security-Policy: default-src 'none'; script-src 'self';
    style-src 'self'; connect-src 'self'; img-src 'self'; base-uri
    'none'; form-action 'none'; frame-ancestors 'none'`;
  - `X-Content-Type-Options: nosniff`;
  - `Referrer-Policy: no-referrer`;
  - `Cache-Control: no-store`, so an updated host never runs an old
    script against a new API;
  - `Connection: close`.
- `index.html` has no inline script or style:
  `<link rel="stylesheet" href="/app.css">` and
  `<script src="/app.js" defer>`.
- `connect-src 'self'` covers the socket. CSP Level 3 §6.7.2.8, the
  `'self'` branch, matches a URL on the same host and port whose scheme
  is "https" or "wss", a change from Level 2 listed in §1.3
  (https://www.w3.org/TR/CSP3/). WebKit followed in bug 235873, "CSP:
  Improve compatibility of source matching", RESOLVED FIXED 2022-04-02
  in r292266 (https://bugs.webkit.org/show_bug.cgi?id=235873; bug
  201591 closed as its duplicate). The Safari release is not named, so
  an iOS whose WebKit predates April 2022 may refuse it. Fallback for
  that WebKit only: `read_request` keeps `Host` and `connect-src` adds
  `wss://<Host>` per response. Never a bare `wss:`, which would allow
  any host. The owner's run checks the console for a violation.
- Pool and rate: a page load is unauthenticated, so it counts against
  the pool of 4 like any stranger. `Connection: close` frees each slot
  as soon as its file is written. The files get no rate window: they
  cost one write of compile-time bytes on the connection's own thread,
  and the pool already bounds a stranger. `/info` keeps its 60 a minute,
  and the page calls it once per load.
- How many connections iOS Safari opens, and whether the warning's
  failed handshake holds a slot, are Unresolved. Fallback: if the
  owner's run shows loads dropped at accept, raise
  `MAX_UNAUTHENTICATED` (`:567`) from 4 to 8 and amend §4.1.
- §5's rule becomes: "Require pairing before anything but `/info`,
  `/pair` and the page's three files, which are the same bytes for
  everyone and carry no state."

**5. `GET /api/v1/device` (read scope).**

- A new arm in `serve_remote` after `authenticate` (`src/app.rs:3033`).
- It answers in the envelope with
  `{device_id, name, scope, server_id, server_name}`, where
  `authenticated` is true.
- The page reads it at load and after any `403 scope`, so a scope
  changed in the dialog shows on the device's next request.
- There is no `info` capability for it: the page ships with the host,
  and the Swift client does not use it yet.
- This rests on the For the owner line "Scope in the header".

## The UI

**Host dialog** (`src/ui/remote.rs`):

- The QR code and "Copy invite" carry the URL. The hover text becomes
  "The QR code's link: a phone's camera opens it; a client takes it
  pasted".
- Under "Paired devices", one new `meta_text` line: "A browser pairs
  from the QR code's link. Its first visit shows a certificate warning:
  accepting it is what trusts this Mac on that device, so accept it only
  on your own network, from a code this window shows. The page checks
  the host's key against the link, which catches a different host, not
  an impostor."

**The page** (`web/index.html`, `web/app.js`, `web/app.css`):

- Plain DOM. Text goes through `textContent`. There is no framework and
  no build step.
- Load:
  1. If the URL has a query, read it, then
     `history.replaceState(null, "", "/")` before anything else.
  2. `GET /api/v1/info`.
- With a code: if `key` or `id` differs from `/info`, the page says
  "This is not the host whose code was scanned. Nothing was sent."
  Otherwise it posts `/api/v1/pair {code, device_name, spki_sha256:
  key}`. The `device_name` is "<browser> on <device>" from
  `navigator.userAgent`, with an iPad that reports a Mac told apart by
  `navigator.maxTouchPoints > 1`. On success it stores
  `{server_id, spki_sha256, token, device_id}` in `localStorage`, every
  access in try/catch. `pairing_closed` and `wrong_code` are said in
  words.
- With a stored token:
  - `/info`'s key differs: "This host's identity changed. Pair again."
    The token is dropped and nothing is sent.
  - The `server_id` differs: "Another host answers at this address.
    Pair again."
  - Otherwise `GET device`. A 401 means "This device was revoked on the
    host. Pair again."
- With neither a code nor a token: "Not paired. On the Mac, open
  Settings → Remote devices…, allow devices on the local network, allow
  pairing, and scan the code with this device's camera."
- Header: `server_name`, the device's name and scope, and the live or
  polling state.
- List, §4.4's order:
  1. Open the socket and wait for `hello`.
  2. Fetch `projects`, then each project's `sessions`.
  3. Reconcile the buffered events by revision.
- Projects with a waiting session come first. Within a project,
  waiting sessions come first and are marked. Each row shows the name,
  kind, card word, `waiting_reason`, and, for the first 8 waiting
  Claude Code rows only, the excerpt from the first departure.
- `changed session` refetches that session, plus its `last-response`
  if it is one of those 8 rows. While polling, an excerpt is refetched
  only when that session's `last_stop_at_ms` moves. `changed sessions` and
  `changed projects` refetch the collection. `removed` drops the row.
  `resync_required` reconnects and refetches everything.
- Session view: `last-response` for `kind: claude`, `screen?lines=60`
  otherwise.
  - It sends `watch` for its one key, fetches after `watching`, and
    refetches on `changed` for that key and every 30 s (§4.4). It sends
    `unwatch` on leaving.
  - The page's watch-set cost is in the excerpt departure.
- Reply box: shown only under `control`. It posts
  `sessions/{id}/send {op, text}`. On a timeout or a network error it
  asks `ops/{op}` before offering a retry (§4.5). A 403 `scope`
  re-reads `device` and hides the box.
- A closed or failed socket, and a 401 on any `fetch`, are handled as
  Actions 1 says.
- All endpoint paths are in one `const API` block at the top of
  `app.js`, which the test reads.

## The tests

The page's logic has no JS harness in the tree, and the issue rules out
adding one (For the owner, "Page logic untested by CI"). The host side
is tested here, and the page is checked by hand at acceptance.

- **`tests/remote.rs`.** Add `http_with`, which sends extra headers and
  returns `(status, headers, body)`. `http` (`:245`) becomes a wrapper.
  - `the_page_is_served_by_the_connection_thread_with_its_headers`:
    `/`, `/?code=X&key=Y&id=Z`, `/app.js` and `/app.css` are requested
    through `http_with` with no `serve_pending`. A reply that arrives
    shows the frame was never asked.
    - Each answers 200 with `web::PAGE`'s bytes and content type, the
      exact `web::CSP`, `nosniff`, `no-referrer`, `no-store` and
      `Connection: close`.
    - `/favicon.ico` and `POST /` answer 404 `not_found` with
      `application/json` and `Connection: close`, and no CSP.
  - `the_page_references_only_what_the_host_serves`:
    - every `src=` and `href=` in `index.html` is a `PAGE` path, and no
      file contains `http:`, `https:` or `//` in a reference;
    - every `/api/v1/` path in `app.js`'s `API` block is requested with
      a paired token, with `${…}` after `sessions/` and `projects/`
      replaced by `seed`'s ids. None answers 404 `not_found`, and the
      events path upgrades.
  - `the_page_writes_no_markup_from_data`: `app.js` has no `innerHTML`,
    `outerHTML`, `insertAdjacentHTML`, `document.write`, `eval(` or
    `new Function`.
  - `a_browser_pairs_and_subscribes_with_its_token_as_a_subprotocol`,
    end to end:
    1. `GET /`, then `info`, then `pair` with
       `device_name: "Safari on iPhone"`; the device is listed under
       that name.
    2. `GET device` gives the name and scope.
    3. Subscribe through a new `subscribe_with_protocol`, which sends
       `Sec-WebSocket-Protocol: switchboard.v1, bearer.<token>` and no
       `Authorization`. The 101 carries `switchboard.v1`, which
       tungstenite checks, and `hello` arrives.
    4. A session that turns waiting sends its `changed`.
    5. A `send` under `control` reaches the fake host.
    6. Revoke closes the socket, and the `GET device` the page makes
       after a close is 401, which is what the page shows as revoked.
  - `a_subprotocol_token_needs_the_switchboard_name`: `bearer.<token>`
    alone is 401.
- **`src/adapters/remote.rs` unit test**, beside
  `a_request_reads_with_its_query_bearer_and_body` (`:1045`):
  `a_request_takes_its_bearer_from_the_websocket_protocol`. A header
  bearer wins. Protocols without `switchboard.v1` leave `bearer` empty.
- **`src/core/tests.rs`:** `invite_url_carries_the_code_key_and_id`,
  which checks the exact string for a fixed identity, code, IPv4 and
  port.
- **`tests/ui.rs`:** `remote_devices_dialog_opens_pairing_and_revokes_a_device`
  (`:7424`) parses the copied invite as a URL instead of JSON
  (`:7486-7492`): `https`, `code`, `key` equal to the identity's key,
  `id`. It also finds the browser trust line by label.
- **Swift** (`client/Tests/SwitchboardClientCoreTests/PairingTests.swift`):
  a URL invite parses to one address, and the JSON still parses. Run
  with `(cd client && swift test)`. It is not in CI or the hook.
- Acceptance by hand, recorded in the design status bullet:
  1. Run an instance on a test data dir with script lines
     `remote-listen lan <port>` and `pairing <code>`.
  2. Scan with an iPhone and accept the warning.
  3. Note whether the page is live or polling, and whether the console
     shows a CSP violation.
  4. Start a session waiting and watch it move to the top.
  5. Send a reply.
  6. Revoke the phone and see the page say so.

## Docs

- **`docs/remote-client.md`:**
  - §4.1: the connection thread answers the page and every non-API
    path, with `Connection: close` and no rate window, and logs a
    non-API refusal by method and path, never the query.
  - §4.3: add rows for the three page paths and `GET /api/v1/device`.
  - §5:
    - the rule at `:306`;
    - the QR as a URL at `:309`;
    - the relay sentence at `:309`, which holds only for a client that
      pins;
    - the click-through as a browser's trust step, and the single-use
      code as the guarantee against a code left in history;
    - the token at `:312`: a bearer header, or a browser's
      `switchboard.v1` / `bearer.<token>` subprotocol pair on the
      events socket, never a URL;
    - a browser keeps its token in `localStorage` and pins nothing, so
      three sentences say "a native client": `:309`'s last ("after
      pairing, every connection verifies the pin before the token is
      presented"), `:313` ("The client stores the token and the pinned
      key hash in its Keychain") and `:317` ("The client verifies the
      pin on every connection …"); each gains that a browser instead
      relies on its certificate exception and checks `/info`'s key
      against what it stored.
- **`docs/design.md`:** a bullet "**The page (#230).**" before "Known
  gaps at the first merge" (`:3573`). It says what is built, the poll
  fallback, the owner's device result, the page's watch-set cost (at
  most 9 keys: 8 excerpt reads and the open view's watch), and the
  gaps: excerpts only for the first 8 waiting Claude Code sessions, no
  tickets, no conversation beyond the
  last response.
- **`README.md`:**
  - Layout lines for `src/adapters/web.rs` and `web/`;
  - the `remote.rs` listener line (`:383`) gains "the page's files and
    any non-API path answered on the connection thread";
  - the `SWITCHBOARD_REMOTE_PORT` dev-aid text (`:276-279`): "Copy
    invite" copies the link, which a browser opens and which
    `switchboard-remote pair "$(pbpaste)"` takes.
- **`client/README.md`:** the `pair` usage line.
- **`CHANGELOG.md`**, under Unreleased: "A browser on the local network
  can pair from the pairing window's QR code and list sessions, read
  the last response or screen, and send a reply."

No spike directory: the one risky mechanism is browser behaviour, which
only the owner's device can answer. The page ships with its own
fallback either way, and the design status records the run.

## Commit

`The remote port serves a browser page that pairs from the QR code's link`

Contract additions (none in `src/core/model.rs`, `src/core/action.rs` or
`src/ports/`):

- `adapters::remote::RemoteRequest::subprotocol: Option<String>`
  (adapter type, outside the contract)
- `adapters::web` (new module): `PageFile`, `PAGE`, `CSP`
- `core::remote::invite_url`
- Wire: `GET /`, `GET /app.js`, `GET /app.css` before pairing;
  `GET /api/v1/device` (read); the `switchboard.v1` and
  `bearer.<token>` WebSocket subprotocols
- Swift: `PairingInvite.parse` accepts the URL form
