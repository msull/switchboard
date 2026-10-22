# A web control surface on the remote port (#230)

A page the host serves on its own remote port, embedded in the binary,
that a browser on the local network opens from the pairing window's QR
code. It lists the sessions with the waiting ones first, shows a
session's last response or screen, and sends a reply. It speaks the
protocol the headset speaks: REST for reads and commands, the events
socket for live changes, replies applied by revision. The host changes
are small. The adapter serves three static files off the frame. The
token can ride the WebSocket's subprotocol header. `/info` names the
asking device. The QR carries a URL.

## Where this departs from the issue

- **"Each with its kind, state and the one-line excerpt the card
  shows."** The excerpt is not on the wire. `SessionView`
  (`control/src/lib.rs:637-688`) has no `last_message` or caption
  field, though `docs/remote-client.md` §4.3 (line 207) says it does.
  The Mac's card excerpt comes from the transcript or the pane in
  `refresh_captions` (`src/app.rs:1093-1096`), and only for cards on
  screen. The page's row therefore shows what the headset's row shows
  (`client/Sources/SwitchboardUI/BrowserView.swift:741-746`): the
  name, the kind, the card word, and `waiting_reason` when there is
  one. An excerpt for every listed session would need a new read model
  fed by the content worker, and this cut does not add one. The plan
  also corrects §4.3's sentence.
- **"The device's scope … in the header."** Nothing today tells a
  device its current scope. The pair reply carries the scope at pairing
  time (`src/app.rs:3660-3665`). A change made later in the dialog
  (`SetDeviceScope`, `src/core/remote.rs:93-100`) only shows up as a
  403 `scope` on the next send. The plan adds one field: `/info`
  answers `device: {id, name, scope}` when the request carries a token
  the host knows. That way the page can show the scope, and hide or
  show the reply box, on every load and reconnect.
- **"A name from the user agent."** The host never sees the
  `User-Agent` header. `read_request` keeps only `content-length`,
  `authorization`, `upgrade` and `sec-websocket-key`
  (`src/adapters/remote.rs:481-494`). The page builds the name from
  `navigator.userAgent` ("Safari on iPhone") and sends it as
  `device_name`, which `/pair` already takes and trims to 64 characters
  (`src/app.rs:3650-3655`, `src/core/remote.rs:169`). The host is
  unchanged.
- **"The QR becomes `https://<address>:<port>/…`."** The dialog
  computes the address with `remote::local_address()` even when the
  port is bound to loopback (`src/ui/remote.rs:156-159`). In that case
  the invite names an address nothing answers on. The URL uses that
  address while the port is on the local network and `127.0.0.1`
  otherwise, so a browser on the Mac itself can pair. That is also the
  dev path.

## The decisions

1. **Browser trust: the click-through is the trust step.** The
   certificate (`make_identity`, `src/adapters/remote.rs:95-127`) has
   no subject alternative names (line 102) and is valid until the year
   4096 (lines 90-92). A browser can trust a downloaded copy only if it
   is reissued:
   - naming the Mac's current address, which DHCP moves;
   - with a validity Apple's platforms accept for a TLS server
     certificate.

   On a phone it would then also need a profile install and a "full
   trust" switch, which trusts it for every name the phone visits.
   That is more steps and a wider trust than the problem needs. So the
   owner accepts the warning on the first visit, and the browser then
   holds an exception for that certificate at that address.

   What this buys:
   - **The pairing request is checked.** The genuine page refuses to
     post the code unless `/info` presents the key in the URL.
   - **The host checks the key too.** It refuses a `/pair` whose
     `spki_sha256` is not its own (`src/app.rs:3636-3645`).
   - **A different certificate shows a new warning** once the
     exception is in place.
   - **DNS rebinding fails**, because a rebound name meets this
     certificate and fails TLS.

   What it does not buy:
   - **An impostor's page is not checked.** If something else answers
     the address at the first visit and the owner accepts its warning,
     it serves its own page. That page skips the key check and can
     relay the code. Browser pairing is trust-on-first-use on the local
     network, weaker than the native pin.
   - **Accepting a second warning hands over the token.** An impostor
     at the same address and port is the same origin, so it can read
     the stored token. An exception does not last forever, so a second
     warning is not by itself a sign of attack.

   The dialog says this in one sentence (see the UI section). §5's rule
   stands with one change: before pairing, a bind on the local network
   serves `/api/v1/info`, `/api/v1/pair`, and the page's static files
   (which hold nothing of the host's), and nothing else.
2. **The token reaches the WebSocket as a `Sec-WebSocket-Protocol`
   value, and lives in `localStorage`.** The page opens
   `new WebSocket(url, ["switchboard.v1", "switchboard.token.<token>"])`.
   - **Host side.** `read_request` takes the token from the
     `switchboard.token.` entry, but only on an upgrade request and
     only when no `Authorization` header is present. The 101 echoes
     `Sec-WebSocket-Protocol: switchboard.v1` and never the token
     entry. `fetch` sends `Authorization: Bearer` as the native clients
     do, and the bearer header stays the native path.
   - **Why not a cookie.**
     - **Cookies are not isolated by port.** A browser sends a cookie
       for an address to every port on that address. A dev server an
       agent starts on the same Mac, opened from the phone, would
       receive the token. `localStorage` is per origin, port included.
     - **Header credentials leave nothing ambient.** No request from
       another origin carries the token, so there is no cross-site
       request forgery to defend against and no cross-site WebSocket
       hijacking to check `Origin` for. The host sends no CORS headers,
       so another origin cannot read a reply.
   - **The cost.** Any script on the origin can read `localStorage`.
     The origin serves only the three embedded files, under a CSP that
     admits no inline script and nothing external. The page writes host
     text only through `textContent`, and a test fails if `app.js` uses
     an HTML-parsing sink (see Tests).
   - **The spec's rule holds.** "Never in a URL" still holds: a
     subprotocol is a header.
3. **One QR: the URL.** The host makes only the URL, and "Copy
   invite" becomes "Copy link" with the same text. `PairingInvite.parse`
   (`client/Sources/SwitchboardProtocol/Info.swift:113-116`) reads a URL
   first and falls back to the JSON. The JSON parse stays on the client
   only, for a host older than this change and for the
   `SWITCHBOARD_INVITE` dev aid's saved lines
   (`client/Apps/SwitchboardVision/SwitchboardVisionApp.swift:56-61`).
   `addresses` becomes the URL's `host:port`, `spki_sha256` becomes
   `key`, and `server_id` becomes `id`. The CLI's one-argument
   `pair` (`client/Sources/switchboard-remote/main.swift:110-111`) and
   the headset's paste (`BrowserView.swift:106` and `:883`) go through
   `parse`, so they take the link without further change.
4. **Static paths.** Exactly these, `GET` only, answered on the
   connection thread from `&'static str`s, never reaching the frame:

   | Path | `Content-Type` | Body |
   |---|---|---|
   | `/` | `text/html; charset=utf-8` | `web/index.html` |
   | `/app.js` | `text/javascript; charset=utf-8` | `web/app.js` |
   | `/app.css` | `text/css; charset=utf-8` | `web/app.css` |
   | `/favicon.ico` | none | `204`, empty (browsers ask for it unprompted) |

   The query string is ignored and never logged. The adapter already
   splits it off the path (`src/adapters/remote.rs:495`) and logs
   nothing on this path. Any other method or path goes to the frame as
   today and is refused 404 there (`src/app.rs:3023-3025`).

   Every static reply carries:

   - `Content-Security-Policy: default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'`
   - `Cache-Control: no-store`, so a new version is never served from
     an old cache
   - `X-Content-Type-Options: nosniff`
   - `Referrer-Policy: no-referrer`

   Every JSON reply gains `Cache-Control: no-store` and
   `X-Content-Type-Options: nosniff`, so a browser keeps no host body
   on disk, matching §7's client rule.

   **Pools and rate windows.**
   - **Pool.** A page load counts against the unauthenticated pool of
     4 (`MAX_UNAUTHENTICATED`, `src/adapters/remote.rs:567`). Static
     replies carry no authentication, and a kept connection moves to
     the paired pool on its first authenticated API request
     (lines 835-849). Three files keep a cold load within the pool.
   - **Rate window.** Static paths get a third window,
     `PAGE_PER_MINUTE = 120` per listener, beside `/pair` (6) and
     `/info` (60) (lines 582-584, `Limits::limited` at 631-638).

   **Opened without a code or a token,** the page reads `/info` and
   shows the host's name, whether pairing is open, and one line: "Not
   paired. On the Mac, open Remote devices, allow pairing, and scan the
   code with this device."
5. **Where the page lives and how it is tested.**
   - **Files.** `web/index.html`, `web/app.js` and `web/app.css` at the
     repository root. They are embedded with `include_str!` in a
     `PAGE_FILES` table in `src/adapters/remote.rs`, with no build
     step.
   - **No browser in the tests.** The pairing flow is tested end to end
     with the test's own TLS client, as the CLI pairs.
   - **One new fixture.** `client/Fixtures/invite.txt` holds the link,
     written from the same core function the dialog uses. No view
     changes, so no JSON fixture changes.
   - **Spike 23.** A new mechanism needs one before it is built on:
     `spikes/23-web-page/README.md` records a browser matrix measured
     against the real port as soon as the static paths and the
     subprotocol read exist, before the page grows its views. The
     matrix covers iPhone Safari, macOS Safari and Chrome. For each:
     - the warning click-through;
     - `fetch` and a `wss` subscription with the subprotocol after it;
     - whether `connect-src 'self'` admits the `wss`;
     - whether `localStorage` survives a browser restart;
     - how long the exception lasts.

     If a browser refuses the socket after the click-through, the page
     still works on its 30-second refetch of the visible view (below),
     and the README says which browser.
6. **The device record.** A browser is a `PairedDevice` like any other
   (`src/ports/remote.rs:94-105`). It has the page's name, the scope
   chosen in the pairing window, and a row in the paired-devices dialog
   with its scope and Revoke. Revoke closes its socket as it closes any
   device's (`src/app.rs:486-497`). The pairing is per origin: if the
   Mac's address changes, the browser has no token at the new address
   and pairs again. The dialog says so, and the old row stays until it
   is revoked.

## The record

No change. `devices.json` holds `PairedDevice` as it is, and
`identity.json` holds `Identity` as it is. `SCHEMA_VERSION` (16,
`src/core/model.rs:12`), `VIEWS_SCHEMA_VERSION` (7,
`src/core/model.rs:612`) and `IDENTITY_SCHEMA_VERSION` (1,
`src/ports/remote.rs:17`) stay where they are. The token sits on the
device in `localStorage` under the origin, as
`{token, spki_sha256, server_id, device_id}`.

## The signals

None new. The page consumes the existing events:

- `hello`, `changed`, `removed`, `watching` and `resync_required` from
  §4.4.
- `watch` and `unwatch` frames (`subscriber_frame`,
  `src/app.rs:1704-1742`), so a content key emits `changed` while the
  page shows it.

## The actions and effects

No new `AppAction` or `Effect`. The host changes are these:

- **Core.** Add `AppCore::pairing_url(&self, address: &str) ->
  Option<String>` in `src/core/remote.rs`. It is a pure function of the
  identity and the open window:
  `https://<address>/?code=<code>&key=<spki_sha256>&id=<server_id>`.
  None of the three values needs escaping: a Crockford code, lowercase
  hex and a UUID. `address` is a `SocketAddr` as text, so IPv6 comes
  bracketed.
- **Adapter** (`src/adapters/remote.rs`):
  - **`PAGE_FILES`** holds `(path, content_type, body)`, with
    `include_str!("../../web/…")`.
  - **`connection`** (line 770) checks for a page path before the rate
    check at line 788. A page path is answered by a new `respond_page`
    with the headers above, under the `PAGE_PER_MINUTE` window, and the
    loop continues.
  - **`respond`** (line 530) adds the two JSON headers.
  - **`RemoteRequest`** gains `subprotocol: bool`: the upgrade offered
    `switchboard.v1`.
  - **`read_request`** (line 444) collects every
    `sec-websocket-protocol` value, comma-split. On an upgrade with no
    `Authorization`, it takes the `switchboard.token.` entry as
    `bearer`.
  - **`respond_upgrade`** (line 519) takes the flag and adds
    `Sec-WebSocket-Protocol: switchboard.v1` when it is set.
  - **Unchanged.** `authenticate` (`src/app.rs:3127`) reads `bearer`
    as before, so the frame is not touched for this.
- **App** (`src/app.rs`). `remote_info` (line 3568) takes the request.
  When `request.bearer` hashes to a known device
  (`core.device_by_token`), it adds `"device": {id, name, scope}` and
  answers `authenticated: true`, which moves a browser's connection to
  the paired pool. It does not dispatch `DeviceSeen`. Any other bearer
  gets the reply as today, with nothing that says the token was
  refused. The call at line 3028 passes the request.
- **Swift.** `PairingInvite.parse` reads a `https://` URL through
  `URLComponents` (host and port into `addresses`, `key`, `code`, `id`)
  and otherwise the JSON. A link missing any of the three parameters
  is an error. The CLI usage line (`main.swift:12`) reads
  `pair '<invite link or json>'`.

## The UI

**The dialog** (`src/ui/remote.rs`):

- **The link.** The JSON payload at lines 160-166 becomes
  `cx.core.pairing_url(&address)`. `address` is `local_address()` with
  the bound port while `identity.listen.lan` is set, and
  `127.0.0.1:<port>` otherwise. The QR (line 172) encodes it, and the
  "Address" line shows the link's host.
- **Copy link.** "Copy invite" (lines 201-206) becomes "Copy link",
  hovered "The link in the QR code, for a browser or a client".
- **The hint** at line 194 becomes "Scan with a phone's camera, or type
  the key and the code on the headset. Grants {scope} scope. Open for
  five minutes or three wrong codes."
- **The browser sentence.** Under the redaction sentence (lines 70-76)
  it reads: "A browser trusts this Mac by the certificate warning it
  shows on the first visit. Accept it only right after scanning, on
  your own network: the page then checks this Mac's key before it sends
  the code. A browser's pairing belongs to the address in the link;
  pair again if this Mac's address changes."

**The page** (`web/`, no framework, no dependency):

- **On load,** `history.replaceState(null, "", "/")` runs first,
  before any request, after reading `code`, `key` and `id` out of
  `location.search`.
- **Then `GET /api/v1/info`,** with `Authorization` when a token is
  stored.
  - **Code in the link.** If `key` is not `info.spki_sha256`: "This is
    not the Mac whose code you scanned. Nothing was sent." Otherwise
    `POST /api/v1/pair {code, device_name, spki_sha256: key}`, store
    the reply, and go on. `wrong_code`, `pairing_closed` and
    `rate_limited` are each one plain line.
  - **Stored token.** If the stored `spki_sha256` differs from
    `info.spki_sha256`: "This Mac's identity changed; pair again."
    Drop the token and send nothing.
  - **Neither:** the not-paired line of decision 4.
- **Connected:**
  1. Open the socket with the subprotocol and wait for `hello`.
  2. Fetch `projects`, then each project's `sessions`, holding one
     revision per resource key and applying a reply only at or past
     it (§4.2).
  3. Apply the buffered events as §4.4 says. `changed {sessions, p}`
     refetches that project's list, `changed {session, id}` the one
     session, `changed {projects}` the projects, and `removed` drops
     the member. A new `epoch`, `resync_required` or a close refetches
     everything after a reconnect with backoff (1 s doubling to 30 s).
- **The list.** Sessions are grouped by project, projects holding a
  waiting session first. Within a project the waiting ones come first,
  marked with an accent bar and "waiting on you". Each row shows the
  name, kind, card word and `waiting_reason`.
- **The header** shows `info.server_name`, the device's scope from
  `info.device`, and "live" or "reconnecting" by the socket's state.
- **The session view,** reached by tapping a row:
  - **Claude Code session:** `watch {conversation, id}`. On `watching`,
    `GET sessions/{id}/last-response`, refetched on
    `changed {conversation, id}`.
  - **Any other kind:** `watch {screen, id}` and
    `GET sessions/{id}/screen?lines=80` in a `<pre>`, refetched on
    `changed {screen, id}`. "Not running" is shown when the screen is
    refused.
  - **Leaving** sends `unwatch`.
  - **Refetch.** The visible view refetches every 30 s regardless
    (§4.4), which also covers a socket that never opened.
- **The reply box.** Shown only when `info.device.scope` is `control`.
  - **Refusals.** It is disabled with `prompt_refusal` beside it when
    the session view carries one. When `trust_question` is set it says
    "Claude Code is asking to trust this folder; answer it on the Mac."
  - **Send.** It posts `sessions/{id}/send {op: crypto.randomUUID(),
    text}`, refusing over 16 KiB by `TextEncoder` length.
  - **A timeout** is followed by `GET ops/{op}` per §4.5's rules:
    `done` clears the box, `interrupted` says the outcome is unknown,
    and `unknown` resubmits once with the same op.
  - **A 403 `scope`** re-reads `/info` and hides the box.
- **Revoked.** Any 401 with the stored key still matching says "This
  device's pairing was revoked on the Mac. Scan a new code to pair
  again." and drops the token. A socket closed by the host triggers an
  immediate `/info` and list read, so a revocation is said within a
  second.
- **Writing.** Host text goes in through `textContent` only, and styles
  through classes (the CSP refuses a `style` attribute).

## The tests

- **Core** (`src/core/tests.rs`, the `remote` module at line 11761).
  `pairing_url` is `None` with no window or no identity. With both, it
  is exactly the expected link for a seeded identity, code and
  `192.168.1.9:43187`, and an IPv6 address comes bracketed.
- **Adapter** (`src/adapters/remote.rs` tests):
  - `read_request` takes the bearer from `switchboard.token.` on an
    upgrade, across one header line or two.
  - It ignores that entry on a plain request.
  - `Authorization` wins when both are present, and `subprotocol` is
    set only when `switchboard.v1` was offered.
  - `respond_upgrade` echoes the protocol and never the token.
- **`tests/remote.rs`.** Add a `fetch` helper that returns the headers,
  and these tests:
  - `the_page_is_served_before_pairing_with_its_headers`.
    - **Bodies and headers.** `/`, `/app.js` and `/app.css` answer 200
      with the table's `Content-Type`, the exact CSP string,
      `no-store`, `nosniff`, `no-referrer`, and the embedded bytes.
    - **Other paths.** `/?code=X&key=Y` answers the same body.
      `/favicon.ico` answers 204. `/index.html` and `POST /` answer 404
      from the frame.
    - **Off the frame.** All of it is requested without calling
      `serve_pending`, which shows that no static reply needs the
      frame.
    - **JSON headers.** A JSON reply carries `no-store`.
  - `the_page_references_only_paths_the_host_serves`.
    - **Static references.** Every `src`/`href` in `index.html` is in
      `PAGE_FILES`.
    - **API references.** Every entry of the `API` table at the top of
      `app.js` (one path template per line, between marker comments)
      is filled with seeded ids and requested with a token, and none
      answers 404 `not available on this port`.
    - **No external resources.** None of the three files holds
      `http:`, `https:` or `//` outside the `API` table's comments.
    - **No HTML-parsing sinks.** `app.js` holds none of `innerHTML`,
      `outerHTML`, `insertAdjacentHTML`, `document.write`, `eval(` or
      `new Function`.
  - `a_browser_pairs_from_the_link_and_subscribes_with_the_token_as_a_subprotocol`.
    1. Open a window, take `pairing_url`, and parse its query.
       `/info`'s key equals `key`.
    2. `/pair` with `code`, `key` and `Safari on iPhone` gives a token
       and the window's scope.
    3. `/info` with the token carries `device` with that name and
       scope, and without the token it does not.
    4. A `GET /waiting` with the bearer answers.
    5. A subscription with `Sec-WebSocket-Protocol: switchboard.v1,
       switchboard.token.<t>` and no `Authorization` gets a 101 echoing
       `switchboard.v1` (`tungstenite`'s client checks this) and
       `hello`. The same with an unknown token is refused.
    6. Revoke closes that socket, and the next `GET` is 401.
  - `a_page_load_counts_against_the_strangers_pool_and_its_own_window`.
    - Four concurrent connections each load `/`, `/app.js` and
      `/app.css` and succeed.
    - On one kept connection, request 121 answers 429 `rate_limited`
      while `/info` still answers.
  - **Fixtures.** `write_fixtures` (line 1587) writes
    `client/Fixtures/invite.txt` from `pairing_url`, and
    `the_client_fixtures_decode_on_both_sides` (line 1829) checks that
    it parses as the link.
- **`tests/ui.rs`.**
  `remote_devices_dialog_opens_pairing_and_revokes_a_device`
  (line 7424) clicks "Copy link" and asserts the copied text is an
  `https://127.0.0.1:<port>/?code=<code>&key=<spki>&id=<server_id>`
  link, since the harness's identity has `lan: false`. It also finds
  the browser sentence by label.
- **Swift** (`client/Tests/SwitchboardProtocolTests/FixturesTests.swift`).
  `testInviteParsesTheLink` reads `Fixtures/invite.txt`, and a second
  case checks that a link without `key` is an error.
  `testInviteParsesTheQRText` stays, for the JSON. The count in
  `README.md:18` and `client/README.md:11` moves to match.
- **By hand, the acceptance:**
  1. Scan with a phone and accept the warning: the sessions show.
  2. Make a session wait on the Mac: it moves to the top without a
     reload.
  3. Send from the phone: it reaches the pane.
  4. Revoke the phone: the page says so.

  This is recorded in `spikes/23-web-page/README.md` with the browser
  matrix of decision 5, with screenshots of the page on the phone, not
  of the Mac's window.

## Docs in the same commit

- **`docs/remote-client.md`:**
  - §4.1: the page window, and that the page is served on the
    connection thread.
  - §4.3: rows for `/`, `/app.js`, `/app.css` and `/favicon.ico`
    (scope none). `info`'s `device` field. The excerpt sentence on
    line 207 corrected: no excerpt is on the wire.
  - §5: the QR is the link. The browser's trust step and what it does
    not buy. The token as a subprotocol and its storage. The rule
    "before pairing: `/info`, `/pair` and the page".
  - §11: the new cases.
  - §13: a "Web page" row.
- **`docs/design.md`.** In the Remote client section
  (line 3107), a bullet "**The web page (2026-10-22).**" before
  "Known gaps" (line 3573), with the known limits: no excerpt, a
  pairing per address, and the spike's browser notes. The trust
  boundary's redacted-text sentence (lines 250-254) gains "or the page
  a paired browser reads".
- **`README.md`:**
  - **Layout.** A `web/` line, and the adapter (line 383),
    `ui/remote.rs` (line 423) and `tests/remote.rs` (line 437) lines
    extended.
  - **Dev aids.** At lines 276-279, "Copy link" puts the link on the
    clipboard, and `open "$(pbpaste)"` pairs the Mac's own browser on
    loopback.
- **`client/README.md`** lines 234-235: the link, or the JSON.
- **`CHANGELOG.md`** under Unreleased: "A browser on the local
  network pairs by scanning the Remote devices QR code and shows the
  sessions, waiting ones first, a session's last response or screen,
  and sends a reply."

## Commit

`A browser control page on the remote port, paired from a link in the QR code`

Contract additions:

- `model.rs`, `action.rs`, `ports/`: none.
- `AppCore::pairing_url(&self, address: &str) -> Option<String>`
  (`src/core/remote.rs`).
- `adapters::remote::PAGE_FILES` and `RemoteRequest::subprotocol`.
- Protocol:
  - `GET /`, `/app.js`, `/app.css` and `/favicon.ico` before pairing.
  - `/info`'s `device: {id, name, scope}` for a known bearer.
  - The token accepted as the `switchboard.token.<token>` WebSocket
    subprotocol beside `switchboard.v1`.
  - `Cache-Control: no-store` on every reply.
  - The pairing QR and "Copy link" carry
    `https://<address>/?code=&key=&id=`, and `PairingInvite.parse`
    accepts it.
