## A web control surface on the remote port (#230)

The remote port serves one more thing: a page, embedded in the binary,
that pairs a browser from a URL, lists the sessions, shows a session's
last response or screen and sends a reply. The page speaks the protocol
the headset speaks (`docs/remote-client.md` §4.3 to §4.5). Line numbers
below are as they are in this worktree at 7fe8949. `facts.md` is the
facts file; `facts.md:N` is its line N.

The feature's basic behaviour does not wait on any unresolved fact. The
page works over polling if a WebSocket is refused, over a typed code if
a QR cannot be scanned, and over an installed certificate if the browser
offers no click-through. Each path that needs a favourable answer is
built behind that default and named where it is.

### Where this departs from the issue

- **"The one-line excerpt the card shows."** It is not on the wire.
  `SessionView` (`control/src/lib.rs:636-688`) has `card`, `waiting`,
  `waiting_reason`, `trust_question`, `prompt_refusal` and the stop and
  activity times, and no excerpt; `facts.md:24` finds no `last_message`
  anywhere. The list shows name, kind, the `card` word and
  `waiting_reason`. The excerpt is the owner's item 9 (`facts.md:58`):
  fetching `last-response` per listed session puts every read key in
  the watch set for a minute and can evict the headset's watches
  (`src/app.rs:1789-1795`, effect unmeasured), so the default fetches
  none. A `SessionView` field is a later, additive change.
- **"The device's scope in the header."** No endpoint returns it after
  pairing (`remote_info` is unauthenticated, `src/app.rs:3568-3600`; the
  scope exists only in the `/pair` reply), so a scope changed in the
  dialog would show stale. The page cannot do without it, so the plan
  adds `GET /api/v1/device` (below). The headset has the same gap and
  may use it later.
- **"'Revoked' said plainly."** `authenticate` (`src/app.rs:3125-3153`)
  answers `unauthorized` for no token and for an unknown one, so the
  code cannot tell revoked from never paired. No new code is added. The
  page keeps a non-secret marker (server id, device id, pinned key) and
  concludes: a 401 on `/device` with a marker, and `/info` still showing
  the marker's key, is "revoked"; a different key is "identity changed,
  pair again"; no marker is "not paired".
- **"Compares the key in the URL with the one `/info` presents."** This
  holds, and buys less than §5 says. A page cannot read the certificate
  it is served over, so the check proves only what `/info` claims. A
  relay forwards `/info` faithfully, and `remote_pair`'s own
  `spki_sha256` check (`src/app.rs:3639-3647`) is fed by the relayed
  page's body, so `docs/remote-client.md:309` ("a relayed request
  through a substituted certificate fails") does not hold for a
  browser. What the check catches is a genuine page on the wrong genuine
  host: a second installation at that address or port, or a rotated key.
  The spec is amended to say so; the integrity of the browser path rests
  on the certificate the device trusts (decision 1).
- **A click-through as the first step.** The issue assumes the owner
  clicks through a warning. The certificate has no `subjectAltName`, a
  CN only, and a `not_after` in the year 4096 (`make_identity`,
  `src/adapters/remote.rs:95-127`, comment at `:91-92`). Whether iOS
  Safari offers a click-through on that certificate for a bare IP is
  unresolved (`facts.md:35`), as is whether a click-through covers
  `fetch` and `wss://` (`facts.md:37`) and whether Apple's SAN and
  825-day rule binds a user-trusted certificate (`facts.md:42`). The
  plan issues a certificate inside Apple's limits so none of these
  matters.
- **`facts.md:28` says about 20 `RemoteReply` literals in `src/app.rs`.**
  `grep` finds 28 there and 2 in `src/adapters/remote.rs`. The plan adds
  no field to `RemoteReply`, so none is touched; the page files are
  answered on the connection thread.

### The record

No new record kind, no `model.rs` change: `SCHEMA_VERSION` (`src/core/model.rs:12`, 16 today) and `VIEWS_SCHEMA_VERSION` (`:612`, 7 today) stay. A browser is one more `PairedDevice` (`src/ports/remote.rs:96-107`) in `devices.json`, which has no version. Its name is the page's own `device_name` in the `/pair` body (`remote_pair`, `src/app.rs:3650-3655`; cut to 64 characters by `new_device`, `src/core/remote.rs:166-175`), such as `Safari on iPhone`, so the host needs no `User-Agent` and the device record gains no `kind` (decision 6: the name is enough). Scope is chosen in the pairing window as today and changed or revoked in the dialog.

The one record that changes is `identity.json` (`Identity`, `src/ports/remote.rs:26-42`). The key is untouched, so `spki_sha256` and every native pin hold (`docs/remote-client.md:308` already says a certificate reissued from the same key still verifies). The certificate is reissued over the stored key, with two additive `#[serde(default)]` fields so the app knows what it covers without parsing DER:

- `certificate_names: Vec<String>`: the IP addresses and `localhost` in the certificate's SAN. Empty means a certificate from before this change.
- `certificate_expires: Option<SystemTime>`.

`IDENTITY_SCHEMA_VERSION` (`src/ports/remote.rs:17`, 1 today) becomes the value there at implementation time, plus one, because a ticket landing in parallel may take the next number first. Nothing reads the version today (`grep` finds it written once, `src/adapters/remote.rs:118`), so the bump marks the shape and no migration step is needed: an empty `certificate_names` is what triggers the reissue. Readers accept the older file; an older build that rewrites `identity.json` drops the two fields and the next launch of this build reissues again, which costs nothing.

Rehydration: launch loads the identity as before (`load_identity`, `src/app.rs:2782`). `bind_remote` (`:2936`) then asks `ensure_certificate`: reissue when `certificate_names` lacks `local_address()` (`src/adapters/remote.rs:962`), the certificate expires within 30 days, or `certificate_names` is empty. The reissue loads the key from the Keychain (`KeyPair::from_pem`, `:138`), signs a new certificate with the same `self_signed(&key)` call (`:109`), and saves through the existing atomic path. It does not clear `devices.json` and does not set `identity_rotated`. The new certificate:

- SAN: the route-chosen IP (`local_address()`), `127.0.0.1`, `::1`, `localhost`. The IP-only URL is the default; `.local` is optional and unresolved (`facts.md:43`).
- EKU `serverAuth`; validity at most 825 days (`not_before` a day back, `not_after` 800 days out); P-256 as today (`facts.md:20`: ECDSA is unaddressed by Apple's page, RSA sizes only, so this is a device check, not a blocker).

`make_identity` (`:95`) uses the same builder for a new key, and its doc comment ("never expires in practice") is rewritten.

The page persists in the browser, under its origin only: `{server_id, device_id, token, scope, key, host_name}` in `localStorage`. Browser exceptions for the certificate persist per browser; whether they survive a relaunch or an address change is unresolved (`facts.md:40`), and the page does not depend on it: an unreachable host shows "cannot reach the host" with the certificate link.

### Decisions the issue asked for

**1. Trust step: the certificate is offered for download; the click-through is allowed, not relied on.** The pairing dialog says: install the host's certificate on the device first (its link and SHA-256 are in the dialog), or proceed past the browser's warning, which trusts whatever answered. Why: the click-through rests on `facts.md:35`, `:37` and `:42`, all unresolved, and `facts.md:50` says a SAN-bearing certificate within Apple's limits serves both. Installing makes the browser validate the very key the dialog shows; a click-through trusts whatever is at the address. What it does not buy: the download itself travels over the unvalidated channel, so the dialog shows the certificate's SHA-256 for the owner to compare with the device's install screen (that the install screen shows it is not in the facts file: a device check). If a browser refuses the connection with no bypass, the download link is behind the same refusal; the dialog copies the link, and a "reveal the certificate file" button, which would let the owner move it by other means, is not built (recorded gap). §5's rule is amended: nothing but `/info`, `/pair`, the page's three files and the certificate is served before pairing. The iOS install steps (the root-trust toggle) are not in the facts file; the dialog text is checked on a phone.

**2. Token carrier: a `Sec-WebSocket-Protocol` value; no cookie.** `new WebSocket(url, protocols)` takes no headers (`facts.md:9`). The page offers `["switchboard.v1", "bearer.<token>"]`. Tokens are 43 characters of base64url (`docs/remote-client.md:312`), which are valid `token` characters (`facts.md:11`), and `bearer.` plus them is a valid value. The host takes the token from `Authorization: Bearer` first, else from a `bearer.` offer, and answers 101 with `Sec-WebSocket-Protocol: switchboard.v1` only, never the token: a server may send only a value derived from the offer, and a browser fails the connection if protocols were offered and none returns (`facts.md:10`). `fetch` sets `Authorization` itself; only the socket needs the protocol. Why not a cookie: it rests on whether Safari sends `SameSite=Strict` cookies on a same-origin WebSocket handshake, unresolved (`facts.md:39`), whether `HttpOnly` hides the cookie is not fetched (`facts.md:46`), and cookies are not isolated by port (`facts.md:13`), so a token cookie would reach every HTTPS service on that host. The cost: the token sits in `localStorage`, readable by any script of the origin (`facts.md:18`). The CSP allows only `/app.js`, and the page writes every host-supplied string (session names, a response, a screen) with `textContent`, never `innerHTML`. A cookie carrier is a later enhancement behind a device check.

**Origin.** RFC 6455 §10.2 says the server should check it (`facts.md:12`). The subprotocol carrier has no ambient credential, so a foreign page has no token to ride. The default still checks one thing that needs no allowlist: a socket that offers a `bearer.` value, which only a browser does, must have an `Origin` equal to `https://` plus its own `Host` header, else 403 `bad_origin`. A request with no bearer-subprotocol, which is every native client, is unchanged, so the headset cannot be broken by an `Origin` it sends. Whether the owner wants an allowlist of other origins (a `.local` name beside an IP) is theirs; the same-origin rule is what the page needs.

**3. One QR.** The QR carries a URL: `https://<host:port>/?code=<code>&key=<spki_sha256>&id=<server_id>`, where `host:port` is `SocketAddr::new(local_address(), port).to_string()` so an IPv6 address is bracketed. "Copy invite" copies the same URL. `PairingInvite.parse` (`client/Sources/SwitchboardProtocol/Info.swift:93-116`) tries a URL first (`URLComponents`: `code`, `key`, `id`, `addresses` from the URL's host and port) and the JSON second, so a pasted old invite still pairs and the three-argument CLI form is untouched (`client/Sources/switchboard-remote/main.swift:12-13`, `:107-125`). `addresses` stays a list on the Swift side (`facts.md:29`); a URL fills it with one entry, which is all the QR carried (`src/ui/remote.rs:160-166`). The JSON is not written by the host any more.

**4. Static paths.** `GET` only, answered on the connection thread from bytes embedded with `include_str!`: `/` (`text/html; charset=utf-8`), `/app.js` (`text/javascript; charset=utf-8`), `/app.css` (`text/css; charset=utf-8`), and `/switchboard.cer` (`application/x-x509-ca-cert`, the identity's DER, handed to the listener at bind). Every other path, `/favicon.ico` included, and any other method on these paths, goes to the frame as today and is refused 404 `not_found` (`src/app.rs:3023`). Headers on all four: `Cache-Control: no-store` (the app and the page ship together, so no cross-version cache), `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`, `Connection: close`, and on the HTML and JS `Content-Security-Policy: default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'`. The query on `/` is split off the path by `read_request` (`src/adapters/remote.rs:495-503`) and the connection thread logs no request line, so the code never reaches a log; `refuse` logs method and path only (`src/app.rs:3754-3772`). Whether `connect-src 'self'` covers `wss:` in every Safari is not in the facts file: if it does not, the socket is blocked and the polling fallback below carries the page; the first device check settles it.

Pool and rate: a static answer carries `Connection: close` and the thread ends after writing, so a page load holds an unauthenticated slot only for the length of a request, however many parallel connections the browser opens (unresolved, `facts.md:41`). That is why the default does not depend on the pool of 4 (`MAX_UNAUTHENTICATED`, `src/adapters/remote.rs:567`) lasting. The static paths share one rate window of 120 a minute per listener beside `PAIR_PER_MINUTE` and `INFO_PER_MINUTE` (`:583-587`, `Limits::limited` `:629-640`), 429 past it. The `/info` and `/pair` calls the page makes keep the connection in the unauthenticated pool until its first authenticated request moves it (`:834-849`).

Opened with neither a code nor a stored token the page says to open the pairing window on the Mac, shows the host's key in the dialog's short form (`/info`), and offers a code field: the owner compares the key with the dialog and types the code, which is the headset's own typed-code path (`docs/design.md:3562-3572`).

**5. Where the page lives.** `web/index.html`, `web/app.js`, `web/app.css`, embedded by a new `src/adapters/web.rs`. No build step, no dependency. The JS logic has no runner in the tree and the issue rules one out; what CI checks is in Tests.

### Signals

No new event kind, no change to the revision counter. The page consumes `hello`, `changed`, `removed`, `watching` and `resync_required` as §4.4 defines them. New inbound signals are request headers the listener reads today only as the bearer: `Sec-WebSocket-Protocol`, `Origin` and `Host`. `RemoteRequest` (`src/adapters/remote.rs:359-372`; built literally only in `read_request`, `:506`; no test builds one) gains three additive fields: `bearer_in_protocol: bool`, `offered_protocol: bool` (the offer lists `switchboard.v1`), `cross_origin: bool` (`Origin` present and not `https://` plus `Host`). `bearer` is filled from the protocol offer when `Authorization` is absent. The token is never logged: the headers are not, and `read_request` keeps the value in `bearer` only.

The page follows §4.4's order: open the socket, wait for `hello`, fetch, apply buffered events whose `revision` is past a snapshot's, refetch the resource they name. On `resync_required` or a close it reconnects (1, 2, 4, 8 s, then every 15) and refetches everything. While the socket is not open it polls the lists every 5 s, so the page is live over polling if the socket is blocked (`facts.md:37`, `:39` unresolved), and "a waiting session moves to the top" holds either way. The page watches only the open session's key (`conversation` for a Claude Code session, `screen` otherwise, as `remote_content_route` keys them, `src/app.rs:1960-1967`), waits for `watching`, then fetches, and sends `unwatch` on close: at most one key per page, so it cannot press the 128-key cap.

### Actions and effects

`AppAction::CertificateReissued(Identity)`: the core replaces its identity (devices, `identity_rotated` untouched) and emits `Effect::SaveIdentity`, which already writes atomically and rebinds only when `listen` changed (`src/app.rs:2893-2905`), so it does not loop. `Effect::EnsureCertificate`: emitted by `OpenPairing` (`src/core/remote.rs:46-52`) unless the core is read-only, so the QR never points at an address the certificate does not cover. `app.rs` runs it through the same `ensure_certificate` that `bind_remote` calls; when it reissued and the port is bound, it dispatches `CertificateReissued` and rebinds (`bind_remote` rebuilds the TLS config). A rebind drops open sockets for a moment; paired clients reconnect (§6) and the page does too. `RemotePort::bind` (`src/app.rs:2969`, its only caller) gains the certificate DER for `/switchboard.cer`. A new port method is not needed: the keychain read and `save_identity` are existing ports with fakes.

Host router: `serve_remote` (`src/app.rs:3017`) gets one arm after `authenticate`: `GET device` answers `{id, name, scope}` for the calling device (read scope, any paired device; the envelope as the other reads). It is the page's scope refresh and its "am I revoked" probe, on connect and every 30 s. Routes of the new form are listed in `docs/remote-client.md` §4.3's table. No capability string is added: the page and the host ship together.

Pure additions in `adapters/remote.rs`: `pairing_url(identity, addr, code) -> String` (used by `ui/remote.rs`, so the test and the QR cannot drift), `reissue_identity(identity, key_pem, addresses, now)`, and a `Site` that maps a path to its bytes and headers.

### UI

The pairing window (`src/ui/remote.rs:140-215`):

- The QR (`qr_texture`) encodes `pairing_url`; "Copy invite" copies it.
- New lines under the key: the certificate link (`https://<addr>/switchboard.cer`) with a "Copy certificate link" button, and the certificate's SHA-256 in `short_hash` form (`:303`).
- The helper text is replaced with: install the certificate on the device first, or proceed past the warning, which trusts whatever answered; scan, or type the key and the code; grants {scope}; open for five minutes or three wrong codes.
- The paired-devices list is unchanged: a browser is a row with its name, scope, last seen and Revoke. The dialog's redaction note names the browser as a device whose token lives in the page's storage.

The page, in the order the issue lists:

- Header: host name, the device's scope (from the pair reply, refreshed from `/device`), and a connection dot (live or polling).
- Pairing, on load before anything else: read `location.search`, call `history.replaceState(null, "", location.pathname)` as the first statement (`facts.md:19`), hold the code in a local variable only; fetch `/api/v1/info`, refuse with "this is not the host whose code was scanned" if its `spki_sha256` or `server_id` differs from the URL's; else `POST /api/v1/pair` with the code, the URL's key and a `device_name` from the user agent (`Safari on iPhone`, fallback `Browser`); store the marker and token. Whether the code is in history before `replaceState` runs is unresolved (`facts.md:44`); it is single-use and the window closes on `Paired` (`src/core/remote.rs:65-66`, `facts.md:23`), so a leaked code is dead after the pairing.
- List: `projects`, then `projects/{id}/sessions` for each; sessions grouped by project, waiting ones first and marked, each with kind, `card` word and `waiting_reason`. Worker records stay hidden (no `scope=all`).
- Session view: the last response for a Claude Code session, the screen for anything else, refreshed on `changed`. A session with `trust_question` or a non-null `prompt_refusal` shows the reason and a disabled reply box; `resume` and `trust` are not on the page in this cut ("answer it on the Mac"), so none of them depends on an owner answer.
- Reply: hidden unless the scope is `control`. `POST sessions/{id}/send {op, text}` with an op `web-` plus 16 hex digits from `crypto.getRandomValues` (falling back to `Math.random`; `crypto.randomUUID` needs a secure context and whether a bypassed page is one is unresolved, `facts.md:38`; `web-…` is never the `<ticket>-<nonce>` form, `control/src/lib.rs:81`). On a timeout the page asks `GET ops/{op}` and follows §4.5: `done` clears the box, `interrupted` shows the session's state and does not resubmit, `unknown` resends the same op.
- States said plainly: "not paired", "revoked" (401 on `/device` with a marker, the key unchanged), "identity changed, pair again" (the key or id differs; nothing is sent, the token included), "cannot reach the host" (a failed `fetch` cannot tell an untrusted certificate from a down host, so the text names both and links the certificate), "read-only".

### Tests

Core (`src/core/tests.rs`, `Clock::at`, no sleeping): `CertificateReissued` swaps the identity, keeps devices and the rotated notice, and emits one `SaveIdentity`; `OpenPairing` emits `EnsureCertificate`, and in a read-only core emits nothing.

Adapter (`src/adapters/remote.rs`): the reissued certificate has the same `spki_sha256`; an `Identity` JSON from before the change loads with empty `certificate_names`; the certificate verifies with rustls's `WebPkiServerVerifier`, the certificate as its own root, for the IP and `127.0.0.1`, fails for another IP, and fails at 826 days past now (if the verifier API resists, the fallback is asserting the SAN bytes in the DER); `pairing_url` brackets an IPv6 address and carries the three parameters.

`src/adapters/web.rs`: the CSP has no `unsafe-inline` and no host; every `src=` and `href=` in `index.html` is one of the served paths; `app.js` and `app.css` contain no `http://` or `https://` literal; `app.js` marks its endpoint list between `// endpoints:begin` and `// endpoints:end`, one `METHOD /api/v1/…` per line.

`tests/remote.rs`, over the real TLS listener with fakes. `http` (`:245`) reads `Content-Length` only, so a sibling returns the status, the headers and the body, and `http` wraps it:

- `the_page_is_served_unpaired_with_its_headers`: the four paths answer 200 with the exact `Content-Type`, the CSP, `nosniff`, `no-store`, `Connection: close`; `/?code=X` answers the same page; `/favicon.ico` and `POST /` answer 404 `not_found`.
- `page_loads_free_the_pool_and_are_rate_limited`: with three stalled strangers (the model is `stalled_strangers_fill_their_own_pool_and_never_a_paired_clients`, `:3226`), ten sequential page loads all succeed; the 121st static request in a minute is 429.
- `the_certificate_download_is_the_identitys`: the DER equals the identity's, and its SPKI hash equals `spki_sha256`.
- `every_endpoint_the_page_names_is_routed`: each GET template in the page's endpoint list, filled from a seeded host, answers 200 with a paired token; `device` answers the scope.
- `a_browser_pairs_from_the_url_and_is_revoked`: build the URL with `pairing_url`, parse its query as the page does, compare the key with `/info`, post `/pair` with `device_name: "Safari on iPhone"`, read `/device` (read), raise to `control` in the core and read it again, send a reply and see it logged once per op, then `RevokeDevice` and see 401 `unauthorized` on the next request. This is the host's own HTTP client driving the page's flow, as the CLI does; no UI test drives a browser.
- `a_websocket_carries_its_token_as_a_subprotocol`: using the `tungstenite` client as `subscribe_at` does (`:1960`), which itself rejects a reply that echoes an unoffered protocol: the offer `switchboard.v1, bearer.<token>` with a matching `Origin` gets 101 with `Sec-WebSocket-Protocol: switchboard.v1` and never the token, then `hello`; a cross-site `Origin` gets 403 `bad_origin` and registers no subscriber; a bad token gets 401; a header bearer and no offer works as before and the 101 carries no protocol header; revoking closes a subprotocol socket (the neighbour is `revoke_closes_the_devices_socket`, `:2892`).
- The fixtures under `client/Fixtures` are unchanged, because no existing wire view changes. `/device` is new and gets a fixture only if the Swift client starts reading it.

UI (`tests/ui.rs`, extending `remote_devices_dialog_opens_pairing_and_revokes_a_device`, `:7424`): the pairing window shows "Copy certificate link" and the certificate line, and the QR texture is rebuilt when the code changes.

Swift (`client/Tests`, run with `swift test` on the Mac, outside the Linux CI): `PairingInvite.parse` reads the URL form, including an IPv6 host and a URL with the parameters in another order, and the JSON form still parses.

Not covered by CI, and said so in the status section: the page's own logic (query parse and `replaceState`, the revision reconcile, the polling fallback, the state texts). It is checked by hand against a test data dir with `SWITCHBOARD_SCRIPT` holding `pairing ABCD2345` (the existing script line, `README.md:278`, so no new script line), then on a phone for the acceptance run. The device checks, none run here: the click-through on a SAN-bearing certificate; `fetch` and `wss://` after it; the installed-certificate path; `connect-src 'self'` on `wss:`; how many connections Safari opens; the exception across relaunch and an address change. The status section records each as run or not run.

### Docs and order

One commit; the app runs after it, and every step inside it keeps `cargo test` green in this order: the `Site` and its headers and the three `RemoteRequest` fields; the certificate reissue and its action and effect; `/device`; the pairing URL, the dialog and `PairingInvite.parse`; the page files; docs.

- `docs/remote-client.md`: §4.1 bounds (the static window, `Connection: close`), §4.3 table (the four paths and `device`), §4.4 (the subprotocol carrier and the same-origin rule), §5: `:306` (the pre-pairing list), `:307` (the certificate is reissued over the same key for names and a bounded validity), `:309` (the QR is a URL; the relay sentence is true of the native clients only and says why), `:312` (the token is still never in a URL; a browser's socket offers it as a protocol value).
- `docs/design.md`, "Remote client (2026-10-12)" status: a bullet for the web page next to the typed-code bullet (`:3562`), and the gaps in "Known gaps at the first merge" (`:3573`): no card excerpt, no resume or trust on the page, no "reveal the certificate file", the owner's Origin allowlist, the device checks.
- `README.md`: Layout rows for `src/adapters/web.rs` and `web/`; the dev-aid text at `:265-278` ("Copy invite" now copies a URL the CLI also takes); the data-directory text at `:78` (the certificate carries names and a bounded validity); the `tests/remote.rs` row.
- `CHANGELOG.md`: under Unreleased, a line that the remote port serves a browser page paired from a URL.
- No other script line, spike or `vendor/` change: the mechanisms are standards the facts file settles, and the unresolved ones are device checks, not spikes.

Commit message: `Serve a browser page on the remote port, paired from a URL, with a certificate that carries names`

Contract additions (the implementer copies these into the commit message):

- `Identity.certificate_names: Vec<String>` and `Identity.certificate_expires: Option<SystemTime>`, both `#[serde(default)]`; `IDENTITY_SCHEMA_VERSION` from the value in `src/ports/remote.rs` at implementation time, plus one.
- `AppAction::CertificateReissued(Identity)` and `Effect::EnsureCertificate`.
- `RemoteRequest.bearer_in_protocol`, `.offered_protocol` and `.cross_origin`; the 101 answers `Sec-WebSocket-Protocol: switchboard.v1` when offered.
- `GET /api/v1/device` → `{id, name, scope}`; `GET /`, `/app.js`, `/app.css`, `/switchboard.cer` served before pairing; 403 `bad_origin` on a cross-origin browser socket.
- `RemotePort::bind` takes the certificate DER; `remote::pairing_url`, `remote::reissue_identity`.
- The pairing QR and "Copy invite" carry a URL; `PairingInvite.parse` reads a URL, then JSON.
- No change to `SCHEMA_VERSION` (`src/core/model.rs`, 16 today), `VIEWS_SCHEMA_VERSION` (7 today), `devices.json`, `SessionView` or `RemoteReply`.
