# Plan: a web control surface on the remote port (#230)

The host serves a small browser page from the remote port. A phone, an
iPad or another Mac pairs from a URL that carries the code and then lists
the sessions, shows one, and sends a reply. One commit. The README and the
remote status section of `docs/design.md` are updated in it. The app runs
at every step.

"The facts" below is `facts.md` for this ticket. "Settled", "Unresolved"
and "For the owner" are its headings. Line numbers are from this worktree
at HEAD 7fe8949.

## Where this departs from the issue

- **"each with ... the one-line excerpt the card shows."** The wire has no
  excerpt. `SessionView` (`control/src/lib.rs:637`) carries `card`,
  `waiting`, `waiting_reason`, `liveness`, `quiet_secs`, `last_activity_ms`
  and `prompt_refusal`, and nothing else a card says. The excerpt is
  computed in the UI from `UiState` caches (`agent_excerpt`,
  `src/ui/cards.rs:558`, and the pane captions), which the core never
  holds (facts, Settled, "The tree"). The issue also says "no new read
  model unless the page cannot do without one", and the page can: a row
  shows kind, the card's word, the waiting reason and how long it has been
  quiet. The excerpt is a follow-up (end of plan), because it needs a core
  field that the UI and the port would share.
- **"the browser then trusts that certificate for that address."** The
  tree does not bear it out. The certificate has no `subjectAltName`
  (`CertificateParams::new(vec![])`, `src/adapters/remote.rs:95-100`), and
  a browser matches an IP host against SAN entries only (facts, Settled,
  "An IP host is matched against `iPAddress` entries"). Chrome-family
  browsers report a name error on top of "self-signed". Whether a proceed
  link is then offered is Unresolved 1. A click-through exception also does
  not reliably cover `wss://` on Safari and iOS (facts, Settled, "A
  click-through exception does not reliably cover `wss://`"). The plan
  therefore puts the address in the certificate, gives the page a polling
  path, and serves the certificate for download as an option that is
  untested on iOS. Click-through is the stated trust step, not an
  assumption the build rests on, and polling is the only iOS path the plan
  promises.
- **Spec lines the plan amends.** `docs/remote-client.md` §5 says the token
  is "sent only as a bearer header ... never in a URL", that pairing is
  required "before anything but `/info` and `/pair`", and that "every
  later request ... the host answers with the device's scope". The first
  two change because the issue asks for them (cookie transport, the page
  served before pairing). The third is not true of the tree: `envelope`
  (`src/app.rs:3730`) carries `epoch`, `revision` and `body` only, so the
  page has no way to learn its scope. That is the one new read the plan
  adds, `GET /api/v1/device`.
- **The issue's `https://<address>:<port>/` with `local_address()`.** The
  address comes from a connected IPv4 UDP socket (`src/adapters/remote.rs:962`),
  so it is never IPv6 and never needs brackets today. The URL builder
  brackets an IPv6 literal anyway, since the owner may reach the host
  another way (For the owner 2).

## The record

`workspace.json` and `views.json` are untouched, so `SCHEMA_VERSION`
(`src/core/model.rs:12`, 16 today) and `VIEWS_SCHEMA_VERSION`
(`src/core/model.rs:612`, 7 today) do not move.
`devices.json` is untouched. A browser is a `PairedDevice` like any other:
a name, a scope, the token's hash. The browser's copy of the token is an
`HttpOnly` cookie (below), so the host's store of hashes is still the only
other copy and rehydration is "reload the page, the cookie rides the first
request".

`identity.json` gets two fields.

- `Identity.addresses: Vec<String>` (`struct Identity`, `src/ports/remote.rs:26`,
  `#[serde(default)]`), the IP addresses and names the current certificate
  lists in its `subjectAltName`. It is how the host knows, without parsing
  its own DER, whether the certificate still covers the address a phone
  would use.
- `Identity.not_after: Option<SystemTime>` (`#[serde(default)]`), when the
  current certificate expires. `None` in an old file, which
  `refresh_certificate` treats as "expired" and reissues once.
- `IDENTITY_SCHEMA_VERSION` (`src/ports/remote.rs:17`, 1 today) becomes
  the value there at implementation time, plus one, since a ticket landing
  in parallel may take the next number first. `identity.json` has no
  `migrate` step and no reader checks the version, so `serde(default)` is
  the whole migration: an old file reads with `addresses` empty and
  `not_after` `None`, which the next bind treats as "not covered" and
  reissues once, which also replaces the year-4096 certificate.

**The certificate is reissued over the same key, never a new one.** The pin
is the SPKI hash and `Identity.spki_sha256` already says so (a certificate
"reissued over the same key still verifies", `src/ports/remote.rs`), and
the native `Pin` verifier ignores names and validity
(`src/adapters/remote.rs:183-200`). So adding a SAN changes no pairing and
clears no `devices.json`. This is not key rotation (§5), and
`identity_rotated` stays false.

- `remote::make_identity` builds the certificate with SAN entries
  `localhost`, `127.0.0.1`, `::1` and the current `local_address()` when
  there is one, and records them in `addresses`.
- **Both builders set an `ExtendedKeyUsage` of `id-kp-serverAuth` and a
  validity of 800 days from now.** Apple's page for TLS server
  certificates (https://support.apple.com/en-us/103769) says a certificate
  issued after 2019-07-01 "must contain an ExtendedKeyUsage (EKU)
  extension containing the id-kp-serverAuth OID" and "must have a validity
  period of 825 days or fewer", and that connections to servers that
  violate this fail. The page does not say whether a user-trusted
  certificate is exempt (facts, Unresolved 4), so the default is to meet
  it: the cost to the native clients is nil, since neither reads EKU or
  dates. rcgen's defaults are no EKU and a year-4096 `not_after`, so
  `CertificateParams::extended_key_usages` and `not_before`/`not_after` are
  set explicitly, and the doc comment on `make_identity` is rewritten to
  say what the certificate now is.
- New `remote::reissue_certificate(identity, key_pem, addresses)` signs a
  new certificate with `rcgen::KeyPair::from_pem` under the same
  parameters, asserts the SPKI hash is unchanged, and returns the identity
  with the new `certificate` and `addresses`. This makes `Identity`'s
  "reissued over the same key" literal.
- **Basic Constraints is not set.** Apple DTS says iOS will not offer a
  self-signed certificate in Certificate Trust Settings without it
  (https://developer.apple.com/forums/thread/720477), but a `CA:TRUE`
  certificate used as a TLS end entity may be refused by browsers. Neither
  is settled here, so rcgen's `IsCa::NoCa` stays and the question is the
  device test named under For the owner 1. Until that test passes, the
  download is offered as "may work, untested on iOS", never as the iOS fix.
- Address and expiry changes: `SwitchboardApp::refresh_certificate`
  reissues when `local_address()` plus the loopback names differ from
  `identity.addresses`, and also when the certificate has under 90 days
  left (it reads the date it wrote from a new `Identity.not_after`, so no
  DER parsing; see the record above). It
  runs at the start of `bind_remote` (`src/app.rs:2936`), and on `Tick`
  while the pairing window is open, which is the only time a phone needs
  the current address. A mismatch reissues over the key from the secret
  store (`identity.key_account()`), dispatches
  `AppAction::CertificateReissued(identity)`, and, outside `bind_remote`,
  calls `bind_remote` again so the listener serves the new certificate. A
  read-only instance does neither. Any live socket on the port drops on
  that rebind. A device must reconnect, which is the cost of a DHCP change
  and is the same as toggling the LAN checkbox today.

## The signals

No new event. The page follows §4.4 as the headset does.

New refusals the page and the native clients can see:

- `403 origin`: a request authenticated by the cookie whose `Origin` is
  missing (for a non-`GET` or an upgrade) or is not the host's own origin.
- `401 unauthorized` is unchanged and is the page's only "revoked" signal.
  The host cannot tell revoked from never-paired from rotated on a 401
  (`authenticate`, `src/app.rs:3127`), so the page decides the wording
  client-side. It remembers `{server_id, spki_sha256, server_name}`, not a
  credential, in `localStorage` (guarded by `try/catch`; it must render
  without it). A 401 with `/info` showing the same `server_id` and key says
  "this device was removed on the host, pair again". A different key or id
  says "identity changed, pair again" and sends nothing. A 401 with nothing
  remembered says "not paired".
- A socket that does not open within 4 s, or closes with no 401 behind it,
  moves the page to polling (see the UI) and says so on the page. This is
  the fallback the facts require whichever way Safari and iOS behave
  (Unresolved 2).

## The actions and effects

### Serving the page

The page is three files under a new root-level `web/` directory,
`index.html`, `app.css` and `app.js`. They are embedded with
`include_str!` in a new `src/adapters/web.rs`. `Cargo.toml` has only an
`exclude` for `vendor/egui_term` (line 91), so `web/` needs no list entry
and the bundle is unaffected (facts, Settled).

**Static paths are answered on the connection thread, before the frame.**
`docs/remote-client.md` §4.1 says the frame does core state only, and a
file is not core state. In `connection` (`src/adapters/remote.rs:770`),
after the rate check and before `requests.try_send`, a `GET` of `/`,
`/app.css` or `/app.js` is answered by `web::asset(path, host)`. Any
other method on those paths is `405`. Every other path outside `/api/v1/`
still reaches `serve_remote` and gets its existing `404 not_found`
(`src/app.rs:3023`). The exact set of paths outside `/api/v1/` is those
three and nothing else. A query on `/` is ignored by the host and never
logged (`read_request` splits it off, and `refuse` logs method and path).

Each asset answer carries:

- `Content-Type`: `text/html; charset=utf-8`, `text/css; charset=utf-8`,
  `text/javascript; charset=utf-8`.
- `Content-Security-Policy: default-src 'none'; script-src 'self';
  style-src 'self'; img-src 'self' data:; connect-src 'self' wss://<host>;
  base-uri 'none'; form-action 'none'; frame-ancestors 'none'`. There is
  no inline script and no inline style, and no external resource, so the
  HTML has no `style=`, no `on*=` and no `<script>` body. `<host>` is the
  request's `Host` header filtered to `[A-Za-z0-9.:\[\]-]`, because older
  WebKit does not match `wss:` against `'self'`. I have not verified that;
  the explicit entry is cheap and does no harm if it is unneeded.
- `Cache-Control: no-store`, so a new build never meets an old script.
- `X-Content-Type-Options: nosniff` and `Referrer-Policy: no-referrer`.
  The latter keeps a `?code=` URL out of any `Referer`; `replaceState`
  (below) deals with the history and says nothing of `Referer`.
- `Connection: close`, and the connection thread ends after the write.

**Pool and rate window.** A page load counts against the unauthenticated
pool of 4 (`MAX_UNAUTHENTICATED`, `src/adapters/remote.rs:567`), because
every connection is unauthenticated until a reply says otherwise
(`reply.authenticated`, line 810) and a static reply says `false`. That
pool is not enlarged: it is §4.1's bound. `Connection: close` returns the
slot at once, where a keep-alive would hold it for `IDLE_TIMEOUT` (30 s,
line 573) and let one browser starve a second device. The page is three
requests before its first `/info`, with at most two of them concurrent,
and the script keeps at most two API requests in flight until the first
authenticated reply arrives (Unresolved 5; the behaviour of a real
browser's parallel connections is for the implementer's manual run).
`Limits::limited` (line 631) gets a third window, `assets`, 120 a minute
for every path outside `/api/v1/`, so the unauthenticated static paths and
junk are bounded as `/pair` (6) and `/info` (60) are.

### Credentials: a cookie, not storage or a subprotocol

Decision 2. **The `/pair` reply sets an `HttpOnly; Secure; SameSite=Strict;
Path=/; Max-Age=34560000` cookie named `sb_session`, and the page's
script never holds the token.** This is the safer of the stores. A token in
`localStorage` or `sessionStorage` is readable by any script on the origin,
and one cookie-less XSS in a page that renders terminal text would hand it
over. An `HttpOnly` cookie cannot be read by script at all. A
`Sec-WebSocket-Protocol` value would have the script holding the token on
every load.

What the cookie costs, stated plainly: cookies are not isolated by port
(RFC 6265 §8.5; facts, Settled), so `sb_session` is sent to every other
HTTPS service the owner runs on the same IP, and one of them could replay
it against this port. The cookie is `Secure`, and RFC 6265 §4.1.2.5 and
§5.4 send such a cookie only over a secure channel, so a plain-HTTP dev
server on that IP never receives it. The cookie is bounded by the scope the owner picked
(the dialog defaults to read-only) and dies when the device is revoked. It
is the price of the choice and the dialog says so (UI). It is not
mitigated further here.

**The mechanics are Unresolved, so they are the first thing built and
tried.** The tree and the facts do not say whether browsers store and send
a `Secure` cookie, over `fetch` and over `wss://`, for an origin they
accepted by click-through (facts, Unresolved 2 covers the socket, nothing
covers the cookie). The implementer's first step (Build order, step 3) is
a real-browser check of exactly that: pair, reload, and the cookie rides
`fetch` and the socket. If it fails, the fallback is a follow-up ticket,
not a second transport in this commit. The page would show its "cannot
keep a login in this browser" state and nothing else would ship half-built.

Host changes (`src/adapters/remote.rs`, `src/app.rs`):

- `RemoteRequest` (line 360) gains `host: Option<String>`,
  `origin: Option<String>` and `session: Option<String>`. `session` is the
  value of the `sb_session` cookie only. `read_request` (line 444) drops
  every other cookie, because a cookie reaches this port from every other
  service on the IP and none of them is ours. As the bearer is, the cookie
  is taken out of the headers so it is never logged with them.
- `RemoteReply` (line 389) gains `headers: Vec<(String, String)>`, and
  `respond` (line 530) writes them, a `Content-Type` among them replacing
  the default. A constructor `RemoteReply::plain(status, body,
  authenticated)` stands for the current literal. Every construction site is converted to it.
- `authenticate` (`src/app.rs:3127`) takes the bearer when there is one and
  otherwise `request.session`, with a bearer winning over a cookie. When
  the credential is the cookie, the request must pass the origin test:
  any `Origin` present equals `https://<Host>`, and a non-`GET` or an
  upgrade also requires one. A failure is `403 origin`, logged by method
  and path only. This closes cross-site `POST .../send` and cross-site
  WebSocket hijacking, for which the tree has no defence today (nothing
  checks `Origin` or the POST `Content-Type`, `src/app.rs:3175`; facts,
  Settled). A bearer request needs none: a header cannot be forged
  cross-site. `Origin` is compared with a port, so another service on the
  same IP with the cookie cannot pass it.
- `remote_pair` (`src/app.rs:3606`) takes an optional body field
  `session: "cookie"`. With it, the reply omits `token` and carries
  `Set-Cookie`, and the request must pass the same origin test (a cookie
  is only ever set for a same-origin page). Without it, nothing changes
  for the native clients: the reply is `{token, scope, server_id,
  device_id}` as now. The device name is the body's `device_name`, which
  `new_device` trims to 64 characters (`src/core/remote.rs:169`). The host
  cannot see a user agent (not retained), so the page sends the name it
  makes from `navigator.userAgent`, for example "Safari on iPhone".
- New `GET /api/v1/device` (`serve_remote`, after `authenticate`) answers
  `{device_id, name, scope}`. It is additive on the allowlist, and
  `capabilities` in `remote_info` (`src/app.rs:3580`) gains `"device"`.

### The certificate for download

Decision 1. **Click-through is the trust step the dialog states, and the
host also offers its certificate**, because the two are not alternatives
the build can choose between: the first needs a favourable answer
(Unresolved 1, For the owner 1), and the second is the one route known to
work for `wss://` on Safari, where only a user-trusted certificate is
reported to (facts, Settled, WebKit 41419). Whether iOS lets the owner
trust this certificate at all is not settled (Basic Constraints, above),
so the download is an option, and polling is the path the plan promises on
iOS. The honest cost of
click-through is in the dialog and in `docs/remote-client.md` §5: the
browser pins nothing a page can check; the URL's `key` lets a genuine page
refuse a host whose `/info` key differs, but a page served by an impostor
on the local network does not check anything, and it can relay the code to
the real host in the five-minute window. That is weaker than the native
client's pin, which §5 builds the whole pairing around. The stronger path
is to trust the certificate on the device first, from a channel other
than the one being protected.

- New `GET /api/v1/certificate`, unauthenticated, rate-limited with
  `/info`'s window, answers the DER from `identity.certificate` with
  `Content-Type: application/x-x509-ca-cert` and
  `Content-Disposition: attachment; filename="switchboard-host.cer"`. It
  is public data every handshake already sends, so it adds no store and
  leaks nothing. It is inside `/api/v1/`, so the page paths stay the only
  exception to the allowlist.
- The page offers the link in its "live updates are off" state, worded
  "may let live updates work; not tested on iOS", and does not depend on it.

### The invite URL

Decision 3. **One form, a URL.** The QR and "Copy invite" carry
`https://<address>:<port>/?code=<code>&key=<spki_sha256>&id=<server_id>`.
All three values are URL-safe (Crockford letters and digits, hex, a
UUID), so nothing is encoded. A new pure function
`core::remote::invite_url(identity, code, address, port) -> String`
builds it, bracketing an IPv6 address. Logic stays out of `src/ui/`.

- `PairingInvite.parse` (`client/Sources/SwitchboardProtocol/Info.swift:114`)
  tries a URL first and then the JSON as before. A URL gives `addresses =
  [host:port]`, `spkiSha256 = key`, `serverID = id`, `code = code`. It is
  the one entry point for every consumer the facts list: `switchboard-remote
  pair` with one argument (`client/Sources/switchboard-remote/main.swift:107-125`),
  `ConnectionManager.pair(invite:)`, `boot(invite:)` and the headset's
  paste field (`client/Sources/SwitchboardUI/BrowserView.swift:849`). The
  three-argument CLI form is untouched. The JSON stays accepted, because
  `client/Fixtures/pin.json`, `pair.json` and about ten Swift tests use
  it, and an invite copied before an upgrade is still valid.

### What the page does at load

1. If `location.search` has `code`, `key`, `id`, read them into a local
   variable and call `history.replaceState(null, '', location.pathname)`
   as the first statement, before any request. The code is never stored.
2. `GET /api/v1/device`. A 200 goes straight to the sessions (the cookie
   rehydrated).
3. A 401 with a code in hand: `GET /api/v1/info`, require its `server_id`
   equal to `id` and `spki_sha256` equal to `key`, and only then
   `POST /api/v1/pair {code, spki_sha256: key, device_name, session:
   "cookie"}`. A mismatch says "this link is for a different host" and
   sends nothing. The host's own `wrong_key` check on `/pair`
   (`src/app.rs:3641`) still applies.
4. A 401 with no code: the not-paired state, with an eight-character code
   field. The page shows the short form of the key that `/info` returned,
   for the owner to compare with the dialog by eye, which is the headset's
   Bonjour path. The comparison is only as strong as that eyeball (the
   host says what it says), and the page says so in one line.

`pairing_open` in `/info` (`src/app.rs:3596`) lets the page say "pairing is
closed on the host" rather than a bare 403.

## The UI

### The page (`web/`)

No framework, no build step, no dependency. `app.js` is plain script with
no `innerHTML`: terminal text and agent replies are untrusted, so every
string goes in through `textContent`.

- **Header.** The host name (`/info.server_name`), this device's scope
  (`/device`), and a state word: "Live" when the socket is open,
  "Polling every 5 s" otherwise, with the `/api/v1/certificate` link beside
  "Polling" and one line on why.
- **List.** `GET /projects`, then `GET /projects/{id}/sessions` for each,
  with the default scope (never `scope=all`, which would add Dispatch's
  worker sessions). The page also reads `GET /waiting` and puts the waiting
  sessions in a group at the top, marked, then the projects, each with its
  waiting sessions first. A row shows name, kind, the card's word, the
  `waiting_reason` when waiting, and "quiet 4 m" from `last_activity_ms`.
- **Session.** For a Claude Code session, `GET /sessions/{id}/last-response`;
  for anything else, `GET /sessions/{id}/screen?lines=60`. On open it sends
  `watch {kind: "conversation" | "screen", id}` and refetches on the
  matching `changed`; on close, `unwatch`. A reply box appears only when
  `/device` says `control`, posts `/sessions/{id}/send {op, text}` with a
  fresh UUID for `op`, and shows `prompt_refusal` instead of the box when
  the session reports one. A `403 scope` re-reads `/device` and hides the
  box. A repeated post with the same `op` is the retry, so a dropped
  connection can resend safely (`src/app.rs:3159`).
- **Live and fallback.** The page opens `wss://<host>/api/v1/events`, waits
  for `hello`, then fetches. When the socket is not open, it polls
  `/waiting` every 5 s and refetches the list and the open session
  whenever the reply's `revision` has moved. It retries the socket every
  60 s.
- **States said plainly.** Not paired, link for a different host, identity
  changed, removed on the host, host unreachable, and pairing closed
  (above).

### The Mac's dialog (`src/ui/remote.rs`)

- The payload at line 160 becomes `invite_url(..)`. The QR, the text beside
  it and "Copy invite" (line 205) carry it.
- With `identity.listen.lan` false the QR is hidden, because the LAN
  address it names is not bound and nothing would answer (For the owner 2
  asks for exactly this). A line says "Turn on 'Allow devices on the local
  network' to pair a phone". The URL and "Copy invite" remain, with the
  loopback address, for a browser on this Mac and for the CLI.
- Under the code, one `meta_text` line: "A browser warns about this
  certificate the first time; accepting it trusts it for this address on
  that browser only. A device that can install the host's certificate
  (`/api/v1/certificate`) before pairing gets a stronger check; iOS is
  untested." Another on the radio choice: "A browser keeps its login in a
  cookie that other HTTPS services on this IP can also receive; prefer
  read-only for a shared device."
- The hover text at line 113 becomes "... only paired devices get past /info,
  the certificate and the pairing page".

## The tests

Core and adapter units:

- `src/core/tests.rs`: `CertificateReissued` replaces the identity, returns
  `Effect::SaveIdentity`, leaves `devices` alone and leaves
  `identity_rotated` false; `invite_url` for IPv4, IPv6 (bracketed) and a
  port other than the default.
- `src/adapters/remote.rs`: `make_identity` records its SAN addresses and
  the DER contains each (`[0x87, 4, ..octets]`, no parser dependency is in
  the tree) and the serverAuth OID bytes (`2B 06 01 05 05 07 03 01`), and
  `not_after` is at most 825 days out; `reissue_certificate` returns the
  same `spki_sha256`; `refresh_certificate` reissues at under 90 days left
  and an old file with no `not_after` reissues once;
  `read_request` keeps `sb_session` and drops other cookies, and keeps
  `Host` and `Origin`. `src/adapters/web.rs`: every `src=`/`href=` in
  `index.html` names a served path; no inline `<script>` body, `style=`,
  `on*=` attribute, `eval(`, `innerHTML` or absolute `http(s)://` URL in
  any of the three files; the CSP string; the content types.

`tests/remote.rs` over the real listener (with an `http_full` helper beside
`http` at line 245 that returns headers, the existing one dropping them):

- The three paths answer 200 with the headers named above, `POST /` is 405,
  `/other` is the JSON 404, and 121 requests in a minute draw a 429.
- A static reply closes its connection. With three stalled strangers
  holding slots, three page fetches in a row still answer.
- Cookie pairing: `/pair` with `session: "cookie"` and a matching `Origin`
  sets a cookie with every attribute and returns no token; the cookie then
  reads `/device`, and a `GET` with no `Origin` is accepted; a `POST send`
  with no `Origin`, or an `Origin` on another port, is `403 origin`; with
  the right `Origin` it types once per `op`. A bearer plus a wrong cookie
  uses the bearer; a wrong cookie alone is 401. A pair without `session`
  is unchanged (the existing pairing tests keep passing untouched).
- The socket: an upgrade with the cookie and the right `Origin` gets
  `hello`; with no `Origin` it is 403; revoking the device closes it and
  the next `GET` is 401. This extends `subscribe_at` (line 1960) with an
  optional cookie and origin header.
- `GET /device` answers `{device_id, name, scope}` and follows
  `SetDeviceScope`; `GET /certificate` answers a DER whose SPKI hash equals
  `/info`'s with no token; after `refresh_certificate` with a different
  address, a client pinned by the old `spki_sha256` still connects and
  `devices.json` is unchanged.
- `write_fixtures` (line 1587) adds `invite_url.txt`. A Swift test
  checks that `PairingInvite.parse` reads `invite_url.txt` and `pin.json` to the same
  fields. The Swift tests are run with `swift test` in `client/` where the
  toolchain is present, and the commit says so if it is not.

`tests/ui.rs`: with `lan` false there is no QR image and the hint line is
present.

Not tested by the suite, because no UI test drives a browser: the page's
JavaScript. The acceptance run below is the check, and the build order
puts the riskiest part first.

### Build order and acceptance

1. Headers channel, the `RemoteReply` constructor, `host`/`origin`/`session`,
   assets and the CSP, with their tests. Green with an empty `app.js`.
2. SAN, `refresh_certificate`, `/certificate`, `/device`.
3. The gate: a one-screen `index.html` and `app.js` that pair with the
   cookie, call `/device` and open the socket, tried on a real phone or a
   second Mac. This answers the cookie question above, so the rest of the
   page is not written before it.
4. The rest of the page, the invite URL, the dialog, the Swift parse.
5. Docs: `docs/remote-client.md` §4.1 (the connection thread answers three
   page paths, the new `assets` window), §4.3 (`/device`, `/certificate`),
   §5 (cookie transport and its port-blindness for other HTTPS services
   on the IP, the `Origin` rule, pairing from a URL, the weaker browser
   trust, certificate SAN, serverAuth EKU, 800-day validity and same-key
   reissue, the amended "before anything but" line); the "Remote client"
   section of `docs/design.md` (ends before line 3585); the README Layout
   table (`web/`, `src/adapters/web.rs`, the rows for `adapters/remote.rs`,
   `ui/remote.rs` and `tests/remote.rs`) and the "Copy invite" paragraph
   (≈ line 276); `client/README.md`; `CHANGELOG.md` under `[Unreleased]`
   (line 9).

Acceptance, as the issue words it, with the instance put in state by the
script rather than clicks (`SWITCHBOARD_SCRIPT` with `remote-listen lan
43187` and `pairing ABCD2345`, `SWITCHBOARD_DATA_DIR` and
`SWITCHBOARD_TMUX_SOCKET` pointing at short test paths): scan the QR with a
phone, accept the warning, see the sessions. Start a session waiting
(`switchboard-hook PermissionRequest` as in CLAUDE.md) and watch it move
to the top with no reload. Send a reply from the phone and watch it arrive.
Revoke the phone in the dialog and the page's next request says it was
removed. None of these steps needs the socket: with it blocked, the same
steps hold on polling, five seconds slower.

## For the owner, named so nothing waits on them

These are not built here (a facts line or an owner answer that would change
a spec decision is a follow-up):

1. Whether your own browsers show a proceed link for the certificate and
   whether the socket connects after it (For the owner 1), and whether iOS
   lets you trust the downloaded certificate with no Basic Constraints, or
   refuses it with `CA:TRUE` on a TLS leaf (a device test, the
   implementer's or yours). The plan works either way; the answer changes
   the dialog's and the page's wording only.
2. Which address you reach the host by (For the owner 2). The certificate
   lists the loopback names and the one outbound address. A `<name>.local`
   or a second interface is a SAN addition in `refresh_certificate`.
3. The card's excerpt as a field on `SessionView` (needs the core to hold
   what `UiState` computes today).
4. A "forget this browser" that clears the cookie (the cookie is `HttpOnly`,
   so only the host can). Revoking from the dialog is the way until then.
5. The cookie's reach to other HTTPS services on the IP, if the owner
   would rather have the token in `sessionStorage` behind a
   `Sec-WebSocket-Protocol` value. That trades port-blindness for
   script-readable storage.
6. Dispatch tickets, decisions, drafts and the longer conversation on the
   page, as the issue says.

Commit message: `Serve a web control surface on the remote port, paired from a URL`

## Contract additions

- `Identity.addresses: Vec<String>` and `Identity.not_after:
  Option<SystemTime>` (both `#[serde(default)]`) in
  `src/ports/remote.rs`, and `IDENTITY_SCHEMA_VERSION` bumped from the value
  there at implementation time, plus one.
- `AppAction::CertificateReissued(Identity)` in `src/core/action.rs`
  (answered with the existing `Effect::SaveIdentity`).
- `core::remote::invite_url` (pure) and `remote::reissue_certificate`.
- `RemoteRequest.host`, `.origin`, `.session`; `RemoteReply.headers` and
  `RemoteReply::plain`.
- Remote API: `GET /api/v1/device`, `GET /api/v1/certificate`;
  `POST /api/v1/pair` accepts `session: "cookie"` and then sets `sb_session`
  and omits `token`; `403 origin`; `"device"` in `capabilities`.
- Outside `/api/v1/`: `GET /`, `/app.css`, `/app.js`.
- Invite text: a URL (the JSON stays accepted by `PairingInvite.parse`).
- Fixture `invite_url.txt`.
