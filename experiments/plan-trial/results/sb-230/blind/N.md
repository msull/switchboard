# Plan for #230: the browser page on the remote port

Worktree at 7fe8949. Line references are to this tree. `SCHEMA_VERSION`
is 16 (`src/core/model.rs:12`) and `IDENTITY_SCHEMA_VERSION` is 1
(`src/ports/remote.rs:17`). Neither moves: the one serialized change is
an additive `#[serde(default)]` field on `Identity`, the way `listen`
was added under version 1 (`src/ports/remote.rs:39-40`); see The
record. "Facts" is `facts/1/repo/facts.md`.

## Where this departs from the issue

- **The code travels in the fragment, not the query.** The QR is
  `https://<addr>:<port>/#code=<code>&key=<spki_sha256>&id=<server_id>`.
  A browser never sends a fragment to the server, so the code never
  reaches a request target, `RemoteRequest::query` or a `Referer`. It
  reaches the host only in the `/pair` body. That makes the notes'
  questions about logging the query, refusing a `GET /?code=`, and
  `Referer` leakage go away. This rests on RFC 3986 §3.5 (the user agent
  handles the fragment). That is not in the facts file, and
  `tests/remote.rs` cannot show it, because the host's client is not a
  browser. The page still calls `history.replaceState(null, "", "/")`
  before anything else, but facts list whether Safari or the camera
  records the URL first as Unresolved (facts:46). The fragment does not
  change that, and the plan accepts it: the code is single-use, and the
  window closes on the first pairing (`src/core/remote.rs:64`).
- **The card's one-line excerpt is not on the wire.** `agent_excerpt`
  derives it in the UI from the parsed conversation (facts:17,
  `src/ui/cards.rs:558-574`), and `SessionView` has no such field
  (`control/src/lib.rs:637-688`). The page's list shows
  `waiting_reason` when the session is waiting, otherwise the card word
  (`card`), plus the first line of `notes`. Facts leave the excerpt to
  the owner (facts:83). If the owner wants it, the fallback is an
  additive `#[serde(default)] excerpt: Option<String>` on `SessionView`,
  computed on the host by the `agent_excerpt` rule. That means a
  fixture rewrite and a `revision.rs` fingerprint change, so it is a
  separate commit.
- **The current certificate is not one a phone is sure to accept.** It
  has no SAN, validity to 4096, and no serverAuth EKU (facts:13, :33).
  The host issues a second, served leaf over the same Keychain key,
  keeps it in `identity.json`, and reissues it only when its address no
  longer matches or its expiry is within 30 days (details under The
  record and The signals). Pins survive (facts:12, spec §5 "a
  certificate reissued from the same key"). A browser's acceptance of
  the served leaf survives an app restart, because the leaf is the same
  bytes. Each issue has its own random serial, so no two certificates
  share issuer and serial (RFC 5280 §4.1.2.2; rcgen would otherwise
  derive the serial from the key, `rcgen-0.14.10/src/certificate.rs:458-468`).
- **"Revoked" is told apart by the page, not the host.** A revoked
  token and an unknown token get the same 401 (`src/app.rs:3142-3148`).
  The page keeps `{server_id, spki_sha256}` per origin in
  `localStorage`. On a 401 it says "revoked" when `/info` still shows
  that key, "identity changed, pair again" when the key differs, and
  "not paired" when it kept nothing. Facts leave this to the owner
  (facts:91). If the owner says otherwise, the fallback is a distinct
  401 code, which needs a record of revoked token hashes. Safari wipes
  script storage after 7 days without a visit (facts:32), so after that
  a revoked browser hears "not paired, scan the QR". That is still the
  right instruction.
- **"The screen for anything else" has no screen when the session is
  stopped.** `screen` refuses a session that is not running (spec
  §4.3). The page says "Not running; nothing to show". Facts leave this
  to the owner (facts:94). If the owner wants more, the fallback is
  `runs` then `runs/{n}/output` for a command or service.
- **The cookie is not scoped by port** (facts:28). A cookie set by
  `https://<ip>:43187` is sent to every port on that address. So the
  cookie name carries the port. Every cookie-authenticated request also
  passes an origin check, because `SameSite=Strict` does not stop
  another HTTPS service on the same IP: that service is same-site.

## Decisions

1. **Trust: the click-through is the trust step, and a browser pairs
   `read` only.** This is the owner's decision (facts:56-58), not a
   settled line, and it weakens two guarantees of §5:
   - "Pairing verifies the key before the code is sent"
     (`docs/remote-client.md:309`). A browser cannot. An impostor that
     the owner clicks through serves its own page, reads the fragment
     and relays the code with the real key. The URL's `key` then only
     catches a QR for another host or a stale QR: a genuine page
     compares it with `/info`'s self-reported `spki_sha256` and posts
     nothing on a mismatch.
   - "after pairing, every connection verifies the pin before the token
     is presented" (same line), and the token "is sent only as a bearer
     header over the pinned connection" (`:312`). A browser sends its
     cookie to whatever certificate it has accepted for the origin.

   What the plan builds against that: a stable served leaf (The
   record), so a second warning on a known origin is
   a sign of trouble rather than routine; and **a browser pairing is
   granted `read` whatever the window's scope.** The owner raises it to
   `control` by hand in the devices dialog, which §5 already allows
   (`docs/remote-client.md:314`). A relayed code therefore yields a
   reader, never a device that types into agent sessions, unless the
   owner raises it. The host tells a browser pairing by the `/pair`
   body's `"cookie": true` (decision 2). The dialog says what the
   click-through trusts in one sentence, and the hover on a browser's
   scope combo says what raising it trusts.

   §5's lines 309, 312 and 317 are rewritten to say the native
   guarantee and the browser's weaker one side by side, not added to
   (Docs).

   **If the owner rejects the click-through as the trust step**, the
   fallback the implementer builds is facts:58: the host serves its
   certificate before pairing at `GET /certificate`
   (`application/x-x509-ca-cert`), issued with basicConstraints CA:TRUE
   plus the served leaf's SAN, EKU and ≤825-day validity (facts:33,
   :34). The dialog gives the Settings > General > About > Certificate
   Trust Settings steps (facts:34), the pre-pairing rule names a fourth
   path, and browsers keep the window's scope. Without that answer the
   build is the read cap above.

   Separately, whether iOS Safari opens `wss://` after the click-through
   is Unresolved (facts:38, owner item facts:53). **The build does not
   depend on that answer.** The page falls back on its own to polling
   REST every 5 s when the socket fails before `hello`. Spike 23
   records it (Tests).
2. **Token: an `HttpOnly; Secure; SameSite=Strict; Path=/` cookie, no
   `Domain`, `Max-Age=34560000`.** The page's script never sees it. It
   survives Safari's 7-day storage cap where `localStorage` does not
   (facts:32). The same-origin WebSocket handshake sends it (facts:25).
   Name: `switchboard-<port>`, so two instances on one Mac never
   overwrite each other's cookie (facts:28). Native clients keep the
   bearer header, which is checked first. The page calls `fetch` in its
   default mode (`cors`) and never sets `mode` or `referrerPolicy` on a
   request: under `Referrer-Policy: no-referrer`, a non-GET request in
   any other mode carries `Origin: null` (Fetch, "append a request
   `Origin` header" step 4.1), which the origin rule refuses. Whether a page reached through
   a certificate override is a secure context, and so keeps a `Secure`
   cookie, is Unresolved (facts:45). If Safari drops the cookie, the
   fallback is the subprotocol route: `/pair` returns the token in the
   body, the page keeps it in `localStorage`, sends it as
   `Authorization` on `fetch` and as the offered `Sec-WebSocket-Protocol`,
   and `respond_upgrade` echoes it (facts:18, :26, :27). Re-pairing
   after 7 days unused is the known cost.
3. **One QR: the URL.** The JSON form leaves the QR and "Copy invite".
   `PairingInvite.parse` (`client/Sources/SwitchboardProtocol/Info.swift:114`)
   accepts the URL and still accepts the JSON, because
   `SWITCHBOARD_INVITE` lines and older pasted invites are JSON. That
   one function covers every caller (facts:14). The URL carries one
   address: the routed address the served leaf names
   (`ServedCertificate::address`, The record), bracketed when it is
   IPv6, so the QR's host and the certificate's SAN cannot disagree.
   When the pairing window opens and `local_address()` differs from
   that address (the Mac changed networks since the bind), the app
   reissues the leaf and rebinds before the QR is drawn (The actions
   and effects). The dialog already shows only one address. Facts leave which
   address to the owner (facts:71). If the owner says otherwise, the
   fallback is a `.local` name, which is Unresolved (facts:49) and not
   built.
4. **Static paths.** `GET /` (`text/html; charset=utf-8`),
   `GET /app.js` (`text/javascript; charset=utf-8`), `GET /app.css`
   (`text/css; charset=utf-8`). Nothing else: `/index.html`,
   `/favicon.ico` and every other non-API path answer 404 `not_found`
   as today. Each of the three carries `Content-Security-Policy:
   default-src 'none'; script-src 'self'; style-src 'self'; connect-src
   'self'; img-src 'self'; base-uri 'none'; form-action 'none';
   frame-ancestors 'none'` as a header. `connect-src 'self'` covers the
   same-origin `wss:` (facts:30), and `frame-ancestors` must be a
   header (facts:31). They also carry `Cache-Control: no-store`,
   `X-Content-Type-Options: nosniff` and `Referrer-Policy: no-referrer`.
   Every JSON reply also gains `Cache-Control: no-store` and `nosniff`.
   `serve_remote` answers all three on the frame, before the `/api/v1/`
   strip. When the request carries a `switchboard-<port>` cookie that
   `device_by_token` knows, the answer is `authenticated: true` (no
   `DeviceSeen`, no origin rule: a GET of public bytes has no effect),
   so the connection moves to the paired pool as an API request's does
   (`src/adapters/remote.rs:835-848`). Otherwise a paired phone's
   reload would leave connections that fetched only `/app.css` in the
   unauthenticated pool for the 30 s idle timeout (`:573`, set at
   `:785`), and the accept loop drops a fifth (`:701-705`): the phone's
   own `/device` and anyone's `/info` would be locked out for 30 s. An
   unknown or absent cookie still gets the page, unauthenticated. Facts
   leave the thread to the owner (facts:74); the frame is chosen
   because it has to serve the cookie case anyway, and one path is
   simpler than two. Static loads count in a new `page` window of 60 a
   minute per listener, shared by the three paths and checked on the
   connection thread with the others, which bounds the frame's load:
   three files per unpaired visit against 8 drains per pass. How many connections iOS Safari opens at
   once is Unresolved (facts:48). An unpaired load is three files and
   `/info`, so at most four in the stranger pool, each held up to 30 s
   idle. Spike 23 counts them; if an unpaired load or a pairing right
   after it is dropped, the fallback is `MAX_UNAUTHENTICATED` 4 → 8 and
   the §4.1 line with it. §5's rule becomes: before pairing,
   nothing but `/info`, `/pair` and the page's three files, which carry
   nothing of the host. With neither a fragment nor a cookie, the page
   shows `/info`'s `server_name`, whether pairing is open, and "On the
   Mac, open Remote devices, allow pairing and scan the QR" (the owner
   question at facts:77; the fallback is static text alone).
5. **Where the page lives and how it is tested.** `web/index.html`,
   `web/app.js` and `web/app.css` are embedded with `include_str!`
   (the precedent for embedding is `include_bytes!` at
   `src/main.rs:167`; the tree has no `include_str!` yet). The test checks a fixed list
   of referenced paths. There is no HTML parser (facts:21) and no JS
   test harness, since the page has no build step and no dependency.
6. **Device record.** The page sends `device_name` from
   `navigator.userAgent`: "Safari on iPhone", "Safari on Mac", "Chrome
   on Android", and so on, else "Browser". Safari on iPadOS sends the
   Mac user agent by default
   (https://developer.apple.com/forums/thread/119186), so a Mac user
   agent with `navigator.maxTouchPoints > 1` is named "Safari on iPad".
   A heuristic: a Mac with a touch screen would read as an iPad, and
   the owner sees the name only in the devices list.
   `/pair` already takes it (`src/app.rs:3650`) and caps it at 64
   characters (`src/core/remote.rs:166`). Scope is `read` for a browser
   whatever the window says (decision 1). The device is listed and revocable as any other. There is no
   `kind` field; the name tells devices apart. Facts leave this to the
   owner (facts:80). If the owner wants one, the fallback is an additive
   `#[serde(default)] kind` on `PairedDevice`, with no version
   (facts:16).
7. **The page's scope after a reload: `GET /api/v1/device`.** It
   answers `{device_id, name, scope}` under any scope. The page cannot
   do without it: the cookie is unreadable, and storage is wiped after
   7 days. It is also the request the page uses to learn it was
   revoked. Facts leave this to the owner (facts:87). If the owner says
   no, the fallback is to keep the scope from the `/pair` reply in
   `localStorage`, show the reply box when the scope is unknown, and
   hide it on a `403 scope`.
8. **The cookie reaches every endpoint the bearer does.** There is no
   second allowlist, because scope is the authority and a browser is a
   paired device like any other. Facts leave this to the owner
   (facts:65). If the owner says otherwise, the fallback is a match on
   how the request authenticated, limited to the page's paths.

## The record

`devices.json`: no change. A browser is a `PairedDevice`; the token's
hash is the same whether the token travels as a bearer header or as a
cookie.

`identity.json` gains the served leaf, so a browser's acceptance
survives a restart. `Identity` (`src/ports/remote.rs:26`) gets an
additive `#[serde(default)] pub served: Option<ServedCertificate>`,
under `IDENTITY_SCHEMA_VERSION` unchanged, as `listen` was
(`:39-40`). It is a field of the existing file rather than a new one
because it is written through the same `Effect::SaveIdentity` and
`RemoteStore::save_identity` path, so no port method is added.

```rust
/// The certificate the port serves: over the identity's key, naming
/// where a browser reaches the host. Reissued only when `address` no
/// longer matches or `not_after` is near.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServedCertificate {
    /// DER, base64.
    pub certificate: String,
    /// The routed address among its SANs, which the QR uses.
    pub address: Option<IpAddr>,
    pub not_after: SystemTime,
}
```

`certificate` in `Identity` stays as it is (the identity's own record
and what `make_identity` checks against the key); `tls_config` serves
`served` when present. A key rotation makes a new identity with
`served: None`. An older build reads the file and ignores the field
(`Identity` has no `deny_unknown_fields`); if it saves the identity, the
field is dropped and the next newer launch issues a fresh leaf, which a
browser meets as one more warning.

A change of the Mac's address is a new origin, so the browser pairs
again from a fresh QR. Write that under Known gaps.

## The signals

These change in `src/adapters/remote.rs`.

- `read_request` (:444) keeps three more headers on `RemoteRequest`
  (:360). `cookie: Option<String>` is only the value of the cookie
  named `switchboard-<port>`; other cookies are dropped. `origin:
  Option<String>` and `host: Option<String>` are the raw values.
  `page: bool` is true when `X-Switchboard-Page: 1` is present. The
  adapter learns its port from `Shared`, which gains `port: u16`,
  filled from `listener.local_addr()` in `bind` (:673).
- `Limits::limited` (:629) gains the `page` window for `/`, `/app.js`
  and `/app.css`; `connection` (:770) otherwise hands them to the frame
  like any request (decision 4).
- `respond` (:530) takes a content type and extra header lines.
  `RemoteReply` (:389) gains `cookie: Option<String>`, the token to set
  as `Set-Cookie`, and `content_type: &'static str`, so the frame's
  static answers carry their own type and headers.
- **The served leaf.** A new `pub fn serving_certificate(identity,
  key_pem, address: Option<IpAddr>, now: SystemTime) ->
  Result<ServedCertificate, String>` makes the leaf over the Keychain
  key with `rcgen::KeyPair::from_pem` (facts:12). It keeps the CN of
  `make_identity`. The SANs are `localhost`, `127.0.0.1`, `::1` and the
  routed address; rcgen turns an IP string into `SanType::IpAddress`
  (facts:13). It sets `extended_key_usages = [ServerAuth]`,
  `not_before = now − 1 day`, `not_after = now + 800 days` (under
  Apple's 825, facts:33), and `serial_number =
  Some(SerialNumber::from_slice(&r))` with `r` 16 bytes from
  `fill_random` and the top bit cleared so the integer is positive
  (`rcgen-0.14.10/src/lib.rs:878`). `time` is already in the tree at
  0.3.55 behind rcgen. Add it as a direct dependency (`time = "0.3"`)
  for `OffsetDateTime::from(SystemTime)`, and commit the updated
  `Cargo.lock`.
- `tls_config` (:144) serves `identity.served` when it is present and
  `identity.certificate` otherwise. Its signature does not change.
- The rule for when to reissue is a pure function in
  `src/core/remote.rs`: `pub fn served_is_current(served:
  Option<&ServedCertificate>, address: Option<IpAddr>, now: SystemTime)
  -> bool`, true when `served` exists, names `address`, and expires more
  than 30 days after `now`. If the owner answers "no" on naming the IP
  (facts:59), the SANs drop the routed address and `address` is not
  compared; the QR then takes `local_address()` itself.

## The actions and effects

One new action, `AppAction::ServedCertificateMade(ServedCertificate)`:
the core stores it on the identity through `update_identity`
(`src/core/remote.rs:105`), which emits the existing
`Effect::SaveIdentity`. No new `Effect`. `Paired` and `RevokeDevice`
carry a browser unchanged. The frame changes in `src/app.rs`:

- `bind_remote` (:2936) reads `local_address()` once. When
  `served_is_current` is false it calls `remote::serving_certificate`
  with the Keychain key it already holds (:2951-2956), dispatches
  `ServedCertificateMade`, and builds `tls_config` from the updated
  identity. A failure is logged and the port serves
  `identity.certificate` as today.
- The core routes `ServedCertificateMade` with the other remote
  actions (`src/core/action.rs:1257`).
- In `SwitchboardApp::dispatch`, after an `AppAction::OpenPairing`, the
  app compares
  `local_address()` with `identity.served.address`. When they differ it
  calls `bind_remote` again, which reissues and rebinds; the QR is
  drawn from the new `served.address` on the next frame. Existing
  connections drop with the old listener, as a `SetRemoteListen`
  rebind does today. The address lookup goes through a field
  `address_source: fn() -> Option<IpAddr>` on `SwitchboardApp`,
  `remote::local_address` by default and set by a test-only
  `set_address_source`, beside `set_remote_port_override`
  (`src/app.rs:2920`).

- `serve_remote` (:3017) routes `("GET", ["device"])` after
  `authenticate` to a new `remote_device`, which returns `{device_id,
  name, scope}` in the envelope.
- `authenticate` (:3127) checks `request.bearer` first, unchanged. Then
  it checks `request.cookie`. A cookie-authenticated request passes only
  when `origin` is absent or equals `https://{host}`, and either it is
  an upgrade whose `origin` equals `https://{host}`, or it carries
  `page`. Otherwise the reply is 403 `cross_origin`, "this request did
  not come from the host's page". The reasoning: a browser always sends
  `Origin` on a WebSocket handshake (RFC 6455 §4.1). A custom request
  header forces a CORS preflight, which the host never answers (Fetch
  standard). A same-origin GET in `cors` mode carries no `Origin`, so
  "absent" is accepted; `Origin: null` is not `https://{host}` and is
  refused. The tests prove the host's side of the rule only.
- `serve_remote` answers `GET /`, `/app.js` and `/app.css` before the
  `/api/v1/` strip: the bytes with their headers, and `authenticated`
  set when `device_by_token` knows the cookie.
- `remote_pair` (:3606): when the body has `"cookie": true`, the
  request must carry `page`, else 403 `cross_origin`. The device is
  made with `Scope::Read` whatever the window's scope (decision 1):
  `new_device` (`src/core/remote.rs:166`) gains no parameter; the app
  sets `device.scope = Scope::Read` before dispatching `Paired`. The
  reply's `scope` says `read`, omits `token`, and sets
  `cookie: Some(token)`. Without `cookie` it is unchanged.
- The invite moves out of `src/ui/remote.rs:160-166` into a pure
  function in `src/core/remote.rs`: `pub fn invite_url(identity:
  &Identity, code: &str, port: u16) -> Option<String>`. The host is
  `identity.served`'s `address`, so the QR names exactly the address
  the served leaf carries; it returns `None` when there is none.
  `SocketAddr::new(address, port)`'s `Display` brackets IPv6.

## The UI

- **The pairing window** (`src/ui/remote.rs:141`) draws its QR from
  `invite_url`, so `remote::local_address()` leaves the dialog (:157)
  and the frame does no lookup. The QR cache is keyed by the URL, not
  the code (:168-172). "Copy invite" copies
  the URL. A new line under the code says: "Scan with a phone's camera
  to open the page in its browser. The browser warns that the
  certificate is not trusted. Accepting it on a network you trust is
  the trust step. The key in the code stops the page pairing with
  another host by mistake, not an impostor. A browser pairs read-only;
  raise it below once it is yours." The stale hover on Control (:218)
  becomes "Reads, and sending replies into a session. A browser still
  pairs read-only."
- **The devices list** (`src/ui/remote.rs:230-276`): the scope combo's
  hover says "Control lets this device type into sessions. For a
  browser, raise it only if you paired it yourself, just now, and saw
  no second certificate warning."
- **The page** (`web/`, plain JS, no framework):
  - **Load.** Read `location.hash`, call
    `history.replaceState(null, "", "/")`, then `GET /api/v1/info`. If
    the fragment's `key` and `id` match `/info`, post `/pair` with
    `{code, spki_sha256: key, device_name, cookie: true}`; on a
    mismatch, say "This code is for another host" and post nothing.
    Keep `{server_id, spki_sha256}` in storage. Every `fetch` carries
    `X-Switchboard-Page: 1`.
  - **Header.** `server_name`, plus the device's name and scope from
    `/device`.
  - **Live data.** Open `wss://${location.host}/api/v1/events`. After
    `hello`, snapshot `projects`, each project's `sessions` and
    `waiting`. Apply buffered and later `changed` events by per-key
    revision as §4.4 says: refetch the resource the event names when
    its revision is past what the page holds. On `resync_required` or a
    close, reconnect and resync. Do the same on `visibilitychange` to
    visible (facts:47). If the socket fails before `hello`, poll the
    same reads every 5 s.
  - **The list.** Projects with a waiting session come first. Within a
    project, waiting sessions come first and are marked. Each row shows
    kind, liveness and the line from the departure above.
  - **The session view.** A Claude Code session shows `last-response`.
    A running session of any other kind shows `screen`, after a `watch`
    frame and its `watching` answer, and sends `unwatch` on leave.
    Refetch on the key's `changed` and every 30 s while visible, as
    §4.4 says. A stopped non-Claude session shows the note.
  - **Reply.** Shown only under `control`. It posts
    `sessions/{id}/send` with `{op, text}`, where `op` is 16 random
    bytes from `crypto.getRandomValues` as hex. `randomUUID` needs a
    secure context, which facts list as Unresolved (facts:45). A `403
    scope` hides the box. Under `read`, where every browser starts, the
    box's place says "Read-only. Raise this device to Control in Remote
    devices on the Mac to reply." `/device` is reread on every socket
    `hello` and on a `403 scope`, so a raise shows
    without a re-pair.
  - **States.** "Not paired", "Revoked", "Identity changed, pair again"
    and "Pairing is closed" are each said in a sentence. Resume, Trust,
    drafts, tickets and the conversation beyond the last response stay
    out (the issue's Not in this cut).

## The tests

- **Core** (`src/core/tests.rs`):
  - `invite_url` for a served leaf with an IPv4 and an IPv6 address,
    and `None` with no served leaf; the URL's host is the leaf's
    `address`, so the two cannot disagree.
  - `served_is_current`: true for a match; false for no leaf, another
    address, and an expiry within 30 days.
  - `ServedCertificateMade` sets `identity.served` and emits one
    `SaveIdentity`.
- **Adapter unit tests** (`src/adapters/remote.rs` tests):
  - `read_request` keeps only `switchboard-<port>` out of a `Cookie`
    with several cookies, and keeps `Origin`, `Host` and the page
    header.
  - `serving_certificate` over a key has the key's SPKI hash, a SAN for
    each name, the serverAuth EKU, and `not_after` at most 825 days
    out. Two issues over one key have different serials, and both
    differ from the serial of `make_identity`'s certificate.
  - An identity saved without `served` loads with `served: None`.
- **`tests/remote.rs`**:
  - Change `http` (:245) into `http_with(.., headers: &[(&str, &str)],
    ..) -> (u16, Vec<(String, String)>, String)` and keep `http` as a
    wrapper.
  - New: `the_page_is_served_before_pairing_with_its_headers`. The
    three paths return 200 with their content types, the exact CSP,
    `no-store`, `nosniff` and `no-referrer`. `/index.html`,
    `/favicon.ico`, `POST /` and `/other` return 404 `not_found`.
    `index.html` references exactly `/app.js` and `/app.css`, with no
    inline `<script>`, no `style=`, and no `http:` or `https:` URL.
    Every `"/api/v1/…"` literal in `app.js` begins with one of `info`,
    `pair`, `device`, `projects`, `waiting`, `sessions/` or `events`.
  - New: `a_browser_pairs_by_cookie_and_reads_and_subscribes_with_it`:
    - `/pair` with `cookie: true` and the page header, inside a
      `control` window, returns a `Set-Cookie` with each attribute, a
      body without `token`, and `scope: "read"`; the device in the
      core is `read`. Without the header it returns 403 `cross_origin`.
    - `/device` with the cookie returns `read`. A `send` with the
      cookie is 403 `scope`; after `SetDeviceScope` to control it
      types. Without the page header, with a foreign `Origin`, or with
      `Origin: null`, `/device` and `send` return 403 `cross_origin`.
    - `GET /app.css` with a known cookie moves its connection to the
      paired pool: `MAX_UNAUTHENTICATED` connections each fetch it with
      the cookie and stay open, and an unpaired `/info` on one more
      connection is answered. Without the cookie, that connection is
      closed at accept.
    - An events upgrade with the cookie and a matching `Origin` gets
      `hello`. A foreign `Origin` is refused. `subscribe_at` (:1960)
      gains a headers argument.
    - A cookie named for another port is not a token.
    - After `RevokeDevice`, the cookie gets 401 and the socket closes.
    - A bearer `/pair` inside a `control` window still grants
      `control`.
  - New: `the_served_certificate_is_kept_across_a_restart_and_names_the_qrs_address`.
    The port's leaf (read from the client handshake) is the same DER
    after the app is rebuilt over the same store, and it carries the
    address source's IP. With the address source changed, opening the
    pairing window rebinds, the served leaf names the new address, and
    `invite_url` names it too. The native pin verifies throughout.
  - No fixture changes: no wire view changes, and the Swift client does
    not read `/device`.
- **`tests/ui.rs`**: `remote_devices_dialog_opens_pairing_and_revokes_a_device`
  (:7424) seeds `served` on the identity and also finds the trust sentence while the window is open.
- **Swift**: `FixturesTests.swift` gains `testInviteParsesTheURL` next
  to the JSON test at :157. Cover IPv4, `[v6]:port`, and refusal of a
  URL missing `code`, `key` or `id`.
- **Spike 23** (`spikes/23-browser-page/README.md`) records the owner's
  device check, run before the commit:
  - scan, accept the warning; does `wss://` open (facts:53)?
  - is the cookie kept across a Safari relaunch (facts:45)?
  - quit and relaunch Switchboard, reload the page: does Safari warn
    again? It must not; a second warning means the served leaf changed.
  - how many connections an unpaired load opens, and whether `/pair`
    right after it is dropped while they sit in the stranger pool for
    their 30 s idle (facts:48); a drop selects the 4 → 8 fallback of
    decision 4.
  - a paired reload: does `/device` answer at once?

  The page works either way on the socket, so the commit does not wait
  on a "No". The README says which path iOS took.
- **Acceptance** is on a real phone with the signed bundle. It is not
  in CI. Between "see the sessions" and "send a reply", the owner
  raises the phone to Control in the devices dialog (decision 1).

## Docs

- **`docs/remote-client.md`**:
  - §4.1 bounds: the page window, and the three static files answered
    on the frame.
  - §4.3: add rows for `/`, `/app.js`, `/app.css` ("none, before
    pairing") and `GET /api/v1/device` (read).
  - §5, the pre-pairing rule (:306): `/info`, `/pair` and the page's
    three files.
  - §5, rewritten rather than added to:
    - :309 "Pairing verifies the key before the code is sent" states
      the native guarantee, then the browser's: the click-through is
      the trust step, the URL key catches a wrong or stale QR and not
      an impostor, and so a browser pairs `read` and is raised by hand.
    - :312, the token: "sent only as a bearer header over the pinned
      connection" becomes the native rule, beside the browser's
      `HttpOnly` cookie named by port, sent to whatever certificate the
      browser accepted for the origin, with the origin rule. "never in
      a URL" stays true: the code travels in the fragment.
    - :317 "The client verifies the pin on every connection" names the
      native clients only, and says a browser relies on the stable
      served leaf, so a second warning is the sign of a changed host.
  - §5, :307: the served leaf beside the identity's certificate, its
    SANs, EKU, validity and random serial, and when it is reissued.
- **`docs/design.md`**, Remote client section (from :3107): a new
  bullet, "The browser page (2026-10-xx)", before "Known gaps". Known
  gaps gain:
  - a changed address means re-pairing a browser;
  - iOS socket behaviour per Spike 23;
  - the excerpt is not on the page.
- **`README.md`**:
  - Layout lines for `web/` and `spikes/23-browser-page`.
  - The `tests/remote.rs` line (:437) gains the page and the cookie.
  - The adapter line (:383) gains the page window and the served leaf;
    the `src/app.rs` line (:401) gains the static files.
  - :277 says "Copy invite" copies the page's URL, which
    `switchboard-remote pair` also takes.
- **`client/README.md:180-190`**: `SWITCHBOARD_INVITE` takes a URL or
  JSON per line.
- **`CHANGELOG.md`** Unreleased: "The pairing QR code opens a page in a
  phone's or tablet's browser that lists the sessions, shows a
  session's last response or screen, and sends a reply once the
  browser is raised to Control in Remote devices."

## Commit

`The remote port serves a browser page, paired from a URL in the QR code`

Contract additions:

- **`src/ports/remote.rs`**: `ServedCertificate {certificate, address,
  not_after}`; `Identity.served: Option<ServedCertificate>`,
  `#[serde(default)]`, `IDENTITY_SCHEMA_VERSION` unchanged.
- **`src/core/action.rs`**: `AppAction::ServedCertificateMade(ServedCertificate)`.
  No new `Effect`.
- **`src/core/model.rs`**: none.
- **Protocol** (`docs/remote-client.md`):
  - static `GET /`, `/app.js`, `/app.css`;
  - `GET /api/v1/device` → `{device_id, name, scope}`;
  - `/pair` body field `cookie: bool`; with it, the device is `read`
    whatever the window grants, and the reply omits `token` and sets
    `Set-Cookie: switchboard-<port>`;
  - request header `X-Switchboard-Page: 1`;
  - refusal code `cross_origin`;
  - `Cache-Control: no-store` and `X-Content-Type-Options: nosniff` on
    every reply;
  - the invite as `https://<addr>:<port>/#code=&key=&id=`, with the JSON
    still accepted by `PairingInvite.parse`.
- **Adapter** (not contract):
  - `RemoteRequest.{cookie, origin, host, page}`;
  - `RemoteReply.{cookie, content_type}`;
  - `remote::serving_certificate`;
  - `tls_config` serves `identity.served` when present.
- **App** (not contract): `SwitchboardApp::set_address_source`.
- **Core helpers** (not contract): `core::remote::invite_url`,
  `core::remote::served_is_current`.
