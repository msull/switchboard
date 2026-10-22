# A web control surface on the remote port (issue #230)

Line numbers are as they are in this worktree at 7fe8949. "Facts" is
`inputs/facts.md`: *Settled* lines are cited as such, *Unresolved* N and
*For the owner* X say so where a decision rests on them.

One rule runs through the plan: **the page works with REST polling, a
token the script holds, and a certificate warning clicked through. Each
thing that needs a favourable answer from a browser or a phone (the
events socket, the cookie, a SAN accepted on an IP) is built behind that
default, chosen at run time by a probe, never assumed.** The acceptance
steps hold on the default.

## Where this departs from the issue

- **"The one-line excerpt the card shows" is not on the wire.**
  `SessionView` (`control/src/lib.rs:636-688`) has no excerpt, and spec
  §4.3 (the `session.last_response` bullet) says the card's excerpt is
  `last_message` and "moves with `changed {session}`", which nothing
  exports. The record has `last_message` (Facts, Settled, the tree; the
  hook writes it at `src/bin/switchboard-hook.rs:37`). Plan: no change to
  `SessionView`, so no fixture change on either side. The list shows
  the card word and `waiting_reason`; for the **waiting** sessions only
  (at most eight at once), the page also reads `last-response` and shows
  its first line. Adding `last_message` to `SessionView` is the later
  fix if the owner wants it on every card.
- **"The device's scope in the header" has no source after pairing.** No
  endpoint returns it (`remote_info`, `src/app.rs:3568`, is
  unauthenticated and carries none), so a scope change in the dialog
  would stay invisible. Plan: an authenticated `GET /api/v1/device`
  answering `{id, name, scope}`, one allowlist arm in `serve_remote`.
- **"Revoked" cannot be told from "never paired" by the host's answer.**
  `authenticate` (`src/app.rs:3125-3153`) refuses both with
  `unauthorized`. Plan: no new code. The page keeps a non-secret marker
  of its pairing (server id, key, device id, name), so a 401 with a
  marker is "revoked or removed on the Mac, pair again" and without one
  is "open the pairing window and scan".
- **"Any path outside `/api/v1/` is refused"** is true
  (`src/app.rs:3023`) but on the frame, after the request was queued. The
  page's files are answered on the connection thread instead, never
  queued, which is also what §4.1 ("the frame does core state only")
  leans toward.
- **The certificate cannot be assumed to be clickable.** The issue says
  the owner clicks through a warning. `make_identity`
  (`src/adapters/remote.rs:95-127`) makes a certificate with no
  subjectAltName, a CN only, and a `not_after` in 4096; Apple's rules
  need a SAN, `serverAuth` and at most 825 days (Facts, Settled, Apple's
  TLS rules), and whether a click-through works at all on this
  certificate is Unresolved 1. Plan: reissue it over the same key (so
  `spki_sha256` and every native pin are unchanged; Facts, Settled, the
  tree) with SANs, `serverAuth` and a bounded validity. Unresolved 1–3
  say this is cheap and helps either way.
- **§5's sentence "the host refuses a pairing whose `spki_sha256` is not
  its own, so a relayed request … fails" does not hold for a browser.**
  A page cannot read the TLS certificate it talks over, so the key it
  compares is what the host's `/info` *says*, and under a relay the
  `/pair` body comes from the impostor's page (`remote_pair`,
  `src/app.rs:3639-3647`, compares what it is sent). The `key` in the URL
  catches a genuine page on the wrong genuine host (a second
  installation at that address, a rotated key); it does not catch an
  impostor. The spec says so (below).
- **`crypto.randomUUID` is secure-context only** (Facts, Settled, secure
  contexts), and Unresolved 4 leaves a click-through origin open. The
  page uses it only when `window.isSecureContext` and otherwise builds a
  v4 id from `crypto.getRandomValues`.
- **The Swift side is bigger than "takes the URL too".** `PairingInvite`
  is `Decodable`-only JSON (`client/Sources/SwitchboardProtocol/Info.swift:93-116`)
  and is used by `ConnectionManager`, `BrowserView`, `switchboard-remote`
  and eight test files (Facts, Settled, the tree). `parse` gains the URL
  form and keeps the JSON form; the struct and `addresses` as a list do
  not change.

## The record

No new record kind, no `model.rs` change: `SCHEMA_VERSION` stays 16
(`src/core/model.rs:12`) and `VIEWS_SCHEMA_VERSION` 7. A browser is one
more `PairedDevice` (`src/ports/remote.rs:96-107`) in the unversioned
`devices.json`, named by the page from its user agent ("Safari on iPhone",
cut to 64 by `new_device`, `src/core/remote.rs:166-175`) and sent as
`device_name`, so no `kind` field is added (decision 6). It is listed and
revoked in the dialog as any device is, and rehydrates as one
(`IdentityLoaded`).

**The one record change is `identity.json`.** `Identity`
(`src/ports/remote.rs:26-42`) gains, both `#[serde(default)]`:

- `certificate_names: Vec<String>`: the SANs the stored certificate was
  issued for;
- `certificate_not_after: Option<SystemTime>`: when it lapses.

`IDENTITY_SCHEMA_VERSION` (1 today, `src/ports/remote.rs:17`) is bumped
to the value in that file at implementation time, plus one; a ticket in
parallel may take the next number first. There is no migrate step
because `identity.json` has none (`RemoteFiles::load` just deserializes,
`src/adapters/remote.rs:55`): an old file loads with empty names, which is
what says "reissue". Written through `RemoteStore::save_identity`'s
atomic path, as now.

**The certificate** is reissued by a new `remote::reissue_certificate(identity,
key_pem, names, now)` beside `make_identity`: the key from
`KeyPair::from_pem` (as `spki_sha256_of_key` does at `:136`), the same
CN, SANs from `names`, `ExtendedKeyUsagePurpose::ServerAuth`, `not_before`
a day back, `not_after` 800 days out (under Apple's 825; Unresolved 3: if
the rule does not apply to a manually trusted leaf nothing is lost). The
names are `localhost`, `127.0.0.1`, `local_address()` when there is one,
and `<hostname -s>.local` (`default_server_name`'s command), so the
`.local` name is in the certificate whichever way For the owner B comes
out; the URL host stays the IP (Unresolved 9). Read the rcgen 0.14 source
under `~/.cargo/registry/src/*/rcgen-0.14*` for how `CertificateParams::new`
sorts a string into an IP or DNS SAN and what `not_before`/`not_after`
take; `time` is in `Cargo.lock` (0.3.55) but not a direct dependency.
`remote::certificate_is_stale(identity, wanted_names, now)` is the pure
test: names differ, no `not_after`, or fewer than 30 days left.

A reissue never changes `spki_sha256`, `server_id`, the Keychain item or
`devices.json`; paired native clients see nothing. A browser exception is
per address (Unresolved 7): an address change means another warning, and
the page and dialog say so.

## The signals

No new event, hook or poll on the host. The page consumes §4.4 as it is:

- **Default (polling).** While visible, the list is refetched every 5 s
  (`GET projects`, then `GET projects/{id}/sessions` per project; a
  `waiting` flag is on each `SessionView`, so a session that starts
  waiting sorts to the top on the next poll, with no reload) and an open
  session's content every 5 s. Each reply carries `{epoch, revision}`;
  the page holds the highest revision per resource and drops an older
  reply, and an `epoch` change refetches everything (§4.4, "Subscribe
  first, then snapshot").
- **Upgrade (events socket, Unresolved 1 and For the owner A).** The
  page opens `wss://<host>/api/v1/events` and, if it opens, follows §4.4
  exactly: wait for `hello`, snapshot, apply buffered events by
  revision, `watch {kind: "conversation" | "screen", id}` for the open
  session and fetch only after `watching`, resend watches after a
  reconnect, `resync_required` refetches all. With the socket live,
  polling slows to the spec's 30 s refetch and an event refetches at
  once. If the socket does not open, or closes and will not reopen after
  three tries, the page falls back to the 5 s poll and says "live updates
  unavailable", never an error. Closing a session view sends `unwatch`.
  The watch set is capped at 128 keys for the whole port (§4.1), so the
  page watches only the one open session.
- **Token on the socket.** A browser cannot set a header on
  `new WebSocket()`. The page offers the subprotocols
  `["switchboard.v1", "token.<token>"]` (base64url, so a legal RFC 6455
  token; Facts, Settled, subprotocol), and the host answers 101 with
  `Sec-WebSocket-Protocol: switchboard.v1` only: the client must fail if
  the 101 names a subprotocol it did not offer, and echoing the
  non-secret one means the token is never echoed. In cookie mode (below)
  the page offers `["switchboard.v1"]` alone.

## The actions and effects

**Core (`src/core/`).** `AppAction::CertificateReissued(Identity)` sets
`self.identity` and emits nothing (the app already saved the file).
`Effect::CheckCertificate` is pushed by `OpenPairing` so that the I/O
stays in `app.rs`: the core cannot ask for an address. Both go in
`src/core/remote.rs`, tests in `src/core/tests.rs` (`CertificateReissued`
sets the identity and returns no effects; `OpenPairing` returns
`[CheckCertificate]`; the existing `OpenPairing` tests at
`src/core/tests.rs:11878-11936` are checked for exact-effect assertions
and updated).

**App (`src/app.rs`).**

- `bind_remote` (`:2926`) calls `refresh_certificate` before `tls_config`:
  if `certificate_is_stale`, it reissues, saves with
  `services.remote.save_identity`, and dispatches `CertificateReissued`.
  It does not go through `Effect::SaveIdentity`, whose arm
  (`:2893-2910`) rebinds, which would recurse. The Keychain key is read
  there already (`:2941`).
- `Effect::CheckCertificate` (arm next to `SaveIdentity` in
  `run_remote_effect`, routed at `:680`): when the port is bound to the
  network and the certificate is stale for the current
  `local_address()`, call `bind_remote()`, which reissues. This is the
  only way the address changing under a running app is noticed; the
  rebind drops open connections, which happens only when the address
  really moved.
- `serve_remote` (`:3017`): a new arm `("GET", ["device"])` after
  `authenticate`, answering `{id, name, scope}`; `info` (`:3568`) lists
  `"device"` among `capabilities`. `client/Fixtures/info.json` is
  rewritten with `SWITCHBOARD_WRITE_FIXTURES=1`; the Swift `capabilities`
  is a `[String]` (`Info.swift:8`), so nothing there breaks.
- `authenticate` (`:3125`) reads the token through one reader: `bearer`
  (the header or the subprotocol token; both are held by script, so
  neither is ambient authority), else the cookie named
  `sb-<server_id, simple form>` from the raw `Cookie` header. A token that
  came from the cookie is accepted only when `Origin`, if present, equals
  `https://<Host>`, and for any non-GET request and for the upgrade
  `Origin` must be present and equal; otherwise `403 bad_origin`. A
  same-origin `GET` carries no `Origin` in browsers, which is why GET is
  the exception. SameSite alone is not enough: a service on another port
  of the same host is same-site (Facts, Settled, cookies are not
  isolated by port), and `remote_pair`/`remote_command` parse the body as
  JSON whatever its content type.
- `remote_pair` (`:3606`): when the body has `"cookie": true` the reply
  carries `Set-Cookie: sb-<id>=<token>; Path=/api/v1/; Max-Age=31536000;
  HttpOnly; Secure; SameSite=Strict`. A native client does not send the
  field and gets no cookie. The token is also in the body as now (the
  page needs it in the default mode).
- Nothing logs a query, a cookie or a token: `refuse` (`:3754`) logs
  method, path and code, and the page's files never reach it.

**Adapter (`src/adapters/remote.rs`).**

- `RemoteRequest` (`:360`) gains `cookie: Option<String>`,
  `host: Option<String>`, `origin: Option<String>`, `subprotocol: bool`
  (the client offered `switchboard.v1`); `bearer` also takes a
  `token.<t>` subprotocol value when no `Authorization` header is
  present. `read_request` (`:444`) is built in one place and no test
  constructs the struct (Facts, Settled, the tree), so only the adapter
  changes. `RemoteReply` (`:389`) gains `set_cookie: Option<String>`; the
  literal sites (grep `RemoteReply {` and skip the signatures; `src/app.rs`
  at about a dozen places and `:829` here) get `set_cookie: None`,
  mechanical. Neither type is in `model.rs`, `AppAction`/`Effect` or
  `ports/`, so CLAUDE.md's additivity rule does not name them (Facts,
  Settled); the change is additive anyway.
- `respond` (`:530`) takes the reply's `set_cookie` and adds
  `Cache-Control: no-store` and `X-Content-Type-Options: nosniff` to every
  JSON reply (private data; a browser must not keep it). `respond_upgrade`
  (`:519`) echoes `Sec-WebSocket-Protocol: switchboard.v1` when
  `request.subprotocol`.
- **The page, on the connection thread.** In `connection` (`:763`), after
  the rate check and before the request is queued, `web::page(method,
  path)` answers `GET` of exactly three paths and closes the connection
  (`Connection: close`, so a page load holds a slot of the pool of 4 only
  for the request itself; Unresolved 8 says the real count needs a
  device):

  | Path | Content-Type |
  |---|---|
  | `/` | `text/html; charset=utf-8` |
  | `/app.js` | `text/javascript; charset=utf-8` |
  | `/app.css` | `text/css; charset=utf-8` |

  Every other path outside `/api/v1/` (`/favicon.ico` included) and every
  other method on these three stays what it is today: the frame's 404
  `not_found`. The query on `/` is ignored and never logged. Headers on
  all three: `Cache-Control: no-store` (the page is compiled in and
  changes with the binary), `X-Content-Type-Options: nosniff`,
  `Referrer-Policy: no-referrer`, and
  `Content-Security-Policy: default-src 'none'; script-src 'self';
  style-src 'self'; img-src 'self' data:; connect-src 'self' wss://<Host>;
  base-uri 'none'; form-action 'none'; frame-ancestors 'none'`. The
  `wss://<Host>` source is added because older Safari does not let
  `'self'` cover `wss:`; it is the request's own `Host` when that is
  letters, digits, `.`, `-`, `:` and brackets, and dropped otherwise. No
  inline script, no inline style, no external resource.
- `Limits` (`:621`) gains a `page` window, 60 a minute (about twenty page
  loads), answered 429 past it, separate from `pair` (6) and `info` (60) so
  a page reload loop cannot use up `/pair`. Static replies are cheap and
  need no frame, but they still count in the pool of 4 while they are
  open; that pool is shared with the strangers and `/pair`, as §4.1 says,
  and the failed handshake behind the warning holds a slot up to 5 s
  already.
- The files are `web/index.html`, `web/app.js`, `web/app.css`, embedded
  with `include_str!` by a new `src/adapters/remote_web.rs` (the table
  above, `page()`, the header block). Plain JavaScript, no framework, no
  build, no dependency. `Cargo.toml` has no `include` list, so the new
  directory ships as-is.

**Pairing URL.** `src/ui/remote.rs:155-173` builds
`https://<SocketAddr>/?code=<code>&key=<spki>&id=<server_id>` instead of
the JSON, where `SocketAddr::new(local_address(), port)` brackets an IPv6
address by itself; with no address it falls back to `127.0.0.1`. Code,
key and id are Crockford, hex and a UUID, so nothing needs encoding. The
QR, the text beside it and "Copy invite" all carry this one string
(decision 3: one QR; the JSON form is no longer produced, and the Swift
parser keeps reading it for text copied from an older host).

**Swift.** `PairingInvite.parse` (`Info.swift:111`) tries the URL first
(`URLComponents`: `code`, `key`, `id`, host and port become the single
`addresses` entry) and the JSON second. `switchboard-remote pair` takes
either as its one argument (`main.swift:107-125` unchanged beyond the
usage text); the headset's paste field goes through the same `parse`.

**The page (`web/app.js`), step by step.**

1. First statement: read `location.search` into a local, then
   `history.replaceState(null, "", location.pathname)`. The code is
   never in the address bar after that. Whether the browser recorded the
   URL before the script ran is Unresolved 6; the plan assumes it did,
   and the code is single-use and gone in five minutes or three failures
   (Facts, Settled, the tree), so a recorded URL is dead after pairing.
2. `GET /api/v1/info`. With a code and key in the URL: if `info.spki_sha256`
   or `server_id` differs, show "this is not the host whose code was
   scanned" and send nothing. If the stored marker has this `server_id`
   with another key, show "identity changed, pair again", drop the token
   and send nothing.
3. `POST /api/v1/pair {code, device_name, spki_sha256: key, cookie:
   true}`. `device_name` is built from `navigator.userAgent` and
   `maxTouchPoints` (an iPad reports `Macintosh`). Errors are worded from
   the code: `pairing_closed`, `wrong_code`, `wrong_key`, `rate_limited`.
4. **Transport probe** (Unresolved 5, For the owner D). The token from
   the reply is held in memory. The page probes `GET /api/v1/device` with
   no `Authorization` header, then an events socket with no token. If both
   succeed, the page is in **cookie mode**: the token is dropped and only
   the non-secret marker is stored. Otherwise **token mode**: the token is
   stored in `localStorage` (per origin including port, unlike a cookie;
   readable by script on the origin, which has no third-party script, by
   the CSP) and goes in `Authorization` on `fetch` and in the subprotocol
   on the socket. The mode is stored with the marker. The host accepts all
   three transports whichever is chosen.
5. With neither a code nor a marker: a short page saying "open Remote
   devices on the Mac, Allow pairing, scan the QR code", the host's short
   key from `/info` in the dialog's grouped form, and a field for the
   eight-character code. This is the headset's typed-code flow (spec §5):
   the owner compares the key by eye with the dialog's Key line, and the
   page pairs with the key `/info` showed. With a marker and no code, the
   page goes straight to the list.
6. List: sessions grouped by project, waiting first and marked, each
   with kind, card word, `waiting_reason`, and the first-line excerpt for
   the waiting ones (departure 1). Header: `server_name` from `/info`, the
   device name and scope from `/device` (refetched on every poll, so a
   scope change shows within 5 s), the live/polling note.
7. Session view: `last-response` for a Claude Code session, `screen?lines=60`
   for anything else (spec §4.3: a non-Claude session answers "no
   conversation"). The reply box appears only for scope `control` and
   only when the view has neither `trust_question` nor `prompt_refusal`
   (those show the reason instead, and the trust answer and resume stay on
   the Mac: out of this cut). Send is `POST sessions/{id}/send {op, text}`
   with a fresh op id; on a timeout the page asks `GET ops/{op}` and
   follows §4.5 (`done` use it; `interrupted` never resend and show the
   state; `unknown` resend the same op). It clears the box only on
   success.
8. A 401 on any request: marker present, "revoked, or removed on the Mac;
   pair again" and the stored token is dropped; no marker, the step-5
   page. A 403 `bad_origin` is shown as a plain error.

## The UI

`src/ui/remote.rs` only:

- The pairing window draws the QR from the URL (`:155-173`); the
  "Copy invite" tooltip and the helper line are reworded: "Scan with a
  phone camera to open the page (a browser warns about the certificate
  the first time; check that the key on the page matches the Key line
  here), or type the key and code on the headset. Grants {scope} scope.
  The link works once and for five minutes at most."
- The paired-devices section gains the trust sentence (decision 1): "A
  browser trusts this Mac by the certificate warning you click through
  on the first visit. That encrypts the connection but does not prove
  who answered; the key on the page against the Key line above does.
  The certificate is renewed before it lapses with the same key, and
  moving to another network address asks again."
- A browser appears in the device list by its user-agent name, with scope
  and Revoke, as any device.

**Decision 1: the click-through is the trust step, not a certificate
download.** It buys an encrypted channel to whoever answers at that
address, plus the page's refusal to send the code to a host whose `/info`
key differs from the URL's. It does not authenticate the host against an
impostor serving its own page; only the owner's by-eye key comparison
does, as on the headset. A download would add a fourth thing served before
pairing, needs a profile and the root-trust toggle on iOS, and whether
Apple would accept this certificate that way is Unresolved 1 and 3. The
live-updates cost is on the table in Facts (Safari `wss` after a
click-through, WebKit bug 41419 and the code-server doc); polling is
the answer to it. If For the owner A comes back unfavourable and the
owner wants the socket on iOS, a later issue adds the download. §5's
"before anything but `/info` and `/pair`" becomes "… and the page's three
files", and no non-loopback bind serves anything else before pairing.

## The tests

`cargo test --locked --workspace` and clippy as in CLAUDE.md. No JS runner
is added (the issue rules out a dependency), so the page's logic is
checked by hand (below) and the host side by the tests.

`tests/remote.rs` (the `http` helper at `:245` keeps only the status and
`Content-Length`; a sibling returns the headers, and `subscribe_at`
(`:1960`) takes extra request headers):

- `the_page_is_served_to_anyone_with_its_headers`: the three paths
  unpaired, with the table's content types, the CSP string, `no-store`,
  `nosniff`; `/favicon.ico`, `/index.html` and `POST /` answer 404; the
  query on `/` changes nothing; answered with no `serve_pending` running
  (the connection thread, not the frame).
- `the_embedded_page_references_only_paths_the_host_serves`: every
  `/api/v1/…` string in `app.js` (collected from the `include_str!`
  text, ids replaced by seeded ones) is sent with a paired token and none
  answers `not_found`; `index.html` references only the three paths; no
  `http://` or `https://` URL, no inline `<script>` body, no `on…=` or
  `style=` attribute.
- `a_browser_pairs_over_the_hosts_own_client_as_the_cli_does`: `/info`,
  key compared, `/pair` with `cookie: true`: the `Set-Cookie` carries
  `HttpOnly`, `Secure`, `SameSite=Strict`, `Path=/api/v1/` and the
  server-id name; the token reads `/device` as `read` or `control`; a
  native pair (no `cookie` field) sets none; the code is single-use.
- `the_cookie_is_accepted_only_from_its_own_origin`: cookie-only `GET`
  with no `Origin` 200; with a foreign `Origin` 403 `bad_origin`; cookie
  `POST send` with no `Origin`, or a foreign one, 403 with nothing written
  to the fake host; with `Origin: https://<Host>` it runs; the bearer
  header needs no `Origin`.
- `the_events_socket_takes_the_token_from_the_subprotocol_or_the_cookie`:
  `["switchboard.v1", "token.<t>"]` upgrades, the 101 names
  `switchboard.v1` only and never the token; cookie + matching `Origin`
  upgrades; cookie without `Origin` is refused; the header path of every
  existing socket test is unchanged.
- `revoking_a_browser_refuses_its_cookie_and_token`: after `RevokeDevice`
  both transports answer 401 `unauthorized` and its socket closes (as
  `revoke_closes_the_devices_socket`, `:2892`).
- `the_page_window_is_its_own`: past 60 page requests a minute the page
  answers 429 and `/pair` still answers; `stalled_strangers_fill_their_own_pool_and_never_a_paired_clients`
  (`:3226`) passes unchanged.
- `device_answers_the_scope_it_has_now`: `/device` before and after
  `SetDeviceScope`; `GET /api/v1/device` unpaired is 401; `info` lists
  `device`. `the_client_fixtures_decode_on_both_sides` (`:1829`) passes
  with `info.json` rewritten.

`src/adapters/remote.rs` unit tests: `reissue_certificate` keeps the
`spki_sha256` and the `server_id`; its names, `serverAuth` and a validity
of at most 825 days are checked on the DER (`x509-parser` is in
`Cargo.lock` at 0.18.1; use it as a dev-dependency if it is only
transitive, or `rustls-webpki`'s name check); `certificate_is_stale` over
its four cases; an `identity.json` without the new fields loads; the
`read_request` parse of `Cookie`, `Host`, `Origin` and
`Sec-WebSocket-Protocol`, the bearer taken from a `token.` value, and a
request with all of them unset behaving as today; `respond` writes the new
headers and a `Set-Cookie` only when given.

`tests/ui.rs`: the pairing window shows an `https://` invite with `code=`,
`key=` and `id=`, the new helper line, and the trust sentence in the
devices section.

Swift (`swift test` in `client/`, run locally; Linux CI does not run it):
`PairingTests` gains the URL form with an IPv4 and a bracketed IPv6 host,
and the JSON form still parsing.

**By hand, recorded in the status section, not in CI.** Script the
instance with `SWITCHBOARD_SCRIPT` lines `remote-listen lan <port>` and
`pairing ABCD2345` and a short `SWITCHBOARD_DATA_DIR`, then open the URL
in a desktop browser: warning, sessions, a waiting session moving to the
top within 5 s, a reply arriving in the pane, Revoke and the page
saying so on its next poll. The iPhone run is For the owner A, C and D;
nothing in the commit waits on it, and the status section says plainly
that no phone run has been made.

## Docs, in the same commit

- `docs/remote-client.md`: §4.1 (the page's three files, answered on the
  connection thread, their own 60-a-minute window); §4.3 (a `GET
  /api/v1/device` row, and one line that `/`, `/app.js` and `/app.css` are
  served outside the allowlist); §5 (the pairing URL replacing the JSON
  invite, "before anything but `/info`, `/pair` and the page", the
  cookie, the subprotocol, the `Origin` rule, the corrected relay sentence
  for browsers, the certificate reissue over the same key).
- `docs/design.md`: a bullet "A web control surface (2026-10-22)" under
  "Remote client (2026-10-12)" (`:3107`) with what is built and the gaps:
  no phone run, polling is the proven path and the socket is a probe, no
  excerpt on every card, no Dispatch tickets or conversation beyond the
  last response, drafts, dictation, runs and pinned files, no trust or
  resume from the page, the cookie shared across ports of the host.
- `README.md`: Layout rows for `web/`, `src/adapters/remote_web.rs`, and
  the `tests/remote.rs` row; the dev-aid text at `:274-279` ("Copy
  invite" is now a URL; open it in a browser, or `switchboard-remote pair
  "$(pbpaste)"` still works). `client/README.md` for `pair`.
- `CHANGELOG.md` under Unreleased: the page, and that the invite is now a
  link.

## Order inside the one commit (the app runs after each)

1. `Identity` fields, the version bump, `reissue_certificate`,
   `certificate_is_stale`, `refresh_certificate` in `bind_remote`,
   `CertificateReissued` and `CheckCertificate`. Native pairing unchanged.
2. `RemoteRequest`/`RemoteReply` fields, the one token reader,
   `Origin` rule, `Set-Cookie`, the subprotocol echo, `/device`.
3. `remote_web.rs`, `web/*`, the page arm and its window.
4. The pairing URL in the UI and the Swift `parse`.
5. Tests and docs.

## Commit message

Serve a browser control surface on the remote port: a page embedded in the binary, paired from a link, with a reissued certificate over the same key

## Contract additions

- `Identity` (`src/ports/remote.rs`): `certificate_names: Vec<String>` and `certificate_not_after: Option<SystemTime>`, both `#[serde(default)]`; `IDENTITY_SCHEMA_VERSION` bumped to its value at implementation time plus one (1 today). `SCHEMA_VERSION` (16) and `VIEWS_SCHEMA_VERSION` (7) unchanged.
- `AppAction::CertificateReissued(Identity)` and `Effect::CheckCertificate` (`src/core/action.rs`).
- `RemoteRequest`: `cookie`, `host`, `origin`, `subprotocol`; `RemoteReply`: `set_cookie` (`src/adapters/remote.rs`, outside the shared-contract files).
- `GET /api/v1/device` → `{id, name, scope}`, and `"device"` in `info.capabilities`.
- `POST /api/v1/pair`: optional body field `cookie: bool`; with it, a `Set-Cookie: sb-<server_id>=<token>` (`HttpOnly; Secure; SameSite=Strict; Path=/api/v1/`).
- Refusal `403 bad_origin` for a cookie-carried token with a missing or foreign `Origin` (non-GET and upgrade) or a foreign one (GET).
- Token transports: `Authorization: Bearer`, `Sec-WebSocket-Protocol: token.<t>` (101 echoes `switchboard.v1`), and the cookie; all three accepted.
- Unauthenticated static paths `GET /`, `/app.js`, `/app.css`, with a 60-a-minute rate window, `Cache-Control: no-store`, `nosniff` and a CSP; `no-store` and `nosniff` added to every JSON reply.
- Pairing invite is the URL `https://<host:port>/?code=<code>&key=<spki_sha256>&id=<server_id>`; `PairingInvite.parse` reads it and still reads the JSON.
