# Plan: issue #230, a browser page on the remote port

One commit. The app runs at every step: the page is additive, the native
clients keep working through every step, and the last step is the docs.
References are to this worktree at 7fe8949. "Facts" is the facts file.

## Where this departs from the issue

- **"The one-line excerpt the card shows."** There is none on the wire.
  `SessionView` (`control/src/lib.rs:636-688`) has the card word and
  `waiting_reason` and no excerpt, and the Stop hook's `last_message`
  never reaches the record: `src/adapters/hooks.rs:196` builds it,
  `src/core/events.rs:132` reads only `pending` (Facts, Settled). The spec's
  claim that the excerpt is "a field of the session's metadata"
  (`docs/remote-client.md` §4.3, `session.last_response`) is wrong today.
  The page's rows show kind, the card word and `waiting_reason`. For the
  waiting sessions only, at most 8, it also fetches `last-response` once,
  without a `watch`, and shows its first line. The list works without that
  fetch; it is the enhancement. No `SessionView` field, so no fixture change.
- **"The device's scope in the header."** No endpoint returns it after
  pairing; `/info` is unauthenticated (`src/app.rs:3568`), and the envelope
  carries only `epoch` and `revision` (`:3730`). §5's "the host answers
  with the device's scope" is not true of the code. The plan adds an
  authenticated `GET /api/v1/device`. It is also the page's cheap probe for
  "am I still paired".
- **"'Revoked' said plainly."** `authenticate` (`src/app.rs:3125-3153`)
  refuses a revoked token and a never-valid one with the same code,
  `unauthorized`. No new code: the page keeps a non-secret marker of what
  it paired with (below), so a 401 while it holds a token is "revoked or
  forgotten", and a changed `/info` key is "identity changed".
- **"Any path outside `/api/v1/` is refused."** True, but on the frame,
  after the request was queued (`src/app.rs:3023`). The static paths are
  answered on the connection thread before the queue, because they need no
  core state (§4.1: "the frame does core state only").
- **§5's "a relayed request through a substituted certificate fails".**
  Not true for a browser: the page is what posts `spki_sha256` to `/pair`,
  so under a relay the impostor's page supplies the host's real key. The
  `key` in the URL catches a genuine page on the wrong genuine host (a
  second installation at that address, a rotated key), not an impostor.
  §5 says so in the amendment.
- **Facts, Unresolved 8: "the code is single-use, refused after its first
  attempt."** The tree says otherwise. The code closes with the first
  *successful* `/pair` (`src/core/remote.rs:65-70`) and a wrong code counts
  toward three (`:56-64`). So a leaked URL is usable until the owner's own
  pairing succeeds or five minutes pass. That is the exposure the QR has
  today; the page sends the code once and the window closes on success, so
  a URL read from history after the pairing is dead. The plan does not
  change the window.
- **Facts, Unresolved 7: "whether `KeyPair::from_pem` exists for a reissue
  is unverified."** It is already called at `src/adapters/remote.rs:137`
  and `:146`; reissuing over the stored key compiles.
- **Decision 1's click-through.** The facts settle that on iOS a
  click-through does not carry `wss://` (WebKit 41419; code-server's iPad
  guide), and Unresolved 1 leaves open whether iOS offers one at all. The
  plan therefore does not make the socket, or the page, depend on it (see
  Decision 1).

## The record

No new record kind, no `model.rs` change, no `SCHEMA_VERSION` (16,
`src/core/model.rs:12`) or `VIEWS_SCHEMA_VERSION` (7, `:612`) bump. A
browser is a `PairedDevice` (`src/ports/remote.rs:96-107`) in
`devices.json`, which is an unversioned bare list; the name is the page's
`device_name` (`remote_pair`, `src/app.rs:3650`), cut to 64 characters by
`new_device` (`src/core/remote.rs:166-175`). **Decision 6: the name is
enough**, no `kind` field (the dialog lists it like any device and
Revoke works unchanged). Rehydration is unchanged: `IdentityLoaded`
(`src/core/remote.rs:22-30`) puts the devices back, `authenticate` checks
the token hash on every request.

One change to `identity.json`: the certificate gains names and a bounded
validity (Decision 1), and the file records which, so a reissue happens
when they change and not on every launch (a reissue changes the
certificate, and a browser's exception is per certificate).

- `Identity` (`src/ports/remote.rs:26-42`) gains
  `#[serde(default)] certificate_names: Vec<String>` and
  `#[serde(default)] certificate_not_after: Option<SystemTime>`.
- `IDENTITY_SCHEMA_VERSION` (1 today, `src/ports/remote.rs:17`) goes to
  **the value there at implementation time, plus one**; a ticket landing in
  parallel may take the next number first. Readers accept the older file
  (both fields default); an identity with no `certificate_not_after` is
  reissued once at the next launch. There is no migration step to write
  beyond the defaults: `RemoteFiles::load` is plain serde
  (`src/adapters/remote.rs:49`). `Identity` is built literally in
  `src/adapters/remote.rs:117`, `src/core/tests.rs:11769` and
  `tests/ui.rs:7436`; those three gain the fields.
- The SPKI does not move (same key), so every pin and every pairing holds.
  `make_identity` today sets no subjectAltName and no validity
  (`src/adapters/remote.rs:95-127`); the "year 4096" comment at `:91` is
  unverified (Facts, Unresolved 7). The new certificate sets explicitly:
  SANs, `serverAuth` EKU, `not_before` a day back, `not_after` 800 days out
  (iOS 825-day ceiling, Facts Settled), `IsCa::NoCa`.

**The page's own state on the device** (not a host record): in
`localStorage`, `sb.<server_id>` = `{token, device_id, name, scope, key,
host_name}`. See Decision 2 for why the token is there.

## The signals

None new. There is no `changed` kind or event type to add: the page
consumes the existing `hello`, `changed {session|sessions|waiting|
conversation|screen}`, `removed`, `watching` and `resync_required`
(`docs/remote-client.md` §4.4) and the revision rule. `GET /api/v1/device`
is a read, not a signal; the page refetches it on `hello` and every 30 s.

The page's live/poll behaviour is a client rule, specified in the UI
section because it follows from a settled fact: a click-through may not
carry `wss://` (iOS), so the page must work with no socket.

## The actions and effects

No `AppAction` or `Effect` variant: nothing here changes core state. The
pairing window, `OpenPairing`/`Paired`/`PairingFailed` and the devices are
as they are. What is added lives at the edges.

### Decisions settled here

**1. Trust: the click-through is the trust step; the certificate download
is offered beside it, not required.** The default must work whichever way
O1 and Unresolved 1 are answered, so:
- *Default.* Page and REST over HTTPS after whatever the browser lets the
  owner do (a click-through where one exists). The pairing dialog says in a
  sentence: "The first visit shows a certificate warning. Continue past it;
  the page then checks this host's key against the one in the link. That
  check catches the wrong Switchboard, not a stranger on your network
  serving a copy of this page: pair only from this dialog's code, on your
  own network." This is what the click-through buys: transport encryption
  to whoever answered, and the page's key comparison. It does not buy
  authentication of the host (a browser cannot read the certificate it is
  talking over, so the `/info` comparison proves only what the host
  *claims*; notes l.217-227).
- *The socket is optional.* The page opens `wss://` and, if it has not
  seen `hello` in 5 s or the socket closes, runs **polling mode**:
  `GET waiting`, the open project's sessions and the open session's
  content every 5 s, retrying the socket every 60 s. The header says
  "live" or "polling". The acceptance step "a session that starts waiting
  moves to the top without a reload" holds in both modes (within 5 s when
  polling). The socket is never a precondition of the page.
- *Enhancement, behind the default.* `GET /switchboard.crt` serves the
  certificate (DER, `application/x-x509-ca-cert`), and the landing screen
  links it as "Trust this host's certificate" for a device whose browser
  refuses the socket. The certificate is public (every TLS handshake sends
  it), so serving it before pairing reveals nothing. Whether iOS then
  accepts `wss://` with this certificate (it may need `CA:true`, per the
  code-server guide; Facts O1) is the owner's to confirm on a device; the
  plan does not claim it works and the README says "untested on a device".
- *Names.* The certificate carries SANs `localhost`, `127.0.0.1`, the
  address from `local_address()` (`src/adapters/remote.rs:962`) and
  `<hostname -s>.local`, because Chrome and Safari match names only
  against SANs (Facts, Settled) and iOS requires one. `.local` reaches
  Mobile Safari over mDNS (Facts, Settled; not tested on a device), but
  the URL in the QR uses the IP, which needs no resolver. The certificate
  is reissued over the stored key (`KeyPair::from_pem`) in
  `load_identity` (`src/app.rs:2782`) when `certificate_names` differs from
  the current names or `certificate_not_after` is under 30 days away; never
  in `bind_remote`, which would loop through `Effect::SaveIdentity`. An
  address that moves while the app runs is covered at the next launch
  (the `.local` name covers the common case); the dialog's text says so.

**2. The token: `localStorage`, sent as `Authorization: Bearer` on `fetch`
and as a `Sec-WebSocket-Protocol` value on the socket. No cookie.**
Why: cookies are not isolated by port (RFC 6265 §8.5, Facts Settled), so a
cookie would go to every HTTPS service on the Mac; a `Strict` cookie can be
missing from a WebSocket in a restored Safari tab (WebKit 200345 and
232666, Facts Settled); and `Secure` on an IP host is unanswered (Facts,
Unresolved 5). The protocol route depends on none of these (Unresolved 4).
Cost, said in the dialog's sentence and the spec: any script on the origin
can read `localStorage`. The origin serves only our two files, under a CSP
with `script-src 'self'` and no inline script, and the page writes every
untrusted string (session names, replies, screens) with `textContent`,
never `innerHTML`; a test asserts the latter. The token is never in a URL
(§5).
- The socket offers two protocols, `switchboard.v1` and the token
  (base64url is a legal `token`, Facts Settled). The host takes the second
  value as the bearer and **echoes `switchboard.v1` in the 101**, never the
  token: a browser fails a socket that offered protocols and got none
  (WHATWG step 11.2, Facts Settled). Native clients keep the header.
- *Origin.* RFC 6455 §4.2.2 says a server that does not validate `Origin`
  accepts connections from anywhere (Facts, Settled). `remote_subscribe`
  (`src/app.rs:2723`) refuses with 403 `bad_origin` when an `Origin` header
  is present and its authority is not the request's `Host`. Same-origin
  works whatever name the device used (IP, `.local`, loopback); no
  `Origin` (native clients) is accepted. Commands carry the bearer header,
  which a cross-site page cannot set, so POSTs need no check.
- *Op ids.* `crypto.randomUUID` is secure-context only (Facts, Settled) and
  whether a click-through origin is one is unanswered (Unresolved 2, O2).
  The page uses `getRandomValues` to make a UUID always; no branch.

**3. One QR, a URL.** The QR and "Copy invite" carry
`https://<host>:<port>/?code=<code>&key=<spki_sha256>&id=<server_id>` (an
IPv6 host in brackets). Nothing produces the JSON after this. The Swift
`PairingInvite.parse` (`client/Sources/SwitchboardProtocol/Info.swift:
93-116`) tries a URL first (`URLComponents`: `code`, `key`, `id`, host and
port become `addresses`) and decodes the JSON second, so a pasted old
invite still works; the visionOS paste field, `ConnectionManager` and
`switchboard-remote pair` all go through `parse` and need no change. That
is the four `Sources` users and eight `Tests` files of Facts, Settled; none
reads `addresses` as more than a list.

**4. Static paths.** Answered by the connection thread in `connection`
(`src/adapters/remote.rs:~773`), for `GET` only, before the rate check and
the frame; any other method on these paths goes to the frame and is
refused as today.

| Path | Content-Type | Source |
|---|---|---|
| `/` | `text/html; charset=utf-8` | `web/index.html` |
| `/app.js` | `text/javascript; charset=utf-8` | `web/app.js` |
| `/app.css` | `text/css; charset=utf-8` | `web/app.css` |
| `/favicon.ico` | none, `204` | (so the browser's guess costs no pool slot twice) |
| `/switchboard.crt` | `application/x-x509-ca-cert` | the identity's DER |

Every static reply carries `Cache-Control: no-store` (the page and the
host move together; nothing to revalidate), `X-Content-Type-Options:
nosniff`, `Referrer-Policy: no-referrer` (the page's first URL carries
the code), `Content-Security-Policy: default-src 'none'; script-src 'self';
style-src 'self'; connect-src 'self'; img-src 'self'; base-uri 'none';
form-action 'none'; frame-ancestors 'none'`, and **`Connection: close`**.
`connect-src 'self'` may not cover `wss:` on an older Safari; then the
socket is blocked and polling mode runs, which is the default design
working. The query on `/` is ignored by the server; it is never logged
(the connection thread logs nothing; `refuse`, `src/app.rs:3754`, never
logs a query either).
- *Pool and rate.* §4.1 allows four unauthenticated connections and the
  accept loop drops a fifth (`src/adapters/remote.rs:699-706`). A static GET
  with `Connection: close` frees its slot when the reply is written, so a
  page load (three files, up to three parallel connections, plus the
  favicon) holds the pool only for the length of the transfer; a browser's
  kept-alive connection never sits in it for 30 s. They count against the
  same unauthenticated pool as everything before pairing. They get a rate
  window of their own, `STATIC_PER_MINUTE = 120` beside `PAIR_PER_MINUTE` and
  `INFO_PER_MINUTE` (`:583-587`), answered 429 past it. How many parallel
  connections iOS Safari opens, and whether a page load plus the warning's
  failed handshake exhausts the pool, is O5 and Unresolved 6: the plan
  builds to a small count (the page is three files, no fonts, no images)
  and the owner measures before the pool size is touched. If a connection
  is dropped at accept, the page's `fetch` retries once after 1 s.
- *Without a code and without a token* the landing screen shows the host
  name and the short key from `/info`, the sentence "Open Remote devices on
  the Mac, choose Allow pairing, and scan the code", a code field (the key
  is taken from `/info`; the owner compares its short form with the
  dialog's) and the certificate link.
- `RemoteRequest` and `RemoteReply` are in `src/adapters/remote.rs:359-393`,
  not under `ports/`. No test builds either as a literal; `RemoteReply` is
  built in `src/adapters/remote.rs:506` and `src/app.rs` at `:1664`,
  `:1679`, `:2365`, `:2371`, `:2376`, `:3570` and in the helpers (Facts,
  Settled). So `RemoteReply` is left alone (static replies never reach
  the frame). `RemoteRequest` gains `host: Option<String>`,
  `origin: Option<String>` and `subprotocol: Option<String>` (the offered
  list, token excluded). `read_request` (`:444-513`) reads `host`,
  `origin` and `sec-websocket-protocol`; on an upgrade with no
  `Authorization` it sets `bearer` from the protocol list's non-
  `switchboard.v1` item. `respond_upgrade` (`:519`) gains an echoed
  protocol argument. The only literal is in `read_request`.

**5. Where the page lives.** `web/index.html`, `web/app.js`, `web/app.css`,
embedded with `include_str!` in a `src/web.rs` that also holds the five
paths and their headers as constants, so the router and the tests read one
list. No build step, no dependency, no framework. Portable to Linux CI.

### Host changes, in order

1. `src/web.rs`: the assets, `Static { path, content_type, body }` lookup,
   the header block.
2. `src/adapters/remote.rs`: the three request fields; the subprotocol
   token; `respond_upgrade` echo; `respond_static`; the static arm in
   `connection` with its rate window; `RemotePort::bind` gains a
   `certificate: Vec<u8>` parameter (one caller, `src/app.rs:2969`) so
   `/switchboard.crt` has its bytes without the frame.
3. `src/adapters/remote.rs`: `make_identity` builds through a new
   `make_identity_for(.., names)` (additive; `make_identity` keeps its
   signature and calls it with `certificate_names()`), plus
   `reissue_certificate(identity, key_pem, names)` that asserts the SPKI is
   unchanged before it returns.
4. `src/app.rs`: `load_identity` reissues when due (not for a read-only
   instance), saving through the store's atomic path; `serve_remote`
   (`:3017`) gains `("GET", ["device"])` after `authenticate` answering
   `{device_id, name, scope}`; `remote_subscribe` checks `Origin`. Neither
   needs the allowlist's wire translation. Static paths never reach it, so
   the prefix refusal at `:3023` stays as written.
5. `src/core/remote.rs`: `pairing_url(host: IpAddr, port, code, key,
   server_id) -> String`, pure, so the UI holds no string logic.

## The UI

**Host side (`src/ui/remote.rs`).** The `json!` payload at `:160-166`
becomes `pairing_url(...)` and feeds the QR and "Copy invite" (`:199-206`)
unchanged in shape. Under the code, one line shows the URL's text. Under
the "Scan, or type the key and the code" line, the sentence from
Decision 1. The paired-devices list (`:230+`) is unchanged: a browser
appears as "Safari on iPhone" and Revoke is the revoke.

**The page (`web/`).** First cut, as the issue lists it, plus the rules
that follow from the decisions:
1. *Load.* If the query has `code`, `key`, `id`: copy them to variables,
   call `history.replaceState(null, "", "/")` **first**, before any
   `fetch`. Then `GET /api/v1/info`; if `info.spki_sha256 !== key` or
   `server_id !== id`, say "This is not the host whose code was scanned"
   and send nothing. Else `POST /api/v1/pair {code, spki_sha256: key,
   device_name}` (name from `navigator.userAgent`: "Safari on iPhone",
   "Chrome on Mac", else "Browser"). Store the marker and the token.
   Nothing logs the query; the code is never stored.
2. *Opened with a token.* `GET /api/v1/device`. 200: header shows host
   name and scope. 401: "This device was revoked, or the host forgot it.
   Pair again." and the token is dropped; `info.spki_sha256` differing
   from the marker's `key`: "This host's identity changed. Pair again."
   and nothing is sent, token included (§5). No token, no code: the
   landing screen of Decision 4.
3. *Sessions.* `GET projects`, then `projects/{id}/sessions` per project;
   a `GET waiting` set marks and sorts waiting ones first, within each
   project; rows show name, kind, card word, `waiting_reason`. Dispatch
   workers stay hidden (the default of `hide_workers`).
4. *Live.* Subscribe first, then snapshot (§4.4): open
   `wss://<host>/api/v1/events` with the two protocols, wait for `hello`,
   fetch, reconcile by revision, refetch what a `changed` names whose
   revision is past the snapshot's. On `hello` with a new `epoch`, drop
   everything. On `resync_required`, reconnect. Polling mode as in
   Decision 1.
5. *Session view.* A Claude Code session: `last-response`, watched through
   `watch {conversation, id}` and fetched only after `watching`. Anything
   else: `screen`, watched the same way. In polling mode it is refetched
   every 5 s. A visible view also refetches every 30 s (§4.4). Unwatch on
   leaving. The page holds at most the open session's key.
6. *Reply box.* Shown when the `device` scope is `control` and the session
   is not at `trust_question` and `prompt_refusal` is empty; hidden
   otherwise (resume and the folder-trust answer are not in this cut). It
   posts `sessions/{id}/send {op, text}`; on a timeout it asks
   `ops/{op}` before it retries (§4.5), so a send is never typed twice. A
   `control`-refused reply updates the stored scope and hides the box.
7. All untrusted text through `textContent`. The layout is one column,
   readable at 320 px wide, `prefers-color-scheme` aware; no framework.

## The tests

- **`src/adapters/remote.rs` unit tests.** `read_request` takes `Host`,
  `Origin` and the protocol list and puts the token into `bearer` only on
  an upgrade, never into any logged field; `respond_upgrade` echoes
  `switchboard.v1` and never the token; a reissued certificate keeps the
  SPKI (`spki_sha256_of_cert`), carries the SANs and a `not_after` within
  825 days, and the old pin still verifies through `pinned_client`;
  `RateWindow` for the static window.
- **`src/core/tests.rs`.** `pairing_url` for an IPv4 and an IPv6 host
  (brackets), and that it contains exactly `code`, `key`, `id`.
- **`tests/remote.rs`** (header assertions need `http` at `:245-296` to
  return the response headers; extend it, keeping its signature for the
  existing callers by adding `http_full`):
  - the five static paths answer to an unpaired client with the table's
    `Content-Type`, CSP, `Cache-Control: no-store`, `nosniff`,
    `Referrer-Policy` and `Connection: close`;
  - the embedded page references only those paths and `/api/v1/*` paths in
    the allowlist (a scan of `index.html` and `app.js` for `src=`,
    `href=` and string paths), has no inline `<script>` or `on…=`
    attributes, and `app.js` contains no `innerHTML`, `eval` or
    `document.write`;
  - `GET /?code=…` is answered and the code is in no log line (use the
    existing log capture, or assert `refuse` is not reached);
  - `info_is_served_to_anyone_…` (`:426`) and the allowlist test (`:682`)
    still hold: `/api/v1/device` unpaired is 401, paired returns the
    device's scope and a scope change in the dialog shows on the next
    call; a revoked token gets 401 on `device`;
  - the pairing end to end as a browser would do it, over the host's own
    client: `GET /info`, `POST /pair` with the URL's values, the token on
    `device`, `waiting`, a `send` under `control`, a `send` refused under
    `read`; the one-attempt and three-failure rules from `:582` unchanged;
  - the socket with a subprotocol: `subscribe_at` (`:1960`) offering
    `["switchboard.v1", token]` gets a 101 echoing `switchboard.v1`, with
    `Origin` equal to `Host` it is accepted, with another `Origin` 403
    `bad_origin`, with no `Origin` (native) accepted; revoke closes it
    (`:2892`); a wrong token in the protocol is 401;
  - static GETs with `Connection: close` do not hold the pool: eight
    sequential page loads all succeed, where `stalled_strangers_…` (`:3226`)
    is unchanged; past `STATIC_PER_MINUTE` the reply is 429;
  - the identity reissue: a `FakeRemoteStore` with an old identity gets a
    new certificate at `load_identity` with the same `spki_sha256`, and no
    second reissue at the next launch.
- **Client.** `client/Tests`: `PairingInvite.parse` of a URL (IPv4, IPv6,
  a host with a port) and still of the JSON fixture; `swift test` in
  `client/` per its README. `client/Fixtures` unchanged (no view changed);
  `device.json` is added, written by `SWITCHBOARD_WRITE_FIXTURES=1` and read
  by both sides, since `/device` is a new body.
- **Not tested by CI:** the page's script logic (no JS runner, and the issue
  rules out a dependency), and how any browser behaves. The acceptance run
  is by hand: `SWITCHBOARD_SCRIPT` with `remote-listen lan <port>` and
  `pairing <code>` against a throwaway data dir, a phone on the network
  scanning the QR; sessions appear, a waiting session moves up (live or
  within 5 s polling), a reply arrives, revoke in the dialog and the page
  says so on its next request. Whether the socket works on that phone is
  recorded as an observation (O1, O2, O3, O5), not a pass condition.

## Docs and the one commit

- `docs/remote-client.md`: §4.1 (static paths answered on the connection
  thread, the static window, `Connection: close`), §4.3 (`GET /device`;
  statics and `/switchboard.crt` as the only non-`/api/v1` paths; correct
  the `last_message` claim), §4.4 (the subprotocol and `Origin` rule), §5
  ("before anything but `/info`, `/pair`, the page and the certificate";
  the QR is a URL; the relay sentence corrected for browsers; the
  token's two homes), §3.1 (the certificate's names and validity).
- `docs/design.md`: a bullet under "Remote client (2026-10-12)" (`:3107`)
  with what was built and the known gaps (excerpt only for waiting
  sessions; the socket on iOS unconfirmed; address moves covered at next
  launch; no JS tests). The "Trust boundary" section (`:219`) gets the
  browser's sentence.
- `README.md`: Layout rows for `src/web.rs` and `web/`, the changed
  `src/adapters/remote.rs` and `src/ui/remote.rs` rows, `tests/remote.rs`;
  the dev-aid text at l.265-278 ("Copy invite" is now a URL, which
  `switchboard-remote pair "$(pbpaste)"` still takes and a phone's camera
  opens); the data directory text for `identity.json`.
- `CHANGELOG.md`, Unreleased: "A browser can pair with the remote port from
  the QR code's link and then list sessions, read the last response or
  screen, and send a reply; the QR code is now a link."

Commit message:

`Serve a browser page on the remote port, paired from a link in the QR code`

Contract additions (to copy into the commit body):
- `GET /api/v1/device` (authenticated): `{device_id, name, scope}`; new
  fixture `client/Fixtures/device.json`.
- Static `GET` paths on the remote port: `/`, `/app.js`, `/app.css`,
  `/favicon.ico` (204), `/switchboard.crt` (DER), answered before the frame.
- WebSocket `Sec-WebSocket-Protocol`: `switchboard.v1` plus the token; the
  101 echoes `switchboard.v1`; `Origin`, when present, must match `Host`
  (403 `bad_origin`).
- The pairing invite is a URL `https://<host>:<port>/?code=&key=&id=`;
  `PairingInvite.parse` accepts it and still accepts the JSON.
- `Identity.certificate_names: Vec<String>` and
  `Identity.certificate_not_after: Option<SystemTime>` (both
  `#[serde(default)]`); `IDENTITY_SCHEMA_VERSION` set to its value in
  `src/ports/remote.rs` at implementation time, plus one.
- `RemoteRequest.host`, `.origin`, `.subprotocol` (adapter type, additive);
  `RemotePort::bind` gains a `certificate` parameter; `make_identity_for`,
  `reissue_certificate`, `certificate_names` in `src/adapters/remote.rs`;
  `pairing_url` in `src/core/remote.rs`; `src/web.rs`.
