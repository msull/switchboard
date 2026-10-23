# Plan: a web control surface on the remote port (#230)

Line numbers are as they stand in the worktree at `7fe8949`. Schema
versions today: `SCHEMA_VERSION = 16` (`src/core/model.rs:12`),
`VIEWS_SCHEMA_VERSION = 7` (`src/core/model.rs:612`), both untouched;
`IDENTITY_SCHEMA_VERSION = 1` (`src/ports/remote.rs:17`), bumped below.

## Where this departs from the issue

- **"The browser then trusts that certificate for that address."** Not
  the certificate this tree makes. `make_identity` builds it with no
  subjectAltName, no EKU and no basicConstraints
  (`src/adapters/remote.rs:101-110`; facts, Settled). Apple fails every
  TLS connection whose name is not in a SAN on iOS 13+ (facts, Settled,
  support.apple.com/103769), and Chrome answers a SAN-less certificate
  with `NET::ERR_CERT_COMMON_NAME_INVALID` (facts, Settled). So the plan
  reissues the certificate **over the same key** with an `iPAddress`
  SAN for each address the port serves, plus EKU `serverAuth`. The pin
  is over the key (`src/adapters/remote.rs:111`; spec §5 "a certificate
  reissued from the same key still verifies"), so no native pin and no
  pairing is lost. Whether iOS Safari offers a click-through at all is
  still Unresolved; a SAN'd certificate is the one form both vendors
  accept, so it is the default whichever way that answer goes.
- **"The one-line excerpt the card shows."** The wire `SessionView` has
  no excerpt (`control/src/lib.rs:637-688`; facts, Settled), and the
  card's excerpt is UI state: `UiState::captions` (`src/ui/mod.rs:75-76`)
  and `agent_excerpt` over the conversation cache
  (`src/ui/cards.rs:175-185`). Putting it on the wire would mean a read
  model built from `UiState`, which the core never reads. The page's row
  shows `waiting_reason` when the session is waiting (what the card puts
  first, `src/ui/cards.rs:304-315`) and the card word otherwise. The
  excerpt is a follow-up.
- **"Refreshed on `changed`" over `/api/v1/events`.** Whether WebKit
  opens `wss://` to a host whose certificate was only clicked through is
  Unresolved (facts: WebKit bug 41419, Cockpit's note). The page opens
  the socket as the headset does and, when it does not reach `hello`
  within 5 s or closes before one, falls back to polling by revision
  (under **The signals**) and says it is polling. The acceptance step "a
  session that starts waiting moves to the top without a reload" holds
  either way.
- **"Scope … in the header."** No endpoint answers a device's scope
  after `/pair`. `authenticate` hands the device only to the handler
  (`src/app.rs:3127-3156`), and spec §5's "the host answers with the
  device's scope" (`docs/remote-client.md:316`) is not what the tree
  does. The plan adds `GET /api/v1/device`, which also gives the page
  one call on load that tells "paired" from "not".
- **"Scan the QR with a phone."** The iOS Camera shows a link that the
  user taps to open Safari (facts, Settled, support.apple.com/102680).
  The acceptance run says "tap the link".

## Decisions

1. **Browser trust: the click-through is the trust step**, said in the
   pairing section of the Remote devices dialog. A certificate download
   would need a certificate that iOS fully trusts. Apple's page names
   root certificates only, and whether a leaf with no basicConstraints
   is listed is Unresolved (facts, support.apple.com/102390). Building
   the download now would mean a CA certificate or a local CA, which
   changes the identity model of spec §5. That is a follow-up for the
   owner. The click-through works whichever way that answer goes.

   What the click-through buys: the browser keeps an exception for that
   certificate at that address, so a different certificate on a later
   visit brings the warning back. What it does not buy: on the first
   visit the owner cannot tell the real host from an impostor that
   relays to it, because both show a warning. The page's check of the
   URL's `key` against `/info` catches only a host at that address that
   is not the one whose QR was scanned (a reused address, a second
   instance, a stale QR). An impostor's page would not run that check.
   The dialog says so in two sentences (under **The UI**). §5's rule
   becomes: before pairing, a non-loopback bind serves `/info`, `/pair`
   and the page's four files, and nothing else.
2. **The token rides in a cookie the page's script never sees.** Where a
   credential lives is a security decision: an `HttpOnly` cookie cannot
   be read by script, and `localStorage` can. So `/pair` with
   `"cookie": true` answers with no `token` in the body and sets
   `__Host-sb-<server_id>=<token>; Path=/; Secure; HttpOnly;
   SameSite=Strict; Max-Age=34560000` (400 days). The name carries the
   `server_id` because cookies ignore the port: two installations on one
   Mac must not overwrite each other's cookie. The WebSocket handshake
   is fetched with credentials `include`, so the cookie rides on the
   upgrade as on `fetch` (facts, Settled, WHATWG WebSockets §2.2 step
   2). Native clients keep the bearer header unchanged. When a request
   carries both, the bearer wins.

   **Origin rule.** A request authenticated by the cookie must carry
   `Origin` equal to `https://` plus its own `Host` if it is a `POST`
   or an upgrade. Without it the host answers `403 origin`. A same-site
   page on another port of the same Mac would otherwise get the cookie
   on a WebSocket, which no CORS rule guards. Cookie-authenticated `GET`
   reads need no check: `SameSite=Strict` keeps a cross-site request
   from carrying the cookie, and a same-site one on another port cannot
   read the reply without CORS headers, which the host never sends.

   **Residual risk, said in §5:** a cookie is sent to every port of the
   address, so another HTTPS service the owner runs on the same Mac and
   visits from that browser receives the token.

   **Unresolved, the implementer's first test:** whether Safari and
   Chrome store and send a `Secure; HttpOnly; SameSite=Strict` cookie
   after a certificate exception, and keep it across a browser restart
   (facts, Unresolved; For the owner, second item). Run it before
   anything else in the commit (**The tests**, step 0). If it fails,
   stop and report. The fallback (the token as a `Sec-WebSocket-Protocol`
   value with an echoed protocol name, kept in script storage) is a
   follow-up for the owner, not a second transport built now.
3. **One QR, a URL.**
   `https://<address>:<port>/?code=<code>&key=<spki_sha256>&id=<server_id>`,
   built by a pure `core::remote::invite_url`. "Copy invite" copies the
   same URL. Nothing needs the JSON form any more: the headset has no
   camera, and the CLI and `SWITCHBOARD_INVITE` parse text. The Swift
   `PairingInvite.parse` takes the URL and still reads JSON, so an
   invite pasted from an older host keeps working. With no LAN address
   (`local_address()` is `None`, or the port is on loopback) the URL
   uses `127.0.0.1`.
4. **The static paths.** The set is exactly `GET /`, `/app.js`,
   `/app.css` and `/icon.svg`. Every other path outside `/api/v1/` keeps
   today's `404 not_found` (the `/other` case in
   `tests/remote.rs:426-476` stays), and so do `/index.html`,
   `/app.js?x`, `/web/app.js`, `/..` and a `POST /`.
   - **Content types:** `text/html; charset=utf-8`,
     `text/javascript; charset=utf-8`, `text/css; charset=utf-8` and
     `image/svg+xml`.
   - **Headers on each static reply:**
     - `Content-Security-Policy: default-src 'none'; script-src 'self';
       style-src 'self'; img-src 'self'; connect-src 'self' wss:;
       base-uri 'none'; form-action 'none'; frame-ancestors 'none'`.
       `wss:` is named because `'self'` does not cover WebSocket schemes
       in every engine (facts, Settled, MDN and WebKit 235873).
     - `X-Content-Type-Options: nosniff`.
     - `Referrer-Policy: no-referrer`, so the subresource loads of
       `/?code=…` never carry the code in a `Referer`.
     - `Cache-Control: no-store`, so a new app version is a new page
       with nothing to invalidate.
     - `Connection: close`.
   - **Every JSON reply** also gains `Cache-Control: no-store` and
     `nosniff`, so transcripts never land in a browser's disk cache
     (spec §1: content in memory only).
   - **Pool and rate window:** the connection thread serves static
     paths from the embedded bytes without reaching the frame. They need
     no state, which is what the adapter is for
     (`src/adapters/remote.rs:1-10`). They count against the
     unauthenticated pool of four (`:567`) like any new connection.
     Because each reply closes its connection, a page load holds a slot
     for one round trip, not for the 30 s idle timeout (`:573`). How
     many connections a first load opens is Unresolved (facts), so step
     0 measures it in desktop Safari and Chrome. Static paths have no
     rate window, since a reply is a memory copy on a connection thread
     and the pool already bounds a stranger.
   - **The query:** `read_request` keeps the query only for paths under
     `/api/v1/` (`:496-504`), so the code in `/?code=…` never enters a
     `RemoteRequest`, a `Debug` print or a log.
   - **The page opened with no code and no pairing** says "This browser
     is not paired. On the Mac, open Remote devices, choose Allow
     pairing, and scan the code." It then calls nothing but `/info`.
5. **Where the page lives:** `web/index.html`, `web/app.js`,
   `web/app.css` and `web/icon.svg`, embedded by `src/adapters/web.rs`
   with `include_str!`. That macro reads the file at compile time into
   a `&'static str` baked into the binary, so the binary carries the
   page and nothing reads `web/` at run time. There is no framework, no
   build step and no dependency. Tests are under **The tests**.
6. **The device record is unchanged.** The page names itself from
   `navigator.userAgent` ("Safari on iPhone", "Chrome on Android",
   "Safari on Mac") and sends that as `device_name`, as every client
   does. The host caps it at 64 characters (`src/core/remote.rs:166-175`).
   Scope is chosen in the pairing window, and the device is listed and
   revoked in the dialog as any device is.

## The record

- **`identity.json`** gains `certificate_addresses: Vec<IpAddr>`
  (`#[serde(default)]`): the addresses the stored certificate's SANs
  name. The bump is `IDENTITY_SCHEMA_VERSION` from its value in
  `src/ports/remote.rs` at implementation time, plus one (1 today). No
  migrate step is needed: an old file reads with an empty list, and the
  first bind reissues. `certificate` is still base64 DER, and
  `spki_sha256` is unchanged by a reissue.
- **`devices.json`** is unchanged. `core/model.rs` is not touched.
- **On the device** (none of it secret): the token in the cookie, and
  in `localStorage` under `sb-pairing-<server_id>` a note
  `{server_id, spki_sha256, device_id, server_name}`. The note is what
  lets the page tell "identity changed" and "revoked" from "never
  paired". Every read and write is wrapped in try/catch. With storage
  refused (a private tab) the page keeps the note in memory and says
  it will forget the pairing when the tab closes.
- **Rehydration.** The page rebuilds everything per load, in this order:
  1. `GET /api/v1/info`, compared with the note's key.
  2. `GET /api/v1/device`.
  3. Open the socket and wait for `hello`.
  4. Fetch the snapshots.
  5. Apply the buffered events by revision, as spec §4.4 "Subscribe
     first, then snapshot" says.

## The signals

- **Certificate addresses.** `bind_remote` (`src/app.rs:2936-2986`)
  works out the wanted set: `127.0.0.1`, plus `local_address()` when
  `listen.lan`. If that set differs from `certificate_addresses`, it
  calls `remote::reissue_certificate(&identity, &key, &addresses)` with
  the Keychain key it already reads, then dispatches
  `CertificateReissued` before it builds `tls_config`. While bound on
  the LAN, the once-a-second poll checks that `local_address()` is in
  the set. On a move (never on `None`, a network flap), it calls
  `bind_remote` again. The rebind drops open connections, native
  clients reconnect, and their pins still hold. A browser paired at the
  old address has to pair again anyway, because both the cookie and the
  certificate exception belong to the address. The dialog's sentence
  says so.
- **Revocation and identity, on the page.**
  - **Identity changed:** a `/info` key that differs from the note's,
    shown as "identity changed, pair again". The page sends nothing
    more.
  - **Revoked:** a `401` with a note present, shown as "This browser's
    pairing was revoked on the Mac. Pair again." The host answers one
    `401 unauthorized` for unknown and revoked tokens
    (`src/app.rs:3127-3150`); the note is what tells them apart.
  - **On a revocation** the host closes the socket
    (`src/app.rs:486-497`). The page's next request (its reconnect's
    `GET /api/v1/device`, or the next poll) gets the `401`.
- **The polling fallback.** While visible, every 3 s the page calls
  `GET waiting`. When the envelope revision moved past the one the page
  holds, it refetches `projects`, each project's `sessions`, and the
  open session's content. The page shows "Live updates unavailable;
  refreshing every 3 s". It stops while `document.hidden`. A content key
  read in the last minute stays in the host's watch set (spec §4.4), so
  the polled screen stays current.

## The actions and effects

- **`AppAction::CertificateReissued { certificate: String, addresses:
  Vec<IpAddr> }`.** The core sets `identity.certificate` and
  `certificate_addresses` through `update_identity`
  (`src/core/remote.rs:105-117`), which emits the existing
  `Effect::SaveIdentity`. It never touches `spki_sha256`, `server_id`
  or the devices. No new `Effect` or port method.
- **`adapters::remote`:**
  - `reissue_certificate`. It shares one certificate builder with
    `make_identity`: CN as today, `subject_alt_names` set to one
    `SanType::IpAddress` per address, `extended_key_usages` set to
    `[ServerAuth]`, rcgen's default validity (NotBefore 1975, which
    keeps it outside Apple's 825-day and EKU rules; facts, Settled),
    the key from `KeyPair::from_pem`. `make_identity` passes no
    addresses, and the first bind fills them.
  - `RemoteRequest` gains three fields:
    - `origin: Option<String>`;
    - `host: Option<String>`;
    - `cookies: Vec<(String, String)>`, which keeps only the cookies
      named `__Host-sb-…`.

    These are read in `read_request`, never logged, and `refuse` still
    logs only method and path.
  - `RemoteReply` gains `set_cookie: Option<String>`, which `respond`
    writes. The 30 existing literals (28 in `src/app.rs`, 2 in the
    adapter) take `set_cookie: None`.
  - `respond` adds `Cache-Control: no-store` and `nosniff`. A
    `respond_static` writes the static headers of decision 4.
    `connection()` (`:770-867`) serves a `GET` of a static path before
    the rate check and the frame, then returns, so the connection
    closes.
- **`adapters::web`:** `pub fn asset(path: &str) -> Option<(&'static
  str, &'static [u8])>` (the content type and the bytes), plus the CSP
  constant.
- **`app.rs`:**
  - `authenticate` takes the bearer, or else the cookie
    `__Host-sb-<server_id>`. For a cookie on a `POST` or an upgrade it
    applies the Origin rule.
  - `remote_pair` reads `cookie: bool`. When it is set, the Origin rule
    applies (`403 origin` without it), the body leaves out `token`, and
    the reply carries `set_cookie`.
  - New route `("GET", ["device"])` answers `{id, name, scope}` in the
    envelope, with `authenticated: true`.
  - `remote_info`'s capabilities add `"device"`.
  - The once-a-second poll gains the address check.
- **`core::remote::invite_url(identity, code, addr: SocketAddr) ->
  String`:** a pure function, moved out of `src/ui/remote.rs:155-166`
  per CLAUDE.md's no-logic-in-`ui` rule.
- **`src/script.rs:730-733`:** `pairing <code> [read|control]`, read by
  default, so a control-scope browser pairing can be driven in the live
  app.
- **Swift:** `PairingInvite.parse`
  (`client/Sources/SwitchboardProtocol/Info.swift:113-116`) takes an
  `https://` URL (`code`, `key`, `id`, and host:port into `addresses`)
  and still takes JSON. The usage line in
  `client/Sources/switchboard-remote/main.swift:12` says "the QR's URL
  or text", and `client/README.md:180-190` changes to match.

## The UI

**The pairing section of the Remote devices dialog** (`src/ui/remote.rs:140-215`):

- The QR and "Copy invite" carry `invite_url`.
- The text beside the QR shows the address, the key's short form and the
  code, as today.
- The hint becomes "Scan with a phone's camera and tap the link, or type
  the key and the code in the headset." The scope and five-minute
  sentence stays.
- One sentence for browsers: "A browser warns that the connection is not
  private on its first visit; accepting that warning is how it comes to
  trust this Mac, so accept it right after scanning, on a network you
  trust. If the warning returns later while this Mac's address has not
  changed, don't accept it; pair again."
- Browsers are listed with the other devices, with nothing new.

**The page** (`web/`). It uses `textContent` only, never `innerHTML`,
since redacted text is still untrusted markup. `app.js` runs in this
order:

1. **Strip the query first.** The first statement reads `location.search`
   and calls `history.replaceState(null, "", "/")`. Only then does it
   wait for the DOM.
2. **Pair, if the URL carried a code.**
   - `GET /api/v1/info`. If its `spki_sha256` or `server_id` differs
     from the URL's `key` or `id`, show "This is not the Mac whose code
     you scanned" and send nothing.
   - Otherwise `POST /api/v1/pair {spki_sha256, code, device_name,
     cookie: true}`. Then write the note and continue.
3. **Load.** `GET /api/v1/device` gives the scope and confirms the
   pairing. The header shows `server_name` and the scope word.
4. **List.** Sessions are grouped by project from `GET projects` and
   each `projects/{id}/sessions`. The rows in `GET waiting` come first in
   each group and are marked. Each row shows the kind, the card word and
   the waiting reason. `changed` and `removed` events refetch by kind,
   at the revisions spec §4.4 gives.
5. **Session view.**
   - A `claude` session reads `last-response`. It falls back to
     `screen` on `400 no_conversation` (facts, Settled,
     `src/app.rs:1869-1886`).
   - Any other session reads `screen` under `watch {kind: "screen", id}`
     frames, and shows "not running" on `400 failed` (`:1863-1866`).
   - It refreshes on `changed` for that key.
6. **Reply.** A text box and Send, shown only under `control` scope.
   Send posts `send {op, text}`. The op is 16 bytes from
   `crypto.getRandomValues` as hex; `crypto.randomUUID` is avoided
   because it needs a secure context, which a clicked-through origin may
   not be.
   - `403 scope` flips the page to read and hides the box.
   - `409 interrupted` says so.
7. **States**, each one sentence: not paired, not this Mac, identity
   changed, revoked, polling, and the host unreachable (with a retry
   every 5 s).

## The tests

0. **First, by hand, before the page is built past a stub.** This settles
   the Unresolved lines decision 2 and decision 4 rest on.
   - Run a test instance: `SWITCHBOARD_DATA_DIR=/tmp/sbw`,
     `SWITCHBOARD_REMOTE_PORT=43190`, and a script with `remote-listen
     lan 43190` and `pairing ABCD2345 control`.
   - In desktop Safari and Chrome on the Mac, open `https://<lan
     ip>:43190/?code=…`. Check four things: the click-through is
     offered; the cookie from `/pair` is sent on the next `fetch` and on
     the `wss` upgrade; the cookie survives quitting the browser; and no
     static load is refused at the pool of four.
   - Write the results into the design.md status bullet. If the cookie
     is not kept or not sent, stop and report: the fallback is a
     follow-up.
1. **`src/core/tests.rs`.**
   - `CertificateReissued` changes the certificate and its addresses,
     keeps `spki_sha256`, `server_id` and the devices, and emits one
     `SaveIdentity`. A read-only core emits none.
   - `invite_url` gives the exact URL for a fixed identity, code and
     address.
2. **`src/adapters/remote.rs` unit tests.**
   - A reissued certificate hashes to the same SPKI
     (`spki_sha256_of_cert`). It carries an `iPAddress` SAN for each
     address and the `serverAuth` EKU, read back with `der_element`
     (the crate builds no x509 parser).
   - `read_request` drops the query off `/api/v1/`, keeps it on it, and
     keeps only `__Host-sb-` cookies, `Origin` and `Host`.
3. **`tests/remote.rs`.** Add a raw variant of `http` (`:245-296`) that
   sends extra headers and returns the status, the headers and the
   bytes.
   - **Static paths.** `/`, `/app.js`, `/app.css` and `/icon.svg` answer
     unpaired with their content type and the exact CSP, `nosniff`,
     `no-referrer`, `no-store` and `Connection: close`. The paths of
     decision 4 answer 404. `GET /?code=…` changes nothing in the core:
     no `PairingFailed`, and `serve_pending` sees no request.
   - **The embedded page references only what the host serves.** Every
     `src=` and `href=` in `index.html` is in the static set. Every
     `/api/v1/` path literal in `app.js` names a route `serve_remote`
     answers. No web file holds `http:`, `https:`, `//` or `data:` URLs.
     `index.html` has no inline `<script>` body, `style=` or `on…=`
     attribute.
   - **Pairing by cookie.** `/pair` with `cookie: true` and a matching
     `Origin` answers 200 with no `token` and a `Set-Cookie` with the
     exact name and attributes. Without `Origin` it answers
     `403 origin`. Bearer pairing is unchanged.
   - **Cookie authentication.**
     - `GET waiting` and `GET device` answer with the cookie.
     - `POST send` with the cookie and no `Origin`, or a foreign one,
       answers `403 origin` with no side effect. With the right `Origin`
       under `control` it is accepted.
     - An upgrade with the cookie and the right `Origin` gets `hello`; a
       foreign one gets 403.
     - A revocation closes that socket, and the next `GET device`
       answers 401.
     - The bearer wins over a cookie.
   - **`GET device`** follows `SetDeviceScope`.
   - **Every JSON reply** carries `Cache-Control: no-store`.
   - **Pool.** Extend `stalled_strangers_fill_their_own_pool…`
     (`:3226`): after four sequential static loads, a fifth connection
     is accepted.
   - **Bind.** The test port's identity has `127.0.0.1` in
     `certificate_addresses`, its SPKI as before, and devices kept
     across the reissue.
   - **Fixtures** are unchanged: no wire view changes, and the headset
     does not read `device`.
4. **`tests/ui.rs`.** The pairing section shows the invite URL and the
   browser sentence with a window open.
5. **Swift, by hand** (not in the hook or CI, design.md A2). Add
   `testInviteParsesTheInviteURL` beside `testInviteParsesTheQRText`
   (`client/Tests/SwitchboardProtocolTests/FixturesTests.swift:157-165`),
   run with `swift test` in `client/`.
6. **The issue's acceptance run, the owner's** (facts, For the owner):
   1. Open the pairing window at control scope.
   2. Scan with the iPhone, tap the link, and accept the warning.
   3. See the sessions.
   4. Make a session wait, and see it move to the top without a reload.
   5. Send a reply, and see it arrive.
   6. Revoke the phone, and see the page say so on its next request.

   Note the iOS version, and whether the page was live or polling.
   Client isolation on the owner's network is the owner's to check.
   Nothing in this tree can fix it.

**Docs in the same commit:**

- `docs/remote-client.md`:
  - §4.1: static paths, connection thread, `Connection: close`, pool,
    no window.
  - §4.3: table rows for the four files and `GET /api/v1/device`.
  - §5: `:306` becomes "/info, /pair and the page's four files"; `:309`
    the QR is the invite URL; `:312` "a bearer header, or for a browser
    the `__Host-sb-` cookie; never in a URL"; plus the browser-trust
    paragraph of decision 1 and the Origin rule with its residual risk.
  - A new §7.4, "The browser page".
  - §11's test list.
- `docs/design.md`: a "Web page" bullet in the Remote client status,
  before "Known gaps at the first merge", with step 0's results and the
  gaps (the excerpt, the scope seen only on load).
- `README.md`:
  - Layout rows for `web/` and `src/adapters/web.rs`, and the
    `adapters/remote.rs` row (`:383`) updated for static replies, the
    cookie and the reissue.
  - The `identity.json` line (`:78`).
  - The script line `pairing <code> [read|control]` (`:279`, `:318`).
- `CHANGELOG.md` Unreleased: "A browser on the local network can pair
  from the QR code and list, read and reply to sessions."

## Follow-ups for the owner

- A certificate to install on the device in place of the click-through.
  This needs a CA certificate or a local CA, so it changes spec §5's
  identity, and rests on the leaf-trust question (Unresolved).
- If step 0 or the iPhone run shows the cookie is not kept, the token as
  a `Sec-WebSocket-Protocol` value with script storage. That is the
  weaker store, so it is the owner's call.
- The card's excerpt on the wire. It needs the caption to move out of
  `UiState`.
- A scope change pushed to a connected device. Today the headset and
  the page both see it only on load or on a `403 scope`.
- A `.local` name in the certificate's SANs, if the page is to be
  offered by name.

## Commit

`The remote port serves a browser page, paired from a URL in the QR, with its token in an HttpOnly cookie`

Contract additions:

- `ports::remote::Identity.certificate_addresses: Vec<IpAddr>`
  (`#[serde(default)]`). `IDENTITY_SCHEMA_VERSION` goes from its value
  at implementation time, plus one.
- `AppAction::CertificateReissued { certificate: String, addresses:
  Vec<IpAddr> }`.
- `adapters::remote::RemoteRequest.origin: Option<String>`,
  `.host: Option<String>` and `.cookies: Vec<(String, String)>`.
- `adapters::remote::RemoteReply.set_cookie: Option<String>`.
- `adapters::remote::reissue_certificate`.
- `adapters::web::asset`.
- `core::remote::invite_url`.
- HTTP:
  - `GET /`, `/app.js`, `/app.css`, `/icon.svg`;
  - `GET /api/v1/device` → `{id, name, scope}`;
  - `/pair` body `cookie: bool`;
  - `403 origin`;
  - `info.capabilities` gains `"device"`;
  - `Cache-Control: no-store` on every reply.
- Script: `pairing <code> [read|control]`.
- Swift: `PairingInvite.parse` accepts the invite URL.
