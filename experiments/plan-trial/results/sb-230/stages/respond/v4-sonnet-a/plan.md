# Plan: a web control surface on the remote port (#230)

Line numbers are as they are in this worktree at 7fe8949. Schema versions
as they stand today: `SCHEMA_VERSION = 16` (`src/core/model.rs:12`),
`VIEWS_SCHEMA_VERSION = 7` (`:612`), `IDENTITY_SCHEMA_VERSION = 1`
(`src/ports/remote.rs:17`). Only the last moves, and it is written below
as "the value in `ports/remote.rs` at implementation time, plus one".
"Facts" means the facts file for this issue; a line cited as Settled N or
Unresolved N is in it.

## Where this departs from the issue

- **"The one-line excerpt the card shows" is not on the wire.**
  `SessionView` (`control/src/lib.rs:636-688`) has no excerpt. The record
  has `last_message` (written by `src/bin/switchboard-hook.rs:37`;
  Settled, tree), and `docs/remote-client.md:207` says the excerpt is
  that field, "a field of the session's metadata" that "moves with
  `changed {session}`", masked before it is cut (`:201`). Masking
  resolves the session's secret set on the worker, not the frame
  (`src/app.rs:1822-1826`, `docs/remote-client.md:463`), so putting
  `last_message` on `SessionView` would send unmasked text from a frame
  answer. Carrying it masked needs a masking path for a metadata field
  that the tree does not have, and the issue is a page, not a change to
  the wire. The plan does not redefine the excerpt (a per-session
  `last-response` fetch is a different thing, and every content read calls
  `touch_watch`, `src/app.rs:2111`, which fills the shared 128-key watch
  set, `:1754-1768`, `:1789-1798`, and costs a whole-transcript parse on
  the one content worker, §4.1). Instead the list rows ship **without an
  excerpt**: session name, kind, `card` word, and `waiting_reason`. The
  open session still shows its last response, fetched under the `watch`
  the spec defines for it. The excerpt is a named gap in the design status
  and a follow-up for the owner (masked `last_message` on the wire).
- **"The device's scope in the header": no endpoint returns it after
  pairing** (`remote_info`, `src/app.rs:3568`, is unauthenticated; the
  `/pair` reply is the only place, and it goes stale when the owner
  changes the scope in the dialog). The plan adds one authenticated read,
  `GET /api/v1/device`. This is the single new read model; the page
  cannot show a current scope without it.
- **"Revoked said plainly" cannot be told from "never paired" by the
  host's answer.** `authenticate` (`src/app.rs:3127-3153`) refuses both
  with `401 unauthorized`, and with an HttpOnly cookie the page cannot
  see whether it holds a token. The page keeps a non-secret marker
  (`server_id`, `device_id`, key hash, name) in `localStorage`. A 401
  with a marker says "This browser is no longer paired (it was revoked on
  the host, or the browser dropped its cookie). Pair again." A 401 with
  no marker says how to pair. "Identity changed, pair again" is the
  marker's `server_id` matching `/info` with a different key hash.
  No new refusal code.
- **"Any path outside `/api/v1/` is refused" is true but late:**
  `serve_remote` refuses it at `src/app.rs:3023`, after the request has
  taken a slot in the 64-deep channel and a frame pass. With a page, a
  browser sends strays (`/favicon.ico`, a prefetch). The connection
  thread now owns this decision: the three page paths are served there, any
  other path outside `/api/v1/` is answered 404 there, and `serve_remote`
  keeps its own refusal as the backstop.
- **"The browser then trusts that certificate for that address" is not
  safe to build on.** Settled (WebKit 41419, code-server's iPad doc): on
  Safari a click-through can cover the page and `fetch` but not `wss://`.
  The page therefore works on REST polling alone and treats the socket as
  an upgrade (decision 1 below).
- **"`history.replaceState` … the code never stays in the history" is
  stronger than the sources.** MDN (Settled) says `replaceState`
  replaces the current entry; it says nothing of what the browser
  recorded before the script ran (Unresolved 6). The plan makes the
  claim the host can keep: the code is single-use and the window closes
  at the first pairing, three failures or five minutes (Settled,
  `src/core/remote.rs:46-72`), so a recorded URL is dead. The dialog says
  that instead of promising the history is clean.

## The record

No new record kind, no `model.rs` change, no `SCHEMA_VERSION` or
`VIEWS_SCHEMA_VERSION` bump. A browser is one more `PairedDevice`
(`src/ports/remote.rs:96-107`) in `devices.json`, which is a bare
`Vec<PairedDevice>` with no version. Its name is the page's `device_name`
("Safari on iPhone", built by the page from `navigator.userAgent`; the
host already cuts it to 64 characters, `new_device`,
`src/core/remote.rs:166-175`). No `kind` field: the name is enough for
the dialog (decision 6) and the host reads no `User-Agent`. Revoke and
scope change already apply on the next request and revoke closes the
socket.

One record changes: `identity.json`. The certificate is reissued over the
same key (Settled: `KeyPair::from_pem` is already used,
`src/adapters/remote.rs:139,146`; the SPKI is of the key, so the pin
and `spki_sha256` do not move, §5). The reason is Settled (Apple's
rules): the current certificate has no subjectAltName, a CN only, no EKU,
and a `not_after` in the year 4096, so it meets none of the SAN, EKU or
825-day rules Safari applies. Reissuing cannot hurt the click-through
path and may help it (Unresolved 1, 2 and 3 are the open questions about
how much), so it does not wait on them. Two additive fields on `Identity`
(`src/ports/remote.rs:26-42`), both `#[serde(default)]`:

- `certificate_names: Vec<String>`: the DNS and IP names the certificate
  carries, sorted.
- `certificate_expires: Option<SystemTime>`.

Bump `IDENTITY_SCHEMA_VERSION` to the value in `ports/remote.rs` at
implementation time, plus one. `identity.json` has no migration step:
`load_identity` is a plain deserialize, so an older file reads with the
defaults (an empty list, `None`), which `bind_remote` treats as "reissue
now". Written through the existing atomic `RemoteFiles::save`.

Rehydration is unchanged: `IdentityLoaded` at launch, devices by token
hash on every request. The pairing window stays non-durable. On the
device, what persists is the HttpOnly cookie (script cannot read it) and
the marker in `localStorage` (not secret: ids and a public key hash).
The cookie is the only place the token lives in the browser (decision 2).

## The signals

### Decision 1: browser trust in the certificate

**Picked: the click-through on the local network is the trust step, said
in the dialog. The host does not offer the certificate for download in
this cut.** Why: a download or profile install is a second trust
mechanism with its own surface (a route that serves a certificate before
pairing, a profile format, what iOS asks of a manually trusted
certificate), and the only evidence that it fixes `wss://` on Safari is
two sources of 2021 and 2026 (Settled), against Unresolved 1-3, which no
experiment here can settle. It is the follow-up if owner question A
comes back unfavourable (named at the end).

What the click-through buys, and what it does not (the dialog says this
in two sentences, and §5 of the spec is amended to say it):

- It lets the owner open the page. It proves nothing about who answered.
- The `key` in the URL lets the genuine page refuse to send the code to a
  host whose `/info` presents a different `spki_sha256`. That catches the
  right page on the wrong genuine host (a second installation on that
  address or port, a rotated key). A page served by an impostor is not a
  genuine page and checks nothing; a page cannot read the key of the TLS
  connection it is on, so even the genuine check proves only that the host
  claims that key. §5's sentence "a relayed request through a substituted
  certificate fails" stands for native clients and is amended: it does not
  hold for a browser.
- What bounds the damage: the code is single-use and short-lived
  (Settled), the pairing window shows scope, and the owner sees the new
  device in the dialog and can revoke it.

The reissue (above) gives the certificate `serverAuth`, a validity of 800
days (under 825, Unresolved 3: if the rule does not bind a manually
trusted leaf, nothing is lost), and SANs for `localhost`, `127.0.0.1`,
the outward address from `local_address()` as an IP SAN, and, on macOS,
`<LocalHostName>.local` as a DNS SAN (Unresolved 2 and 9: both forms are
present, so whichever Safari accepts is there). The dialog says the
certificate is renewed (key, and so pin, unchanged) and that a changed
address means a new browser warning (Unresolved 7).

The page's default is REST polling every 5 seconds plus a refetch of the
open session. When `new WebSocket` opens, the page switches to events
and stops polling; when it fails or closes, the page says "Live updates
unavailable, refreshing every 5 s" and polls. This is the same page
whether Unresolved 1 comes back favourable or not; the socket is the
enhancement, built behind the polling and not in front of it.
`crypto.randomUUID` is used only when `window.isSecureContext`, with a
`getRandomValues` fallback otherwise (Unresolved 4).

### Decision 2: how the token reaches the host

**Picked: an `HttpOnly; Secure; SameSite=Strict` cookie, set by the
`/pair` reply, carried by `fetch` and by the socket handshake. The page's
script never sees the token. Nothing in `localStorage`, no subprotocol
token.** That is the safer store (a script on the origin can read
`localStorage`; it cannot read an HttpOnly cookie), so it is the one this
plan builds. The facts file's Owner D says the server accepts all three
transports; this plan builds one (plus the bearer header the native
clients use) and does not build the other two.

Mechanics that are unresolved are the implementer's first test, before
any other page code (Unresolved 5): on Safari, Chrome and Firefox with the
reissued certificate, does a `Secure` cookie set by `/pair` over an IP
host come back on a same-origin `fetch`? (This is the basic behaviour; the
`__Host-` prefix needs `Secure`, `Path=/`, no `Domain`, all met.) Does it
come back on a same-origin `new WebSocket` handshake? If the `fetch` half
fails, the finding goes to the owner as a follow-up and no
`localStorage` or subprotocol transport is added in this commit. If only
the socket half fails, nothing changes: the page polls (decision 1).

Host side:

- `read_request` (`src/adapters/remote.rs:444-513`) keeps three more
  headers, as the bearer is kept, so none are logged with the rest:
  `cookie`, `origin`, `host`. `RemoteRequest` (`:360`) gains
  `cookie: Option<String>`, `origin: Option<String>`,
  `host: Option<String>`. Settled: only `adapters/remote.rs` builds it,
  and no test constructs it.
- The token is the bearer if present, else the cookie named
  `__Host-sb-<server_id>` (per-installation name: Settled says cookies are
  not isolated by port, so two instances on one Mac would otherwise
  collide). `authenticate` (`src/app.rs:3127`) takes it from a new
  `RemoteRequest::token(&self, server_id)`; the cookie value is checked
  for length and base64url characters before it is hashed.
- **Origin check.** A cookie is ambient: `SameSite=Strict` is by site, and
  a site includes every port on the host, so another page on another port
  of this Mac is same-site. For a request authenticated by the cookie (no
  bearer), any method but GET and any WebSocket upgrade must carry an
  `Origin` equal to `https://` + the request's `Host` header, compared
  case-insensitively; otherwise `403 forbidden_origin`, no side effect.
  Comparing with the `Host` header, not a list of addresses, is what makes
  this correct for an IP, a `.local` name or loopback. Bearer-authenticated
  requests are exempt: a header is not ambient. This answers the owner
  question on `Origin`, and it is a security decision, so it is decided
  here.
- **`/pair`** (`remote_pair`, `src/app.rs:3606`) takes an additive body
  field `"transport": "cookie"`. With it the reply omits `token`, and
  `RemoteReply` carries `Set-Cookie: __Host-sb-<server_id>=<token>;
  Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=34560000` (400 days,
  the longest browsers keep). Without it the reply is unchanged, so the
  CLI and the headset are untouched. `RemoteReply` (`:389`) gains
  `set_cookie: Option<String>` and `#[derive(Default)]`; Settled says it
  is built literally at about 15 sites in `src/app.rs` and none in `tests/`,
  so each literal gets `..RemoteReply::default()`, a mechanical edit.
- The cookie goes to every HTTPS service on that host, on any port
  (Settled). It is `HttpOnly`, so it cannot be read by script; it can be
  sent to a service the owner runs on the same Mac. The dialog says so, and
  it is named for the owner at the end.

### Decision 3: one QR

**One URL.** The pairing window's QR and "Copy invite" carry
`https://<ip>:<port>/?code=<code>&key=<spki_sha256>&id=<server_id>`,
built by one pure function in `src/core/remote.rs`,
`invite_url(address: SocketAddr, identity, code)`. It formats the address
through `SocketAddr`, so an IPv6 result gets its brackets (the JSON form
did not). Code, key and id are Crockford, hex and a UUID: nothing needs
percent-encoding. The JSON form is no longer emitted. Settled: the
visionOS paste field, `ConnectionManager.swift`, `BrowserView.swift:106,
883` and the CLI all go through `PairingInvite.parse`
(`client/Sources/SwitchboardProtocol/Info.swift:114`), so only that
changes: it tries `URLComponents` first (`code`, `key`, `id`, and
`addresses = ["host:port"]`) and the existing JSON decode second, so an
invite copied before the update still pairs. `addresses` stays a list
(`ConnectionManager.swift:483`, `main.swift:120`). The `.local` name is not
put in the URL: owner question B decides that, and the IP works whichever
way it comes back.

### Decision 4: the static paths

- **Paths, exactly:** `GET /` (also with a query), `GET /app.js`,
  `GET /app.css`. Content types `text/html; charset=utf-8`,
  `text/javascript; charset=utf-8`, `text/css; charset=utf-8`. The HTML
  carries `<link rel="icon" href="data:,">` so no `/favicon.ico` request is
  made. Any other method on these is `405` with `Allow: GET`; any other
  path outside `/api/v1/` is `404` with the usual JSON error.
- **Headers on all three (and the 404/405):** `Content-Security-Policy:
  default-src 'none'; script-src 'self'; style-src 'self'; connect-src
  'self'; img-src data:; base-uri 'none'; form-action 'none';
  frame-ancestors 'none'`, `Cache-Control: no-store`,
  `Referrer-Policy: no-referrer` (the URL with the code must not leave in
  a `Referer`), `X-Content-Type-Options: nosniff`, `Connection: close`.
  The page has no inline script, no inline style and no `style=` attribute,
  and no external URL. Whether WebKit treats `connect-src 'self'` as
  covering `wss://` on the same host is not in the facts; the first
  manual run on Safari records it, and the polling path is why a "no" does
  not break the page.
- **Who answers:** the connection thread (`connection`,
  `src/adapters/remote.rs:770`), before the request is handed to the
  frame, because the bytes need no core state and §4.1 says the frame does
  core state only. The bytes are `include_str!("../../web/index.html")`,
  `app.js`, `app.css` in `adapters/remote.rs`. No build step, no
  dependency.
- **Pool and rate window.** A page load counts against the 4
  unauthenticated connections like any stranger (§4.1; unchanged), and
  `Connection: close` frees the slot as soon as the file is sent, so the
  three files do not hold three slots for 30 s of idle (Unresolved 8: how
  many parallel connections iOS Safari opens needs a device). Static paths get
  their own `RateWindow` in `Limits` (`:623-639`), 60 per minute per
  listener, `429` past it, so a page-load loop cannot be charged to the
  `/pair` window (6/min) or the `/info` window (60/min). The page's own
  `/info` and `/pair` calls count against theirs as today.
- **Before pairing the listener serves `/info`, `/pair` and the page.**
  §5's "Require pairing before anything but `/info` and `/pair`" is
  amended to add the page. The page's bytes are identical on every
  installation and carry no host data (no id, no key, no name): the
  host-specific values arrive only in the URL the owner scanned and in
  `/info`.
- **Opened with neither code nor marker:** a plain message: "Open
  Remote devices in Switchboard on the Mac, choose Allow pairing, and scan
  the code with this device's camera." No code-entry form in this cut: a
  typed code without the scanned key would skip the one check the page
  can make. A paste field is the natural follow-up.
- **`refuse` and the code.** The page request never reaches the frame, so
  `refuse` (`src/app.rs:3754`, logs method and path only) is not on the
  path; the static arm logs nothing and never logs the query.

### The page's protocol (`web/app.js`)

Order on load: (1) if `location.search` has `code`, hold `code`, `key`,
`id` in variables and call `history.replaceState(null, "",
location.pathname)` as the first statement; (2) `GET /api/v1/info`;
(3) with a code: refuse to continue unless `info.spki_sha256 === key` and
`info.server_id === id`, else say "this is not the host whose code was
scanned" and send nothing; (4) `POST /api/v1/pair {code, device_name,
spki_sha256, transport: "cookie"}`; on success store the marker and
continue, on `403 pairing_closed|wrong_code|wrong_key` say which, plainly;
(5) with no code and a marker: if `info.server_id` equals the marker's and
the key differs, "identity changed, pair again" and send nothing; (6)
`GET /api/v1/device` for the name and current scope; a 401 is the revoked
or never-paired state above.

After that it speaks §4.2 and §4.4: every reply is the envelope
`{epoch, revision, body}`; one held revision per resource key (a project's
session list, one session, one session's `conversation` or `screen`); a
reply is applied only at or past the held revision; a changed `epoch`
refetches everything. Subscribe first, then snapshot: open
`wss://<host>/api/v1/events`, wait for `hello`, fetch `projects` and each
`projects/{id}/sessions` (at most 4 in flight), then apply buffered events
by revision. `changed {session}` refetches that session and re-sorts;
`changed {sessions, project}` refetches that project; `changed {projects}`
refetches the list; `removed` drops it; `resync_required` reconnects and
refetches. Opening a session sends `watch {kind: "conversation", id}` for a
Claude session or `watch {kind: "screen", id}` otherwise, waits for
`watching`, then fetches `last-response` or `screen`; leaving sends
`unwatch`. Refetch the open view every 30 s regardless (§4.4, covers
eviction). Without the socket, every poll refetches the list and the open
view; the held-revision rule is the same.

A reply is `POST sessions/{id}/send {op, text}` under `control`, with the
op made per command. On a timeout the page asks `GET ops/{op}` and follows
§4.5: `done` uses the reply, `interrupted` shows the session's state and
never resubmits, `unknown` resubmits with the same op. A session with
`trust_question` shows "Waiting at Claude Code's folder trust question;
answer it on the Mac" and no box; one with `prompt_refusal` shows the box
disabled with that reason. Trust, resume, new session, the draft,
dictation, Dispatch tickets and the conversation beyond the last response
are not on the page (owner question; the API serves them).

### `GET /api/v1/device`

Authenticated, any scope. Reply, in the envelope: `{device_id, name,
scope, server_id}`, read from the core's current device, so a scope change
in the dialog shows on the next call. Handled in `serve_remote`
(`src/app.rs:3017`) after `authenticate` and before the allowlist match,
beside `events`, with no `wire::Body`. No fixture: Swift does not decode
it (the fixtures exist for Swift and Rust to agree, and nothing on the
Swift side reads this).

## The actions and effects

One new action, `AppAction::CertificateReissued { certificate: String,
names: Vec<String>, expires: SystemTime }`, handled in
`src/core/remote.rs` through the existing `update_identity`
(`:105-117`), which already emits `Effect::SaveIdentity` and does nothing
when read-only. `update_identity` changes only what its closure changes,
and the only place that sets `schema_version` is `make_identity`
(`src/adapters/remote.rs:118`), so the `CertificateReissued` closure also
sets `schema_version` to `IDENTITY_SCHEMA_VERSION`; without that an
upgraded `identity.json` would carry the new fields under version 1. No new effect, no new port method: `RemoteStore` already
saves the identity.

Adapter: `remote::reissue_certificate(identity, key_pem, names, now) ->
Result<(String, SystemTime), String>` beside `make_identity`
(`src/adapters/remote.rs:95`), over `KeyPair::from_pem`, with the SANs,
`serverAuth`, `not_before` a day back, `not_after` 800 days out; and
`remote::local_names()`, the sorted names above (the `.local` name from
`scutil --get LocalHostName`, absent when the command fails or off macOS,
so Linux CI compiles and passes). In `bind_remote` (`src/app.rs:2936`),
before `tls_config` at `:2957`: if `identity.certificate_names` differs
from `local_names()`, or `certificate_expires` is `None` or within 30 days,
call `reissue_certificate` with the Keychain key already fetched there,
dispatch `CertificateReissued`, and build the TLS config from the core's
identity afterwards. `make_identity` also fills the two new fields. An
address that changes while the port is running is not noticed until the
next bind; that is a known gap in the design status, not machinery here.

Serving the page is not an effect: it never leaves the connection thread.
The request-side changes (three headers, `set_cookie`, the Origin check)
are in the adapter and `authenticate`, with no action.

## The UI

**The pairing window** (`src/ui/remote.rs:155-215`). The `json!` payload
and `qr_texture` input become `core::remote::invite_url(...)`; "Copy
invite" copies the URL and its hover says "The QR code's link, to open in
a browser or paste into a client". The caption keeps the key and code and
gains the trust text from decision 1 (two sentences: the browser will warn
about this host's certificate; accepting it is the step that trusts this
network and does not verify the host; the key in the link only stops a
page from sending the code to a different Switchboard; the link works
once and expires in five minutes or three wrong codes). The paired-devices
section (`devices`, `:230`) gains one line under the table: a browser's
token is an HttpOnly cookie sent to any HTTPS service on this Mac's
address. It uses `GAP` and `PAD`; no logic beyond the format call.

**The page** (`web/index.html`, `app.js`, `app.css`), one file each, no
framework. A header with the host's name (from `/info`), the device's
name and scope word (from `/device`), and the live/polling indicator.
Below: a "Waiting on you" group first (every session with `waiting`,
marked, with `waiting_reason`), then one group per project; each row has
the session name, kind and `card` word (no excerpt, see the departures). A session view with
the last response (Claude) or screen text (anything else), refreshed on
`changed`, and the reply box under `control`, absent for `read`. States,
each one sentence: not paired, pairing closed, wrong host, identity
changed, no longer paired, host unreachable, live updates unavailable,
`read` scope. Text only, `textContent` never `innerHTML`, since session
text is attacker-influenced.

## The tests

CI cannot drive a browser and the tree has no JS runner (the issue rules
out a dependency), so the page's reconcile logic is checked by hand. The
host's side is tested in full.

- **`src/core/tests.rs`:** `invite_url` for an IPv4 and an IPv6 address;
  `CertificateReissued` updates the identity, sets its `schema_version`
  to `IDENTITY_SCHEMA_VERSION` (starting from an identity at version 1),
  and returns one `SaveIdentity`; it is a no-op read-only.
- **`src/adapters/remote.rs` unit tests:** `reissue_certificate` keeps
  `spki_sha256` (`spki_sha256_of_cert` equals the key's) and `tls_config`
  accepts it; `RemoteRequest::token` prefers the bearer, reads only the
  cookie with the right name, rejects an oversized value; the same-origin
  rule (equal, different port, different scheme, missing, case).
- **`tests/remote.rs`** (extend `http`, `:245`, to return the headers; keep
  the old signature as a wrapper so existing callers do not change):
  - the three paths answer unpaired with the exact `Content-Type`, the
    CSP above, `Cache-Control: no-store`, `Referrer-Policy`, `nosniff` and
    `Connection: close`; `/favicon.ico`, `/web/app.js` and `/%2e%2e/` give
    404; `POST /` gives 405; `GET /?code=ABCDEFGH` serves the page and the
    frame sees nothing (the port's `poll` returns no request).
  - **The embedded page references only paths the host serves:** every
    `src`/`href` in the HTML is one of the three paths or the `data:` icon;
    no inline `<script>` body, `on…=` or `style=` attribute, no `http(s)://`
    URL at all; every `/api/v1/…` literal in `app.js` (a `${…}`
    replaced by the id of a session seeded in the core, and `ops/{op}` by
    a fresh op id) is on the allowlist: it never answers the
    `"not available on this port"` refusal (`src/app.rs:3108-3114`). A
    missing-record `404 not_found` (`:1856-1861`) is not the signal, which
    is why the ids are seeded.
  - **Rate window and pool:** 61 static GETs in a minute give a 429 on the
    61st, and `/pair` still answers; ten sequential page loads leave no
    unauthenticated slot held, so a pairing right after them works. The
    existing `stalled_strangers_fill_their_own_pool_and_never_a_paired_clients`
    (`:3226`) stays as is.
  - **Cookie transport:** `/pair` with `transport: "cookie"` returns no
    `token` and a `Set-Cookie` with each attribute named above and the
    per-`server_id` name; the cookie alone then reads `waiting` and
    `device`; a `send` with the cookie and no `Origin`, or with another
    origin, is `403 forbidden_origin` and sends nothing; with the matching
    `Origin` it goes; an upgrade with the cookie and a wrong `Origin` is
    403; a bearer request with no `Origin` is unchanged. Pairing without
    `transport` returns the token as today.
  - **`/device`:** returns the name and scope; after `SetDeviceScope` the
    next call shows the new one; after revoke the cookie gets `401
    unauthorized` and the socket closes (beside
    `revoke_closes_the_devices_socket`, `:2892`).
  - **The pairing flow as the CLI does it:** parse `invite_url`'s output,
    GET `/info`, compare the key, POST `/pair`, read `/device` and
    `/projects`; this is the acceptance flow minus the browser.
  - **The reissued certificate:** a rustls client that trusts the
    certificate as its own anchor completes a handshake for `ServerName`
    `localhost` and for the IP SAN; the existing pinned config connects
    unchanged (the pin did not move). If webpki refuses a self-signed leaf as
    its own anchor, the implementer asserts the SANs another way (a dev
    dependency that decodes the DER, named in the commit) rather than
    dropping the test.
- **`tests/ui.rs`** (beside `remote_devices_dialog_opens_pairing_and_revokes_a_device`,
  `:7424`): with a pairing window seeded, the dialog shows the trust text,
  the key and code, and "Copy invite" with `record_actions` copies a text
  that starts `https://` and carries `code=`.
- **Swift** (`client/Tests`): `PairingInvite.parse` of a URL (IPv4, IPv6
  with brackets, missing `code`), of the old JSON, and of garbage; the
  CLI usage text. Run `swift test` in `client/` where the toolchain is
  present.
- **By hand, once, recorded in the design status:** the issue's
  acceptance on a phone: scan, accept the warning, see the sessions, a
  session starting to wait moves to the top without a reload, a reply
  arrives, revoke in the dialog and the next request says so. Order:
  (1) the cookie test of decision 2 first; (2) whether `new WebSocket`
  connects (Owner A); (3) the pool during the page load (Unresolved 8);
  (4) `connect-src 'self'` and `wss`. With
  `SWITCHBOARD_DATA_DIR` short, `SWITCHBOARD_REMOTE_PORT` set, and the
  script lines `remote-listen lan <port>` and `pairing <code>` (no new
  script line is needed).

## Order of work and docs (one commit; the app runs at each step)

1. `RemoteRequest`/`RemoteReply` fields, `token`, the Origin rule,
   `/device`, `transport: "cookie"` (native paths unchanged).
2. Static paths on the connection thread, the rate window, the 404 for
   strays; `web/` files as a stub page that loads.
3. `Identity` fields, `reissue_certificate`, `bind_remote`.
4. `invite_url`, the dialog, `PairingInvite.parse`, the CLI usage text.
5. The page.
6. Docs: `docs/remote-client.md` §4.1 (static pool and window,
   `Connection: close`), §4.3 (table rows for the page and `/device`), §5
   (QR is a URL, the browser trust step and what the key check does not
   prove, the cookie, the Origin rule, the reissue); `docs/design.md`
   "Remote client" status (what is built, plus the gaps: address change
   while running, no list excerpt, no code-entry form, no `.local` URL, polling when `wss`
   fails, the manual run's findings); `README.md` Layout rows for `web/*`
   and the changed rows for `remote.rs` (core, adapters, ui) and
   `tests/remote.rs`, the dev-aid text for "Copy invite" (now a link; the
   CLI still takes it) in the `SWITCHBOARD_REMOTE_PORT` entry;
   `client/README.md` for the CLI's argument; `CHANGELOG.md` Unreleased:
   "The remote port serves a web page: scan the pairing window's QR code
   with a phone to see the sessions, which are waiting, and to reply."

## For the owner (follow-ups, not in this commit)

- **A (Safari, `wss`):** if the phone run shows `wss://` fails after the
  warning, install-the-certificate (a download route or a profile) is the
  follow-up. It is a second trust mechanism and changes §5.
- **B (`.local`):** if `https://<name>.local:<port>/` resolves from the
  phone, put it in the URL. The certificate already carries it.
- **D (cookie):** if the cookie fails on `fetch` in Safari with an IP
  host, the fallback is a follow-up decision, not a second transport here.
- Cookie reach across ports on one host (Settled): is it acceptable, or
  should the page move to a `.local` name so the cookie is host-scoped to
  one name?
- The list excerpt: carrying `last_message` on the wire, masked as
  `docs/remote-client.md:201,207` requires, so the rows can show it.
- A code-entry form, and Dispatch tickets, trust, resume, new session and
  the draft on the page.

## Commit message

`Serve a browser page on the remote port: pairing by link, cookie token, a /device read, a reissued certificate`

## Contract additions

- `RemoteRequest` (adapter, not the shared contract): `cookie`, `origin`,
  `host` and `token()`; `RemoteReply`: `set_cookie` and `Default`.
- `GET /api/v1/device` → `{device_id, name, scope, server_id}` (spec §4.3).
- `POST /api/v1/pair` request field `transport` (`"cookie"`); reply omits
  `token` and sets `Set-Cookie: __Host-sb-<server_id>`.
- Error code `forbidden_origin` (403).
- Static `GET /`, `/app.js`, `/app.css` on the remote port (spec §4.1, §5).
- `Identity.certificate_names`, `Identity.certificate_expires` (both
  `#[serde(default)]`); `IDENTITY_SCHEMA_VERSION` = the value in
  `src/ports/remote.rs` at implementation time, plus one.
- `AppAction::CertificateReissued { certificate, names, expires }`; no new
  `Effect`.
- `core::remote::invite_url`; `remote::reissue_certificate`,
  `remote::local_names`.
- Pairing QR payload: an `https://` link replaces the JSON object;
  `PairingInvite.parse` (Swift) accepts both.
- No `model.rs`, `SessionView`, `control/` or fixture change.
