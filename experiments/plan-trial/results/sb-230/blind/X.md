# Plan: issue #230, a web control surface on the remote port

Worktree `/Users/sbgragent/.dispatch/worktrees/da9387bb` at `7fe8949`
(Release 0.4.1). Line numbers are from this tree. The facts file for
the ticket is the yardstick for every claim about browsers, Apple's
rules and the crates; a claim below marked *Settled* cites a line under
its Settled heading, one marked *Unresolved* or *Owner* says so where
the decision is made.

## Where this departs from the issue

- **The certificate warning is not enough; the certificate itself must
  change.** The issue assumes "the first visit shows a certificate
  warning the owner clicks through". *Settled:* the host's certificate
  has no subject alternative name, no extended key usage and a validity
  to the year 4096 (`src/adapters/remote.rs:102-110`, rcgen's
  defaults), and Apple's TLS rules require a SAN naming the server,
  `id-kp-serverAuth` and 825 days or fewer, with "connections to TLS
  servers violating these requirements will fail"; the vendor tracker
  shows the page loading past the warning and the WebSocket failing
  until a SAN was added. So the plan reissues the certificate from the
  same key with those three properties, which *Settled* says breaks no
  paired native device (both clients pin the SPKI hash and ignore
  names, dates and chain). The click-through stays the trust step; the
  certificate is made one a browser can be told to accept.
- **The card's one-line excerpt is not on the wire.** The issue's first
  cut lists "the one-line excerpt the card shows". `SessionView`
  (`control/src/lib.rs:637-688`) carries `card`, `waiting`,
  `waiting_reason`, `kind`, `liveness` and the timestamps, and no
  excerpt; spec §4.3 (`docs/remote-client.md:207`) describes a
  `last_message` field that exists nowhere in `control/src/lib.rs` or
  `src/core/model.rs`. The Mac derives its excerpt from the parsed
  transcript (`agent_excerpt`, `src/ui/cards.rs:558`). The first cut
  shows the state word and the waiting reason on the card and the last
  response in the session view; the excerpt field is a wire change
  named under follow-ups, and the spec line is corrected to say so.
- **Four unauthenticated connections do not fit one browser.** The
  issue takes §4.1's four as given and asks only which pool a page load
  counts against. *Settled:* Chrome and Firefox open up to six HTTP/1.1
  connections per host and the fifth stranger is dropped at accept
  (`src/adapters/remote.rs:567`, `699-706`); *Unresolved* whether
  Safari's figure is six, with the line "the pool of 4 is too small for
  one browser either way". The plan raises the unauthenticated pool to
  8 and the paired pool to 16 and amends §4.1.

Everything else in the issue's shape stands as written.

## The record

- **`Identity` gains two additive fields** (`src/ports/remote.rs:26-43`),
  both `#[serde(default)]`: `certificate_names: Vec<String>`, the
  subject alternative names the current certificate was issued for as
  text (the IP addresses and `localhost`), and
  `certificate_expires: Option<SystemTime>`, its `not_after`. They let
  the app decide "reissue or not" by comparing strings and a time, with
  no X.509 parser in the tree. An `identity.json` written before this
  reads with an empty list and `None`, which means "reissue at the next
  bind". Bump `IDENTITY_SCHEMA_VERSION` (`src/ports/remote.rs:17`,
  today 1) to the value in that file at implementation time plus one;
  the loader (`load`, `src/adapters/remote.rs:55`) has no version gate
  and none is added, since nothing but this app reads the file.
- **`PairedDevice` and `devices.json` are unchanged.** A browser is one
  more row with the name the page posts (decision 6), listed and
  revocable as today. The host does not read `User-Agent`: the page
  derives "Safari on iPhone" from `navigator.userAgent` and posts it as
  `device_name`, cut at 64 characters by `new_device`
  (`src/core/remote.rs:166-175`) as every name is.
- **Nothing in `src/core/model.rs` changes**, so no `SCHEMA_VERSION`
  bump and no `migrate` step.
- **The browser's own state** is the device's, as the native client's
  Keychain is (spec §7): the token in an `HttpOnly` cookie the page's
  script cannot read, and in `localStorage` only what is not secret:
  `server_id`, the pinned `spki_sha256`, `device_id`, `scope`, and a
  `paired` flag the page uses to tell "revoked" from "never paired",
  since `authenticate` answers both with 401 `unauthorized`
  (`src/app.rs:3127-3156`). The page itself is embedded at compile time
  and rehydrates by a rebuild, which is why nothing about it is cached
  across versions (below).

## The signals

### Trust in the certificate (decision 1)

The owner's click-through on the local network is the trust step, said
in the paired-devices dialog. Not the certificate download: trusting a
downloaded certificate on iOS is a configuration-profile install under
Settings, several screens deep, which is the "something to install" the
issue excludes, and it would still not spare the owner a warning when
the address in the URL is not among the certificate's names. What the
click-through buys and what it does not, in the dialog's words: the
`key` in the URL stops a genuine page from sending the code to a host
whose `/info` key differs; TLS stops a passive reader; and the browser
remembers the certificate it accepted, so a different certificate at
the same address warns again, which is the owner's signal. It does not
protect against an impostor at that address whom the owner clicks
through a second time: a page served by an impostor is not a genuine
page, and the browser attaches the cookie to every request to that
origin, `/info` included, so after pairing the token is only as safe as
the owner's refusal to accept a second warning. That signal holds only
if the host's own certificate at an address is stable: Chromium keys
the click-through to the certificate's fingerprint, so every reissue
would show the warning the dialog says means an impostor. So the
reissue policy below changes the certificate at an address only when
it must, at bind, and never while the port is up. The §5 rule stands
with one addition: a non-loopback bind serves nothing but `/info`,
`/pair` and the page's own files before pairing.

The certificate is made to pass a browser's checks (*Settled*, Apple's
rules):

- `remote::make_certificate(key, names, now)` in
  `src/adapters/remote.rs`, replacing the body of `make_identity`'s
  `CertificateParams::new(Vec::<String>::new())` (line 102): the SANs
  are the one address the URL carries (`local_address()`, 962-971, the
  outward route) plus `127.0.0.1`, `::1` and the DNS name `localhost`
  (`SanType::IpAddress` and `SanType::DnsName`, *Settled*: rcgen 0.14
  takes both); not every interface's address, so a VPN, a tether or a
  container bridge coming up changes nothing; extended key
  usage `ServerAuth`; `not_before` a day before `now`, `not_after`
  825 days after; the common name as today. `make_identity` calls it
  and fills the two new fields.
- `remote::reissue_certificate(identity, key_pem, names, now) -> Identity`:
  the same key (`KeyPair::from_pem`, as `tls_config` does at line 146),
  a new certificate, `server_id`, `server_name` and `listen` kept, the
  `spki_sha256` unchanged by construction.
- **When: at bind only, never from `Tick`.** In `bind_remote`
  (`src/app.rs:2936-2985`) before `tls_config`, which `load_identity`
  (2782-2857) reaches through `listen_remote` at start and
  `run_remote_effect` reaches on a listen change: reissue when
  `certificate_names` does not contain `local_address()`, or
  `certificate_expires` is `None` or within 30 days. The certificate is
  then stable for as long as the Mac keeps the address the owner was
  shown, and a browser sees one warning per address: a new address is a
  new origin anyway, where the host-only cookie and `localStorage` do
  not follow (*Settled*, a cookie without `Domain` is host-only), so
  the browser pairs afresh there and its one warning is the one the
  dialog's sentence covers. A Mac that changes address while running
  serves the old certificate until the next bind; a browser at the new
  address warns on the first visit, as it would for any new address,
  and the owner restarts the app or toggles the network checkbox to
  rebind if the warning is to name the address. *Unresolved* whether an
  IP SAN "survives" the Mac changing address: with this policy the
  answer only decides whether that one rebind is ever needed, and the
  identity's key never rotates either way, so no pairing is lost
  (*Settled*). A reissue is `AppAction::CertificateReissued` (below);
  its `Effect::SaveIdentity` writes the file and, since `listen` is
  unchanged, `run_remote_effect` (2890-2915) only re-advertises. No
  rebind rides on a reissue, so no paired device's socket or kept
  connection is touched by one; `remote_bound` (`src/app.rs:182`)
  stays `Option<Listen>`.
- `local_address()` (962-971), the outward route's address, is the one
  address the URL and the certificate carry; no other address is listed
  or named.

### The page's files on the port (decision 4)

- **Paths**: `GET /` (`index.html`), `GET /app.js`, `GET /app.css`,
  `GET /icon.svg`. A new module `src/web.rs` holds the table,
  `web::file(path) -> Option<(&'static str, &'static [u8])>`, with
  `include_str!("../web/index.html")` and the others from a `web/`
  directory at the repository root; no build step, no dependency.
- **Served by the connection thread, never the frame.** In
  `connection()` (`src/adapters/remote.rs:770-867`), after the rate
  check (788) and before `try_send` (808): a path outside `/api/v1/`
  is answered there, with the file, or 404 `not_found` for anything
  else (a browser asks for `/favicon.ico` and
  `/apple-touch-icon*.png` on its own), logged at `debug` by path, or
  405 for a method other than `GET`. A flood of page loads therefore
  never reaches the inbox of 64 (581) and never 503s a paired device.
  The frame's `strip_prefix("/api/v1/")` at `src/app.rs:3023` is
  unchanged and now unreachable from outside the prefix; the
  allowlist test (`tests/remote.rs:682`) still sees 404 `not_found`
  for `/api/v2/projects`. The request line of `GET /?code=…` carries
  the code, and nothing logs it: `read_request` splits the query off
  the path (495-503) and the connection thread logs the path alone.
- **Headers.** `respond` (530-552) takes a content type and extra
  headers, defaulting to `application/json` so every existing call is
  unchanged; `RemoteReply` (389-393) gains `headers: Vec<(String,
  String)>` for the frame's replies (`/pair`'s cookie, below), empty
  at its 28 literal sites in `src/app.rs`. The files answer
  `Content-Type: text/html; charset=utf-8`, `text/javascript;
  charset=utf-8`, `text/css; charset=utf-8`, `image/svg+xml`;
  `Cache-Control: no-store` on all four, which is the issue's "no
  caching across versions" with no version stamp to substitute (three
  small files per load on a local network); `X-Content-Type-Options:
  nosniff`; `Referrer-Policy: no-referrer`; and on the HTML
  `Content-Security-Policy: default-src 'none'; script-src 'self';
  style-src 'self'; connect-src 'self'; img-src 'self'; base-uri
  'none'; form-action 'none'; frame-ancestors 'none'`. No inline
  script, no inline style, no external resource. *Settled:*
  `connect-src 'self'` matches a same-host `wss://` in Chrome, Firefox
  and WebKit since April 2022; an older Safari fails the socket and
  the page falls back to the poll (below), so the scheme is not
  listed.
- **Pools and windows.** A page load counts against the
  unauthenticated pool, raised from 4 to 8 (`MAX_UNAUTHENTICATED`,
  567); `MAX_PAIRED` (568) from 12 to 16, since a browser's several
  connections each move to the paired pool on their first
  authenticated reply (835-849) and sit there until the 30 s idle
  (573) closes them. The files get no rate window: they are static
  slices answered on the connection thread before `try_send`, so a
  flood spends nothing past that thread, and the pool bounds how many
  threads there are. That answers the issue's "which pool and rate
  window": the unauthenticated pool, and no window.
- `/info` lists `web` among `capabilities` (`src/app.rs:3580`), so a
  client can tell a host that serves the page.

### The token and the WebSocket (decision 2)

The token travels as a cookie, not as a `Sec-WebSocket-Protocol`
value. Where a credential lives is a security decision: an `HttpOnly`
cookie is a store the page's script cannot read, `Sec-WebSocket-Protocol`
needs the token in script and so in `localStorage`, which any script
on the origin reads. *Settled:* the browser WebSocket handshake carries
cookies; a `Secure` cookie is sent over TLS with no rule about the host
being an IP; a cookie without `Domain` is host-only; `SameSite=Strict`
is sent on same-site requests; and setting it on the `/pair` fetch
reply rather than on a 101 avoids Safari's `HttpOnly`-on-101 report.
Nothing here sends a subprotocol, so `respond_upgrade` (519-526) is
unchanged (*Settled:* a browser fails the connection when it offered
one and none is echoed, which is one more reason not to use it).

- **`/pair` gains an optional body field `credential`.** With
  `"credential": "cookie"` the reply carries
  `Set-Cookie: switchboard_token=<token>; Path=/; Secure; HttpOnly;
  SameSite=Strict; Max-Age=34560000` (400 days, the longest Chrome
  keeps; the owner revokes before that or pairs again after) and the
  JSON body omits `token`, keeping `scope`, `server_id` and
  `device_id`. Without the field the reply is as today
  (`src/app.rs:3660-3673`), so the native clients and the `pair.json`
  fixture are untouched.
- **`read_request` keeps one more header** (`src/adapters/remote.rs:444-514`):
  `cookie_token: Option<String>`, the `switchboard_token` value out of
  `Cookie`. The struct is built in one place (506), so adding the field
  breaks nothing (*Settled:* the one test of it asserts method, path,
  query, bearer and body).
- **`authenticate`** (`src/app.rs:3127-3156`) takes the bearer first,
  else the cookie; the same 401s as today for neither or an unknown
  one. Cross-site requests are covered by `SameSite=Strict` alone
  (*Settled*: a Strict cookie is sent on same-site requests only). A
  revoked browser's stale cookie rides on its requests until the next
  `/pair` overwrites it, which costs one header the host ignores; the
  page clears its own `paired` flag on the 401. `remote_subscribe`
  (2723-2760) already takes the device from `authenticate`, so the
  events socket needs nothing more.
- Spec §5 line 312 ("sent only as a bearer header … never in a URL")
  is amended: as a bearer header, or as the `HttpOnly` cookie `/pair`
  set for a browser; never in a URL, a log, a record or the page's
  script.

### Pairing from the URL (decision 3)

One QR, a URL: `https://<address>:<port>/?code=<code>&key=<spki_sha256>&id=<server_id>`,
an IPv6 address in brackets (the dialog's `format!("{ip}:{port}")` at
`src/ui/remote.rs:157-159` leaves them out today; the Swift
`Address.init?` at `client/Sources/SwitchboardTransport/HostClient.swift:15-24`
already strips them). The three values are Crockford, hex and a UUID,
so no percent-encoding is needed on either side. *Settled:* the URL
form is 153 bytes against the JSON's 189 and encodes as QR version 9
against 10, so the code gets smaller. The JSON form leaves the QR and
"Copy invite"; nothing still needs it: every native consumer goes
through `PairingInvite.parse`.

- `Identity::invite_url(&self, address: SocketAddr, code: &str) -> String`
  in `src/ports/remote.rs`, pure, used by the dialog and the tests.
- `PairingInvite.parse` (`client/Sources/SwitchboardProtocol/Info.swift:113-115`):
  text beginning `https://` is read with `URLComponents` into
  `addresses: ["<host>:<port>"]`, `code`, `spkiSha256` from `key`,
  `serverID` from `id`; anything else decodes as JSON as today. The
  CLI (`client/Sources/switchboard-remote/main.swift:109-125`, usage
  line 12 says "invite url"), the headset's paste field
  (`BrowserView.swift:883`) and the `SWITCHBOARD_INVITE` dev aid
  (`BrowserView.swift:106`) all pass through `parse`, so one change
  covers them.
- **The page's handshake**, in order: read the query with
  `URLSearchParams`; `history.replaceState` to `/` before any request
  (*Settled:* it needs no secure context, only a same-origin URL);
  `GET /api/v1/info`; compare `key` with `spki_sha256` and `id` with
  `server_id`; on a match `POST /api/v1/pair {code, spki_sha256,
  device_name, credential: "cookie"}`; store `server_id`, the key,
  `device_id`, `scope` and `paired` in `localStorage`; load the
  sessions. On a mismatch the page says "This is not the host whose
  code was scanned" and posts nothing. The code is in no URL after
  `replaceState`, in no storage, and in no log on the host.
- **Addresses.** *Owner:* whether the one address `local_address()`
  picks routes to the phone. The URL and QR carry it, and the dialog
  shows the full URL in text; a list of every interface's address is
  the enhancement if the owner's phone proves the one address short,
  named under follow-ups. *Owner:* whether a version 9 code at
  4 px per module reads from the Mac's display; `QR_SCALE`
  (`src/ui/remote.rs:41`) stays 4 and the dialog's 620 holds 8 if not.
- A stale URL (scanned after five minutes) is answered
  `pairing_closed` by `remote_pair` (`src/app.rs:3615`) and does not
  count a failure; the page says "pairing is closed on the Mac; open
  it again".

### The page

Four files under `web/`, plain HTML, CSS and JavaScript, no framework.

- **States.** No cookie and no `paired` flag: "Pair this browser from
  the Mac's Remote devices dialog: open pairing there and scan the
  code, or type the address it shows." Code in the URL: the handshake
  above. Paired: the list. `/info` presents a key other than the stored
  one: "This host's identity changed; pair it again", and the page
  sends nothing further (the browser has already attached the cookie
  to `/info`; the dialog's trust sentence covers that). A 401 after
  `paired`: "This browser was revoked on the Mac; pair again", the
  flag cleared. The host unreachable: "retrying" with a backoff.
- **The list.** `GET /api/v1/projects` and
  `GET /api/v1/projects/{id}/sessions` per project (collections are
  answered whole, spec §4.2; without `scope=all`, so `hide_workers`
  applies, *Settled*), grouped by project, waiting sessions first then
  by `last_activity_ms` descending, each card with the name, the
  kind's word, the `card` state word and `waiting_reason`, a waiting
  card marked. The header shows `server_name` from `/info`, the
  device's scope word and a dot for live or polling.
- **The session view.** A `claude` kind reads
  `GET /api/v1/sessions/{id}/last-response` (`final_text`, `n`,
  `truncated`; the route at `src/app.rs:1967`); every other kind reads
  `GET /api/v1/sessions/{id}/screen?lines=60`. On open the page sends
  `watch {kind: "conversation" | "screen", id}` on the socket and
  `unwatch` on leaving, so the content worker polls the key; the
  content refetches on a `changed` for that key and every thirty
  seconds regardless (spec §4.4). A reply box posts
  `POST /api/v1/sessions/{id}/send {op: crypto.randomUUID(), text}`
  under `control` scope (the fields `remote_command` reads,
  `src/app.rs:3179-3192`), disabled while a send is in flight; a 403
  on it hides the box with "read-only now", since a scope lowered in
  the dialog takes effect on the next request and nothing on the port
  reports scope otherwise. The box is hidden from the start for a
  `read` device.
- **Live changes, and the poll behind them.** The page opens
  `wss://<same host>/api/v1/events` (same origin, the cookie carried),
  waits for `hello`, then snapshots and reconciles by revision as §4.4
  says. On `changed` with kind `projects`, `project`, `sessions`,
  `session` or `waiting` the collections are refetched, debounced
  200 ms; on `removed` the card goes; on `resync_required` or a close
  the socket reconnects with a backoff from one to thirty seconds.
  *Unresolved* whether Safari on iOS applies the certificate
  click-through to `wss://` once the certificate has a SAN. The default
  that works either way is built first: whenever the socket is not
  open, the page polls `GET /api/v1/waiting` every five seconds and
  compares the envelope's `revision` (every reply carries it, §4.2); a
  moved revision refetches the collections and the open session's
  content. The socket is the enhancement behind that poll: with it the
  change arrives at once, without it within five seconds, and the
  acceptance step "a session that starts waiting moves to the top
  without a reload" holds in both. *Owner:* whether the socket opens on
  the owner's phone; the header's dot says which mode the page is in.
- **The device name** from `navigator.userAgent`: "Safari on iPhone",
  "Safari on iPad", "Safari on Mac" (iPadOS presents as a Mac; accepted),
  "Chrome on Android", "Firefox on Mac", "Browser" otherwise.

## The actions and effects

- `AppAction::CertificateReissued { certificate: String, certificate_names: Vec<String>, certificate_expires: SystemTime }`:
  the core's `remote_action` (`src/core/remote.rs:21`) applies it with
  `update_identity` (105), which emits `Effect::SaveIdentity` as the
  name and listen changes do. No new effect, and `run_remote_effect`
  (`src/app.rs:2890-2915`) is unchanged: the listen did not move, so
  it saves and re-advertises without a rebind.
- The reissue itself runs in `app.rs` (it needs the key from the
  Keychain and the clock): a helper `reissue_if_needed(&mut self, key_pem)`
  called from `bind_remote` alone, after the key is read and before
  `tls_config`, dispatching `CertificateReissued` through
  `dispatch_inner` so the `ServerConfig` built next carries the new
  certificate. Nothing runs on `Tick`.
- `remote_pair` reads `credential` and sets the cookie header;
  `authenticate` reads the cookie; `remote_info` lists `web`.
- The adapter: `connection()` answers the files and the 404s;
  `read_request` keeps the cookie token; `respond` and `RemoteReply`
  carry headers; the two pool constants; `make_certificate`,
  `reissue_certificate`.
- No new script line: `remote-listen lan <port>` and `pairing <code>`
  (`src/script.rs:729-746`) already put a running instance in the state
  the page needs, and the URL is then on the dialog's clipboard.

## The UI

`src/ui/remote.rs` only.

- `pairing` (141-227): the payload at 160-166 becomes
  `identity.invite_url(SocketAddr::new(ip, port), &window.code)`;
  "Copy invite" (201) copies the URL; the QR texture is remade from it
  as today (167-173). Beside the QR: the spaced code and the short key
  as today, then the full URL as selectable text. The sentence at 194
  becomes: "Scan with a phone's camera to open the page in its browser,
  or paste the address into a client. The browser
  warns about this Mac's certificate once per address; accepting it on
  your own network is the trust step, and a second warning at the same
  address means another device answered. Grants {scope} scope. Open for
  five minutes or three wrong codes." The host section (89) gains a
  line `Certificate issued for <addresses>, until <date>` from the two
  new fields, so the owner can check what the browser was shown.
- `devices` (230-278): a browser is a row like any other. The dialog's
  closing text (72) gains one sentence: "A browser pairs like any
  device and is revoked here; its token is a cookie the page cannot
  read."
- `host` (89): the hover text at 113 becomes "…only paired devices get
  past /info and the page".

## The tests

- **Adapter unit tests** (`src/adapters/remote.rs`): `read_request`
  keeps the cookie token, beside the existing
  `a_request_reads_with_its_query_bearer_and_body` (1045);
  `make_certificate` is accepted for one of its addresses by a
  rustls client whose `RootCertStore` holds the certificate itself,
  connecting as `ServerName::IpAddress`, which is webpki's SAN, EKU
  and validity check, the one a browser makes short of trust; the
  same client refuses the certificate for an address not among its
  names; `reissue_certificate` keeps `spki_sha256` and changes the
  DER.
- **Core test** (`src/core/tests.rs`): `CertificateReissued` changes
  the certificate and the two fields, keeps everything else, and
  emits `SaveIdentity`; a read-only core emits nothing.
- **`tests/remote.rs`.** The `http` helper (245-297) returns the
  response headers beside the status and body (or a sibling
  `http_full` does, with `http` unchanged). Then:
  - `the_page_is_served_unpaired_with_its_headers_and_references_only_served_paths`:
    `GET /` with no token is 200 `text/html` with the CSP,
    `no-store`, `nosniff` and `no-referrer`; every `src=` and `href=`
    in the HTML is fetched and is 200 with its content type; `/x` is
    404 `not_found`; `POST /` is 405; `/api/v1/waiting` unpaired is
    still 401. A regular expression over the embedded HTML and the
    `fetch(`/`WebSocket(` string literals in `app.js` asserts that
    every path the page names is one the host serves (the four files
    and the `/api/v1/` routes in `serve_remote`), which is the issue's
    "references only paths the host serves".
  - `a_browser_pairs_from_the_url_and_reads_and_subscribes_with_the_cookie`:
    the host's own client, as the CLI does. Open the window, build the
    URL with `invite_url`, parse its query, `POST /pair` with
    `credential: "cookie"` and the query's key and code: the body has
    no `token`, the `Set-Cookie` carries `HttpOnly`, `Secure`,
    `SameSite=Strict` and `Path=/`; `GET /waiting` with the `Cookie`
    header and no bearer is 200; `GET /events` upgraded with the
    cookie answers `hello` (`subscribe_at`, 1960, gains a cookie
    variant); the device is listed with the posted name and the
    window's scope.
  - `a_revoked_browser_is_refused_and_its_socket_closed`: after
    `RevokeDevice`, the next cookie request is 401 `unauthorized` and
    the socket closed, as `revoke_closes_the_devices_socket` (2892)
    checks for a bearer.
  - `the_certificate_is_reissued_from_the_same_key_at_bind_when_its_names_or_expiry_say_so`:
    seed the fake store with an identity whose `certificate_names` is
    empty; at start the certificate changes, `spki_sha256` and
    `server_id` do not, the devices stay, `identity_rotated()` is
    false, and the pinned client still connects (so the native pin
    is untouched); a second start with matching names reissues
    nothing.
  - The bounds: `stalled_strangers_fill_their_own_pool_and_never_a_paired_clients`
    (3226) reads the new constants; a ninth stranger is dropped.
- **`tests/ui.rs`**: `remote_devices_dialog_opens_pairing_and_revokes_a_device`
  (7424) parses the clipboard text as a URL and asserts `code`, `key`
  and `id` from its query (7486-7492 today assert the JSON).
- **Swift**: `testInviteParsesTheQRText`
  (`client/Tests/SwitchboardProtocolTests/FixturesTests.swift:157`)
  keeps the JSON case and gains the URL case with an IPv4 and a
  bracketed IPv6 host, asserting `addresses`, `code`, `spkiSha256`
  and `serverID`.
- **Fixtures**: no view changes shape, so `client/Fixtures` is not
  rewritten; `the_client_fixtures_decode_on_both_sides` (1829) runs
  as is.
- **Not testable here, said in the dialog and the status**: the
  browser's warning and its click-through have no stand-in in the
  harness (every test client pins), and the acceptance steps run on
  the owner's phone under the signed bundle.

## Docs and status, in the same commit

- `docs/remote-client.md`: §0 one bullet for the page; §4.1 bounds
  (125: 8 unauthenticated and 16 paired of 24; a `page` window of 60);
  §4.3 after the table (195): the page's four paths, answered by the
  connection thread, and a correction that the excerpt is not yet on
  the wire (207); §5: line 306 adds the page's files, 309 the URL in
  place of the JSON and the browser's trust step, 312 the cookie; a
  new paragraph "The web page" under §5 with the CSP, the cookie, the
  `SameSite` rule and what the click-through does and does not buy.
- `docs/design.md`: a new entry **The web page (2026-10-22)** in the
  Remote client status before "Known gaps at the first merge" (3573);
  the gaps gain "the card's excerpt on the wire" and "a browser's
  socket under Safari is confirmed only on the owner's devices"; the
  Trust boundary sentence at 249-253 adds that a browser's token is an
  `HttpOnly` cookie the page's script never holds.
- `README.md`: the data-directory line for `identity.json` (78) names
  the certificate's addresses and expiry; the dev aid at 267-279 says
  "Copy invite" is now the URL, that the same URL opened in a Mac
  browser shows the page, and that `switchboard-remote pair
  "$(pbpaste)"` still pairs; Layout lines for `src/adapters/remote.rs`
  (383), `src/app.rs` (401), `src/ui/remote.rs` (423),
  `tests/remote.rs` (437) and `client/` (446) updated, and new lines
  for `src/web.rs` and `web/`. No new dependency to document.
- `client/README.md` 180-190: `SWITCHBOARD_INVITE` takes the URL or
  the JSON.
- `CHANGELOG.md` under Unreleased: "The remote port serves a browser
  page: scan the pairing QR with a phone to see the sessions, the
  waiting ones first, read a session's last answer or screen and send
  a reply; the host's certificate is reissued so browsers accept it,
  with no change to paired devices."

The app runs at every step: the adapter and page land before the
dialog changes, each with its tests.

## Follow-ups for the owner, not in this plan

- The card's one-line excerpt on the wire (an additive `SessionView`
  field, a fixture change on both sides); spec §4.3 line 207 describes
  it as built.
- A DNS name in the SAN and the URL (the Bonjour `.local` host name)
  would make the certificate and the browser's exception independent
  of the address, so decision 1's signal and address churn would stop
  pulling against each other. It is deferred, not dropped, because the
  URL would then depend on the phone resolving `.local` over mDNS
  (iOS does; Android browsers often do not), the Linux build has no
  advertiser to take the name from (`adapters/bonjour.rs` is a stub
  there), and the owner's one-address question is still open; the
  identity model and the pin are untouched by it either way.
- Counting strangers per source address rather than in one pool, if
  two browsers at once prove the pool of 8 short.
- Listing every interface's address in the dialog (`if-addrs` 0.15 is
  already in `Cargo.lock` through `mdns-sd`), if the owner's phone
  proves the one outward-route address does not reach it.
- The browser's `wss://` under Safari: if the owner's phone never
  opens the socket, the page runs on the poll; whether to pursue a
  trusted certificate on the device is the owner's call.

## Commit message

Serve a browser page on the remote port, paired from the QR's URL, with the token in a cookie and a certificate browsers accept

## Contract additions

- `ports::remote::Identity`: `certificate_names: Vec<String>` and
  `certificate_expires: Option<SystemTime>`, both `serde(default)`;
  `Identity::invite_url(&self, SocketAddr, &str) -> String`.
- `ports::remote::IDENTITY_SCHEMA_VERSION`: the value at
  implementation time plus one.
- `core::action::AppAction::CertificateReissued { certificate, certificate_names, certificate_expires }`.
- `adapters::remote::RemoteRequest`: `cookie_token: Option<String>`.
- `adapters::remote::RemoteReply`: `headers: Vec<(String, String)>`.
- `adapters::remote`: `make_certificate(..)`,
  `reissue_certificate(..)`; `MAX_UNAUTHENTICATED` 8, `MAX_PAIRED` 16.
- `web::file(&str) -> Option<(&'static str, &'static [u8])>` and the
  paths `/`, `/app.js`, `/app.css`, `/icon.svg`.
- The wire: `/pair` body field `credential` (optional, `"cookie"`);
  `Set-Cookie` on that `/pair` reply; `web` in `/info`'s
  `capabilities`.
- Swift: `PairingInvite.parse` accepts the URL form.
