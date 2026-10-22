# Plan: a web control surface on the remote port (#230)

Checked against the worktree at 7fe8949. Facts are cited as "facts" with the
section they sit under (Settled, Unresolved, For the owner).

## Where this departs from the issue

- **"The one-line excerpt the card shows."** It is not on the wire.
  `SessionView` (`control/src/lib.rs:637`) has `card`, `waiting_reason`,
  `last_stop_at_ms` and no excerpt or `last_message` (facts, Settled: the
  grep is clean in `control/src/lib.rs`, `src/app.rs` and `client/Sources`).
  The list shows kind, card word and waiting reason; the last response is in
  the session view. The issue's "no new read model unless the page cannot do
  without one" holds. An excerpt field on `SessionView` is the owner's call
  (facts, For the owner 9) and is not in this cut. The default does not
  depend on the answer: fetching `last-response` per listed session would put
  one watch per session into a set capped at 128 whose eviction takes the
  oldest `last_read` across every subscriber (`src/app.rs:1789-1795`, facts,
  Settled), and its effect on the headset is unmeasured (Unresolved).
- **"The device's scope in the header."** No endpoint returns a device's
  scope after `/pair` (`remote_info`, `src/app.rs:3568`, is unauthenticated).
  The plan adds `GET /api/v1/device`. It is additive and is the owner's call
  (facts, For the owner 8). If the owner strikes it, the page shows the scope
  from the `/pair` reply and hides the reply box on the first 403 a command
  gets (the four `Scope::Control` refusals in `src/app.rs`, e.g. `:3166`).
- **"Revoked" said plainly.** `authenticate` (`src/app.rs:3127`) answers
  `unauthorized` for both a missing token and an unknown one, so the code
  cannot say revoked. The plan adds no code. The page keeps a non-secret
  marker `{server_id, spki_sha256, device_id}` beside the token. A 401 with
  a token held, and `/info` still showing the marker's key, reads "this
  device was removed on the host". A different key for the same `server_id`
  reads "identity changed, pair again" and sends nothing, the §5 rule.
- **"Any path outside `/api/v1/` is refused."** True (`src/app.rs:3023`),
  but only after the request has been queued to the frame. The static arm and
  the refusal move to the connection thread (below), which is nearer
  §4.1's "the frame does core state only".
- **The key check in the URL.** `docs/remote-client.md:309` says a relayed
  pairing "fails even if the code reached it" because the host refuses a body
  whose `spki_sha256` is not its own. For a browser that does not hold: a
  page cannot read the certificate it is served over, so under a relay the
  impostor's page posts the host's real key, which `/info` also reports. The
  page's check catches a genuine page aimed at the wrong genuine host (a
  second installation at that address, a rotated key). It does not catch a
  relay. §5 gets a sentence saying so, and the paired-devices dialog gets the
  trust wording below.
- **The token never in a URL** is `docs/remote-client.md:312`, the project's
  own rule, not a web standard (facts, Settled). It holds: the code is in the
  URL, the token never is.
- **Certificate.** The issue treats the self-signed certificate as only a
  warning to click through. `make_identity` (`src/adapters/remote.rs:95`)
  makes one with no subjectAltName, a CN, and `not_after` in the year 4096.
  Apple requires a SAN, serverAuth and at most 825 days for certificates
  issued after July 2019, and says nothing about user-trusted ones; Chrome
  intends a SAN requirement (facts, Settled). Whether Safari allows the
  click-through on such a certificate is Unresolved. So the certificate
  changes (below) so the answer does not matter.

## The record

- **No `model.rs` change and no `SCHEMA_VERSION` bump.** A browser is a
  `PairedDevice` (`src/ports/remote.rs:96`) in `devices.json`, a bare
  `Vec` with no version. The name is the `device_name` the page sends
  (`remote_pair`, `src/app.rs:3606`), cut to 64 characters by `new_device`
  (`src/core/remote.rs:166`). The page builds "Safari on iPhone" from
  `navigator.userAgent`. No `kind` field: the name is enough (decision 6).
- **`identity.json` changes.** `Identity` (`src/ports/remote.rs:26`) gains
  `#[serde(default)] pub certificate_addresses: Vec<String>`: the IP
  addresses the stored certificate carries as SANs. A file without it reads
  as empty, which triggers a reissue at the next launch. This is a serialized
  type, so `IDENTITY_SCHEMA_VERSION` (`src/ports/remote.rs:17`) goes
  **from the value there at implementation time, plus one**; the value is
  currently written (`src/adapters/remote.rs:118`) and read by nothing, so the
  `serde(default)` is the whole migration; the reissue rewrites the file at
  the new version.
- **Reissue over the stored key.** A new `remote::reissue_certificate(identity,
  key_pem, addresses)` calls `KeyPair::from_pem` (`src/adapters/remote.rs:139`
  shows the call) and the same `params.self_signed(&key)` as `make_identity`.
  `spki_sha256` is unchanged, so every native pin and every device record
  holds (§5, "What the client pins"). The certificate carries: IP SANs for
  the route-chosen `local_address()` (`:962`), `127.0.0.1` and `::1`; DNS SAN
  `localhost`; EKU `serverAuth`; `not_before` now and `not_after` 800 days
  on, within Apple's 825 (facts, Settled). Reissue runs (a) at launch in the
  identity-load path (`src/app.rs:2799-2850`) when `certificate_addresses`
  lacks the current address or fewer than 60 days remain, and (b) when the
  pairing window opens, because the phone must reach the address the QR
  names. Both write through `save_identity` (the existing atomic path) and
  (b) rebinds with `bind_remote` (`src/app.rs:2946`) so the new certificate
  is served. A read-only instance does not reissue. `make_identity` also
  fills the new fields so a fresh identity is already compliant.
- **On the device.** One `localStorage` entry per origin:
  `{server_id, spki_sha256, device_id, name, scope, token}`. Decision 2 below
  says why this and not a cookie.
- The pairing window is not durable and stays so. The page itself is static
  bytes compiled in.

### The two decisions that live here

**Decision 1, trust in the certificate.** The click-through is the trust
step, and the paired-devices dialog says so in plain words: "A browser
cannot check this Mac's key itself. Accept the certificate warning only
after confirming the Key line below matches the one the page shows; anyone
who can answer on this network can show a page that looks the same." The
page shows the key's short form (the same `KeyFingerprint.short` form as
the dialog) before it asks to pair, so the owner can compare by eye. Why
not a download as the primary: a certificate fetched over the untrusted
connection proves nothing an impostor could not also serve, so it adds no
trust; it adds only reach for a browser that refuses the click-through. That
reach is the fallback, built behind the default: the host also serves
`GET /switchboard.cer` (the certificate's DER, `application/x-x509-ca-cert`,
public data) and the page's unpaired state links it. The SAN-bearing
certificate above is what makes either path work (facts, For the owner 1,
Unresolved on Safari and on user-trusted limits). Whether iOS Safari
accepts the click-through, and whether it carries over to `fetch` and
`wss://`, are device checks; the acceptance run below records them, and
nothing in the build depends on them. §5's rule becomes: before pairing
the host serves `/info`, `/pair`, the page's static files and the
certificate, and nothing else.

**Decision 2, how the token reaches the WebSocket.** `fetch` can send the
bearer header the host already reads, so only the WebSocket needs a
carrier. Default: the `Sec-WebSocket-Protocol` carrier. The page opens
`new WebSocket(url, ["switchboard.v1", "bearer." + token])`. The token is
43 base64url characters, all legal token characters (RFC 6455 §4.1, facts,
Settled). The host picks the token out of the `bearer.` entry when there is
no `Authorization` header, and **echoes `switchboard.v1` only**, which is
one of the offered values, as RFC 6455 §4.2.2 requires and without ever
reflecting the token (facts, Settled). A native client offers no
subprotocol and gets none. The cookie route is not taken: it rests on
whether Safari sends `SameSite=Strict` cookies on a same-origin WebSocket
handshake (facts, Unresolved) and on cookies not isolating by port (RFC
6265 §8.5, facts, Settled), so a token cookie would reach every other HTTPS
service on the Mac. The cost of the default: the token sits in
`localStorage`, readable by any script of the origin (facts, Settled). The
CSP below allows only the page's own file, and no inline script, so that is
the page's script alone. A cookie is an enhancement only after a device
check clears it, and is not in this cut.

## The signals

- **No new event kind and no new revision source.** The page uses the
  existing socket: `hello`, `changed {kind,id}`, `removed`, `watching`,
  `resync_required` (§4.4), and sends `watch`/`unwatch` for the open session
  only. Reads are applied by revision per resource key as §4.2 says; the page
  keeps one held revision per key and drops a reply older than the held one.
- Subscribe-first is the page's order: open the socket, wait for `hello`,
  fetch `waiting`, `projects` and each `projects/{id}/sessions`, apply
  buffered events, then enable the reply box. On `changed {sessions}` or
  `changed {waiting}` it refetches that collection, which is what moves a
  session that starts waiting to the top without a reload. On `changed
  {session}` it refetches that session; on `changed {screen|conversation}`
  for the watched key it refetches the content (`last-response` for Claude
  Code kind, `screen` otherwise). A watched key is re-read every thirty
  seconds as §4.4 says, which also covers an evicted key. On
  `resync_required` or a close it reconnects with backoff and refetches all.
- Page loads exposed to the pool of 4 (§4.1, facts, Unresolved on Safari's
  connection count): static replies carry `Connection: close`, so the
  unauthenticated slot is freed when the file is sent and a page load cannot
  hold the pool through the 30 s idle timeout (`IDLE_TIMEOUT`,
  `src/adapters/remote.rs:573`). The page retries a failed subresource load
  once and reconnects a dropped socket. After pairing, every request carries
  the bearer header, moves its connection to the paired pool
  (`:834-849`), and the unauthenticated pool is not used again.
- A new log line: none. `refuse` (`src/app.rs:3754`) logs method and path,
  never the query, and the static handler logs nothing, so `GET /?code=…`
  is never written down.

## The actions and effects

- **No `AppAction` or `Effect` variant.** The page is static bytes the
  connection thread serves; `GET /api/v1/device` is a read answered on the
  frame the way `remote_info` is. The pairing window, its code, scope and
  failure count (`src/core/remote.rs:46-73`) are untouched.
- **`src/adapters/remote.rs` — the listener.**
  - `RemoteRequest` (`:359`) gains `origin: Option<String>`, `host:
    Option<String>` and `subprotocol: bool` (the client offered
    `switchboard.v1`); `read_request` (`:444`) reads `origin`, `host` and
    `sec-websocket-protocol`, and when there is no `Authorization` bearer it
    takes the token from a `bearer.` entry into `bearer` (so the token is
    never held in a field that could be logged with headers). No test builds
    `RemoteRequest` literally (facts, Settled). `RemoteReply` (`:388`) is
    **not** changed: its literals (about twenty in `src/app.rs`) would all
    need touching, and static replies never reach the frame.
  - `respond_upgrade` (`:519`) takes `subprotocol: bool` and, when true,
    writes `Sec-WebSocket-Protocol: switchboard.v1`.
  - A new `respond_static(w, status, content_type, body)` beside `respond`
    (`:530`) writes the headers below and `Connection: close`.
  - `connection` (`:768`), after `limited` and before the queue send, answers
    on the connection thread: a `GET` of a static path from the table; any
    other path without the `/api/v1/` prefix with the existing JSON 404
    (`not_found`, `no such endpoint`) and `Connection: close`; a non-`GET`
    to a static path with 405. Then it ends the connection. So the frame's
    own refusal (`src/app.rs:3023`) becomes unreachable and stays as a guard.
  - `Limits` (`:583-640`) gets a third window, static paths at 120 a minute
    per listener, answered 429 as the others (a page load is four requests;
    the number is the owner's, facts, For the owner 6, and changes nothing
    else). Static requests count against the unauthenticated pool while the
    connection lives, and `Shared` (`:642`) carries the certificate DER.
- **The static table** (`web/` embedded with `include_str!`, no build step,
  no dependency, portable to Linux CI):

  | Path | Content-Type | File |
  |---|---|---|
  | `/` | `text/html; charset=utf-8` | `web/index.html` |
  | `/app.js` | `text/javascript; charset=utf-8` | `web/app.js` |
  | `/app.css` | `text/css; charset=utf-8` | `web/app.css` |
  | `/switchboard.cer` | `application/x-x509-ca-cert` | the identity's DER |

  Every static reply carries: `Cache-Control: no-store` (so a page that
  arrived with a code in its URL is never cached, and no version is served
  stale after an update), `Content-Security-Policy: default-src 'none';
  script-src 'self'; style-src 'self'; connect-src 'self'; base-uri 'none';
  form-action 'none'; frame-ancestors 'none'`, `X-Content-Type-Options:
  nosniff`, and `Referrer-Policy: no-referrer`. No inline script, no inline
  style, no external resource. `connect-src 'self'` covering `wss://` on the
  same host and port is standard CSP3 behaviour that older WebKit may not
  honour; the page works the same on any browser that does, and the acceptance
  run records it. There is no favicon: `/favicon.ico` is the generic 404.
- **`src/app.rs`.**
  - `serve_remote` (`:3017`): a new authenticated arm `("GET", ["device"])`
    answering `{device_id, name, scope}` of the calling device. It is not a
    content route and needs no worker.
  - **Origin.** Where the request is a `POST` or an upgrade and carries an
    `Origin`, the frame refuses with 403 `bad_origin` unless the origin's
    host and port equal the request's `Host` header (same-origin by
    construction, so no allowlist of names or addresses to maintain: IP,
    `.local` and loopback all work). RFC 6455 §10.2 "SHOULD verify" (facts,
    Settled); the allowlist question is the owner's (For the owner 4) and
    this default needs no answer. Native clients send no `Origin` and are
    unaffected. This is defence in depth: the token is not an ambient
    credential.
  - `bind_remote` (`:2946`) passes the DER to the port.
  - `remote_pair` (`:3606`) is unchanged: the page posts `{code,
    device_name, spki_sha256}` as the native clients do and keeps the reply's
    token.
- **Pairing text.** `src/ui/remote.rs:160-166` builds the payload as the URL
  `https://<host>:<port>/?code=<code>&key=<spki_sha256>&id=<server_id>`, with
  an IPv6 address in brackets. The same string is the QR and the "Copy
  invite" text (`:199-206`). The JSON form is no longer produced (decision
  3: nothing needs it; the headset pairs from Bonjour and a typed code, and
  the CLI reads text).
- **Swift.** `PairingInvite.parse` (`client/Sources/SwitchboardProtocol/
  Info.swift:93-116`) tries a URL first (`URLComponents`: `code`, `key`, `id`;
  the host and port become the one-element `addresses`) and JSON second, so an
  older pasted invite still works. `addresses` stays a list for the
  headset's Bonjour path (`KnownServer.swift:14`, `ConnectionManager.swift:483`,
  `switchboard-remote/main.swift:120`). Nothing else under `client/Sources`
  changes. A pasted URL is read as text and never opened.

## The UI

- **The page** (`web/index.html`, `web/app.js`, `web/app.css`; plain
  JavaScript, one file, no framework). On load, before any other code:
  read `location.search`, call `history.replaceState(null, "", "/")`
  (MDN: modifies the current history entry, facts, Settled), then act on what
  was read. Whether the code was in history before the script ran, and
  whether iOS keeps it in recently closed tabs, are Unresolved; the code is
  single-use (facts, Settled), so a leaked one is dead once the pairing
  completes or the window closes at five minutes or three failures.
- **Pairing**: `GET /api/v1/info`; refuse when its `spki_sha256` or
  `server_id` differs from the URL's ("this is not the Mac whose code was
  scanned"); show the key's short form; `POST /api/v1/pair`; store the entry;
  list the sessions. Op ids: the page does not call `crypto.randomUUID()`,
  which exists only in a secure context, and whether a click-through page is
  one is Unresolved (facts); it makes ids from `crypto.getRandomValues` bytes
  in hex, which §4.5 needs only as unique.
- **No code, no token**: one line, "Open Remote devices on the Mac, choose
  Allow pairing, and scan the QR code with this device's camera," and the
  certificate link. No entry form: the code never has a reason to be typed
  into the page.
- **Header**: host name, the device's scope from `GET device` (refreshed on
  every reconnect), and a Forget button that drops the stored entry.
- **List**: grouped by project, waiting first (`waiting`, `CardState` order as
  §8), each with kind, card word and `waiting_reason`. Workers hidden as the
  endpoints hide them. No excerpt (departure above).
- **Session view**: last response for a Claude Code session, the screen for
  anything else, refreshed on `changed` as above. The reply box posts `send`
  `{op, text}` and shows only under `control` scope; a timeout is followed
  by `GET ops/{op}` before any resend (§4.5). When `prompt_refusal` or
  `trust_question` is set the box is replaced by that sentence; answering
  trust, resume and a new session are not on the page.
- **States said plainly**: "Identity changed, pair again" (nothing sent,
  token included); "This device was removed on the Mac. Pair again." (401
  with the marker held and the key unchanged); "The Mac is not reachable";
  "Pairing is closed on the Mac."
- **The Mac's dialog** (`src/ui/remote.rs`): the QR is the URL; add the
  Decision 1 trust sentence under it and a line naming the certificate link.
  The paired-devices list, scope and Revoke are unchanged; a revoked page
  learns on its next request, and revoke closes its socket as it does for any
  device (`revoke_closes_the_devices_socket`, `tests/remote.rs`).
- Dev aid: the existing script line `pairing <code>` opens a window with a
  known code, which with `SWITCHBOARD_DATA_DIR` and
  `SWITCHBOARD_TMUX_SOCKET` drives the page by hand. No new script line.

## The tests

- `tests/remote.rs`: the `http` helper (`:245`) returns the response
  headers as well as status and body, so existing callers are adjusted.
  - Static paths: each table entry answers 200 with the stated
    `Content-Type`, the CSP, `Cache-Control: no-store`, `nosniff`,
    `Referrer-Policy` and `Connection: close`; a `POST` to `/` is 405; an
    unknown path and `/favicon.ico` are 404; none of them reaches the frame
    (the inbox is unchanged).
  - **Embedded references**: every `src=`/`href=`/`fetch("…")`/`new
    WebSocket` path in the three embedded files is either in the table or an
    `/api/v1/` path on the allowlist; the CSP contains no host name and the
    HTML has no inline `<script>`, `style=` or `on…=`.
  - Pool: twenty sequential static loads leave the unauthenticated count at
    zero, and a following `/info` is still answered (model: `stalled_strangers_
    fill_their_own_pool_and_never_a_paired_clients`, `:3226`). A rate-window
    test for the static window beside `pair_is_rate_limited_per_listener`
    (`:1406`).
  - **Pairing as the page does it**, with the test's own HTTP client as the
    CLI does: `GET /` (no code), `info`, `pair` with the URL's key and code,
    the token, then `GET device` and `GET waiting`. A wrong key and a wrong
    code are refused and count against the window.
  - **Subprotocol**: a socket opened with offers `switchboard.v1` and
    `bearer.<token>` and no `Authorization` header is accepted, the 101
    carries `Sec-WebSocket-Protocol: switchboard.v1` and not the token; a
    wrong token gets 401; a revoked device's socket closes; the existing
    bearer-header path (`subscribe_at`, `:1960`) is unchanged.
  - **Origin**: a `POST` or upgrade with a mismatched `Origin` is 403
    `bad_origin`; a matching one and none at all pass.
  - `GET device` answers the scope and follows a scope change in the dialog
    on the next request; a read device's `send` is refused by name.
  - Certificate: a reissued certificate keeps `spki_sha256`, carries the
    stated IP and DNS SANs, `serverAuth`, and a validity under 825 days; the
    port serves it and the pinned client still connects; an identity file
    without `certificate_addresses` is reissued once and not again. Unit
    tests beside `make_identity` (`src/adapters/remote.rs:982`) cover the
    certificate; parse it with the crates already in the tree, adding none.
- `tests/ui.rs:7492` (the invite is JSON with `addresses`) becomes: the
  invite is an `https://` URL with `code`, `key` and `id`, an IPv6 address
  bracketed. The Remote devices dialog test also finds the trust sentence.
- Swift (`client/Tests`): `PairingInvite.parse` reads the URL form, the JSON
  form, an IPv6 host, and rejects a URL with a missing `key`; the fixtures
  need no change (no view changes), but `FixturesTests.swift` is re-run.
- **Not tested by CI**: the page's own logic (query parse, `replaceState`,
  revision reconcile), since the tree has no JS runner and the issue forbids
  a dependency. It is checked by hand in the acceptance run, and the file
  stays small and plain for that reason.
- **Acceptance run** on a real phone, recorded in the PR: open the pairing
  window, scan, accept the warning (or install `/switchboard.cer` if the
  browser refuses it), see the sessions; start a session waiting and watch it
  move to the top; send a reply and see it arrive; revoke the phone and see
  the page say so on its next request. Record also what the facts left open:
  whether the click-through covered `fetch` and `wss://`, whether the load
  filled the pool of 4, whether the CSP's `connect-src 'self'` admitted the
  socket, and whether `.local` resolves. The build is not gated on any of
  them.
- **Docs, in the same commit.** `docs/remote-client.md` §4.1 (static
  responses on the connection thread, the static window), §4.3 (`device`),
  §5 (line 306's pre-pairing set, the URL invite at `:309`, the relay
  sentence, the subprotocol carrier and `localStorage` against line 312, the
  SAN certificate and reissue). `docs/design.md`: a line in "Remote client"
  (`:3107`) and the "Known gaps at the first merge" item (`:3573`): no excerpt
  on the list, the page's logic untested in CI. `README.md`: a Layout row for
  `web/`, the Development text at `:276-278` (Copy invite is a URL; the CLI
  takes it), and the data-directory text for `identity.json`.
  `CHANGELOG.md` Unreleased: "A web page on the remote port: pair a phone
  from the QR code and answer waiting sessions from its browser."

## Commit message

`Serve a web control surface on the remote port, paired from a URL`

## Contract additions (copy into the commit message body)

- `Identity.certificate_addresses: Vec<String>` (`#[serde(default)]`);
  `IDENTITY_SCHEMA_VERSION` from the value in `src/ports/remote.rs` at
  implementation time, plus one.
- `RemoteRequest.origin`, `RemoteRequest.host`, `RemoteRequest.subprotocol`
  (`src/adapters/remote.rs`; not under `ports/`, no existing literal).
- `GET /api/v1/device` → `{device_id, name, scope}` (authenticated, read).
- Static paths served before pairing: `/`, `/app.js`, `/app.css`,
  `/switchboard.cer`; refusal `bad_origin` (403).
- WebSocket subprotocols `switchboard.v1` (echoed) and `bearer.<token>`
  (read, never echoed).
- Pairing invite is a URL `https://<address>:<port>/?code=&key=&id=`;
  `PairingInvite.parse` reads it and the earlier JSON.
- No `model.rs`, `AppAction`, `Effect`, `SessionView` or `RemoteReply`
  change.
