# A web page on the remote port (issue #230)

The host serves a small page from the remote port it already has. Any
browser on the local network opens it, pairs from a URL in the pairing
window's QR code, lists the sessions with the waiting ones first, shows
a session's last response or screen, and sends a reply. The API is the
one the headset uses, plus one small read. The page is three files
under `web/`, embedded with `include_str!`. There is no build step and
no dependency.

Line numbers are as of 7fe8949 in this worktree. "Facts" means
`facts/1/repo/facts.md`, cited by section and line.

## Where this departs from the issue

- **The pairing data goes in the fragment, not the query.** The URL is
  `https://<ip>:<port>/#code=…&key=…&id=…`.
  - A fragment never appears in the request line or in `Referer` (Facts,
    Settled, line 17). The code therefore never reaches the host's
    `RemoteRequest.query`, a proxy, or a log. A `?code=` would reach all
    three before any page script runs.
  - The page still calls `history.replaceState` first. Whether Safari,
    iCloud or the camera keep the original URL is unresolved (Facts,
    Unresolved, line 35), so the plan does not depend on it. The code's
    protection is what it already is: single use, five minutes, and
    three failures close the window.
- **Pairing waits for one tap, with a name field.**
  - The page shows "Pair as [name]" with a default taken from the user
    agent, and posts `/pair` only when the owner taps Pair.
  - The tap is needed because iPadOS Safari may identify itself as a Mac
    (Facts, Unresolved, line 36), and the dialog has no way to rename a
    device. So the name has to be chosen on the page.
- **The events socket is an enhancement. Polling is the default.**
  - Whether `wss://` connects in Safari after a certificate click-through
    is unresolved (Facts, Unresolved, lines 28–33).
  - The page first works by polling: `waiting` every 5 s, the open
    session's read every 5 s, and the lists every 30 s, only while the
    tab is visible.
  - When the socket opens, the page follows §4.4 and stops polling. When
    the socket fails, the header says "Live updates unavailable".
  - Under either answer, "a waiting session moves to the top without a
    reload" holds.
- **The page needs one new read: `GET /api/v1/device`.** It answers
  `{device_id, name, scope}`.
  - Only the `/pair` reply tells a device its scope (Facts, Settled,
    line 7), and the reply box depends on scope.
  - The page's first authenticated call on load is this read, so a 200
    or 401 here also decides between "paired" and "pair again".
- **Not every session gets a one-line excerpt.**
  - `SessionView` has no excerpt field (`control/src/lib.rs:637`). The
    Mac builds its excerpt from the transcript in
    `src/ui/cards.rs:558`, and the headset reads `last-response` once
    per session (`client/Sources/SwitchboardUI/BrowserView.swift:152-155`).
  - The cost of that on real data is unresolved (Facts, Unresolved,
    line 40). So the page reads `last-response` only for waiting Claude
    Code sessions, at most 8 of them, and for the session that is open.
  - Every other row shows its `waiting_reason`, or its `card` word.
  - An excerpt field on `SessionView` is a follow-up.
- **The certificate is reissued over the same key.** The issue does not
  mention this.
  - Today's certificate has no SAN, no EKU, and a `not_after` in the year
    4096 (`src/adapters/remote.rs:95-110`; Facts, Settled, line 10).
    Apple says connections with such a certificate "will fail" (Facts,
    Settled, line 11). Whether Safari or Chrome still offer a
    click-through for it is unresolved (Facts, Unresolved, line 27).
  - The plan builds a compliant certificate:
    - a SAN for the LAN IP, `127.0.0.1` and `localhost`;
    - the serverAuth EKU;
    - a validity of 800 days, reissued when fewer than 30 days remain or
      when the current address is missing from the SAN.
  - The SPKI does not change, so every native pin stays valid (spec §5:
    "a certificate reissued from the same key … still verifies").
- **A browser's acceptance of the certificate is not permanent.**
  - Chrome forgets a bypass after 7 days (Facts, Settled, line 18), so on
    Chrome the warning comes back once a week.
  - How long Safari remembers one is unresolved (Facts, Unresolved,
    line 34). The page assumes nothing about it.
- **"Identity changed" appears after the browser's own warning.**
  - After a key rotation, the browser shows its interstitial before any
    page script runs (Facts, Settled, lines 18–19).
  - Once the owner accepts, the page compares `/info`'s key with the one
    it stored at pairing and shows "identity changed, pair again".
- **The page learns "revoked" over REST, never from the socket.**
  - A refused upgrade reaches the page only as a 1006 close (Facts,
    Settled, line 15). The page therefore follows every socket close
    with a `GET device` call. A 401 there shows "No longer paired".
  - The host gives a revoked device and a rotated pairing the same 401
    (`src/app.rs:3127-3157`). The page tells them apart from the key it
    stored, not from the reply.
- **`SameSite=Strict` does not isolate one port from another.**
  - Every port on one IP is same-site, and a cookie is sent to all of
    them (Facts, Settled, line 16).
  - To keep two instances from colliding, the cookie is named after the
    `server_id`.
  - To stop another page on the same IP from riding on the cookie, a
    cookie is accepted only on a request carrying `X-Switchboard-Page: 1`
    (a cross-origin page cannot send that header without a preflight the
    host never answers), or on an upgrade whose `Origin` is the host's
    own (Facts, Settled, line 13: browsers must send `Origin`).
  - The plan accepts that the cookie also reaches any other HTTPS service
    on the owner's own Mac's IP. That cost buys a token that no script
    can read.
- **The QR's address has to be reachable.**
  - The QR prints `local_address()` even while the port is bound to
    loopback, and with a VPN up that is the VPN's address (Facts,
    Settled, line 9).
  - While the port is on loopback, the dialog says the phone cannot reach
    it. Choosing an interface is a follow-up.

## The record

- **No new file, and no schema change.**
  - `devices.json` is unchanged. A browser is a `PairedDevice` whose
    `name` the page chose.
  - `identity.json` keeps its layout (`IDENTITY_SCHEMA_VERSION` is 1 at
    `src/ports/remote.rs:17`; store `SCHEMA_VERSION` is 16 at
    `src/core/model.rs:12`). Neither is bumped. Only the value of
    `certificate` changes when the certificate is reissued.
- **Reissue** (`src/adapters/remote.rs`):
  - `make_identity` issues through a new helper, `issue_certificate(key,
    server_id, names, now)`. `names` comes from `certificate_names()`:
    `127.0.0.1`, `localhost`, and `local_address()` when one exists. A
    fresh identity is therefore compliant from the start.
  - `certificate_facts(der) -> Option<CertFacts { not_after, ips }>`
    extends the DER walker that `spki_sha256_of_cert` (`:247`) already
    uses. It reads the validity and the iPAddress entries of the SAN
    extension. A certificate with no SAN yields no IPs.
  - `reissued(identity, key_pem, now) -> Option<Identity>` returns a new
    identity when any of these holds:
    - `certificate_facts` fails;
    - a name from `certificate_names()` is missing;
    - fewer than 30 days remain.
  - The key, `spki_sha256`, `server_id` and `created` are unchanged.
- **Where it runs.** In `SwitchboardApp::bind_remote` (`src/app.rs`,
  after the key is read and before `tls_config`):
  - The new identity is saved straight through
    `services.remote.save_identity`, as `make_identity` already does
    (`src/app.rs:2860`).
  - It is then handed to the core with `AppAction::CertificateReissued`,
    and TLS is built from it.
  - A failed save is logged, and the port listens with the old
    certificate. A reissue never blocks listening.
  - Startup binds through this same path, so no extra call is needed in
    `load_identity`.
- **Rehydration.** Nothing new on the host, because the page is in the
  binary. On the device, two things are stored:
  - The token, in an `HttpOnly; Secure; SameSite=Strict` cookie that no
    script can read.
  - Under `localStorage["switchboard:" + server_id]`, the public
    `{spki_sha256, device_id}`. These are not secrets. The page uses them
    only to tell "identity changed" apart from "not paired".
  - An address change is a new origin, so the device pairs again. The
    old record stays in the dialog until the owner revokes it.

## The signals

No new host signal. The page reacts to signals that already exist:

| Signal | Page state |
|---|---|
| `/info` key differs from the URL's `key`, or `id` differs | "This address answers with a different key. Nothing was sent." |
| `/info` key differs from the stored key | "This Mac's identity changed. Pair again." |
| `GET device` 401, with a stored key that matches | "This device is no longer paired. Pair again from the Mac." |
| `GET device` 401, nothing stored | "Not paired" (below) |
| `/pair` 403 `pairing_closed` / `wrong_code` / `wrong_key` | The host's message, and "open pairing on the Mac and scan again" |
| socket `hello`, then `changed` / `removed` / `resync_required` | §4.4 reconcile, with polling off |
| socket close or error | `GET device`: a 401 is the revoked state; otherwise poll, and retry the socket after 5, 10, 30 and then 60 s |
| a 403 `scope` on `send` | Read `device` again and hide the box |
| `/info.version` differs from the version at load | `location.reload()` |

The page reads `device` again on load, on every `hello`, and after any
403 `scope`. A scope raised in the dialog therefore shows at the next
reconnect or reload.

## Actions and effects

**Core**
- `AppAction::CertificateReissued(Identity)` replaces `identity` and
  leaves the devices, the pairing window and the rotation flag alone. It
  emits no effect, because the app saved the file before dispatching, as
  `IdentityLoaded` does. Routed in `src/core/action.rs:1248-1257` to
  `remote_action`.
- `pub fn pairing_url(identity: &Identity, code: &str, address:
  Option<SocketAddr>) -> Option<String>` in `src/core/remote.rs`. It is a
  pure format:
  - `https://<ip>:<port>/#code=<code>&key=<spki>&id=<server_id>`, with an
    IPv6 address in brackets;
  - `None` when there is no address.

**Adapter** (`src/adapters/remote.rs`). These types are adapter types,
not contract.
- `RemoteRequest` (`:359-371`) gains four fields:
  - `host: Option<String>`, kept only if it matches
    `[A-Za-z0-9.:\[\]-]{1,255}`;
  - `origin: Option<String>`;
  - `page: bool`, true when the request carries `X-Switchboard-Page: 1`;
  - `cookies: Vec<(String, String)>`, only those whose name starts with
    `__Host-switchboard-`. Like `bearer`, these are never logged; `refuse`
    logs only the method and path (`src/app.rs:3754-3772`).
  - `read_request` (`:444-516`) fills these fields.
- `RemoteReply` (`:389-393`) gains `set_cookie: Option<String>`, a full
  header value. The 28 literals in `app.rs` gain `set_cookie: None`.
  `respond` (`:530-553`) writes the header when the field is set.
- `respond` also adds `Cache-Control: no-store` and
  `X-Content-Type-Options: nosniff` to every JSON reply.
- `session_cookie(server_id, token)` and `cleared_cookie(server_id)`
  build the header:
  - `__Host-switchboard-<server_id>=<token>; Path=/; Secure; HttpOnly;
    SameSite=Strict; Max-Age=34560000`;
  - the cleared form has `Max-Age=0`.
- **Paths outside `/api/v1/` never reach the frame.** In `connection`
  (`:770-876`), before the rate check and `try_send`, a request whose
  path does not start with `/api/v1/` is answered on the connection
  thread from `web::PAGE`, the way the 429 is:
  - `GET /`, `/app.js` or `/app.css` gets 200 with that file's headers;
  - any other method on those paths gets 405;
  - any other path gets 404 with the same `not_found` JSON body that
    `serve_remote` gives today, so
    `the_allowlist_refuses_everything_else_with_no_side_effect`
    (`tests/remote.rs:682`) keeps passing for `/api/v2/projects`.
  - Every one of these replies carries `Connection: close`, and the
    connection ends after it. The page's file loads therefore free their
    unauthenticated slot at once, and a burst of them cannot hold the
    four slots that `/pair` needs (Facts, Unresolved, line 39: build so
    that static files never starve `/pair`).
  - No rate window applies. These replies cost the frame nothing; the
    pool and the 5 s deadlines bound them.
  - The `strip_prefix` refusal in `serve_remote` (`src/app.rs:3023`)
    stays as a guard.
- **`src/adapters/web.rs`** (new) holds `PAGE: [(path, content_type,
  body); 3]`, built with `include_str!("../../web/…")`, and
  `headers(host: Option<&str>) -> Vec<(&str, String)>`:
  - `Content-Type`:
    - `text/html; charset=utf-8`;
    - `text/javascript; charset=utf-8`;
    - `text/css; charset=utf-8`.
  - `Content-Security-Policy: default-src 'none'; script-src 'self';
    style-src 'self'; img-src 'self' data:; connect-src 'self'
    wss://<host>; base-uri 'none'; form-action 'none'; frame-ancestors
    'none'`.
    - `wss://<host>` is listed explicitly because CSP's `'self'`
      covering `wss` is not in the facts file.
    - When `host` is missing, the directive is `connect-src 'self'`
      alone, and polling still works.
  - `Cache-Control: no-store`. Nothing is cached, so no version
    mismatch survives a reload.
  - `X-Content-Type-Options: nosniff`.
  - `Referrer-Policy: no-referrer`.
  - `Connection: close`.

**App** (`src/app.rs`)
- `authenticate` (`:3127-3157`) takes the token in this order:
  1. `bearer`, as today, for native clients;
  2. otherwise the cookie named `__Host-switchboard-<server_id>`, only
     when one of these holds:
     - `request.page` is set and this is not an upgrade;
     - this is an upgrade and `origin == "https://" + host`.
  - A cookie that fails either check is treated as no token.
  - A 401 to a request that presented this host's cookie carries
    `cleared_cookie`.
- `remote_pair` (`:3606-3674`) handles a body with `"cookie": true` this
  way: the reply leaves out `token` and sets `session_cookie`, so the
  page's script never sees the token. Without that field, the reply is
  as today.
- `("GET", ["device"])` comes right after `authenticate`, beside
  `events` (`:3036`). It answers `{device_id, name, scope}` in the
  envelope, with `authenticated: true`.
  - A cookie-authenticated reply sets `session_cookie` again, so a page
    in use never reaches the cookie's 400-day `Max-Age`.
- `remote_info` capabilities (`:3580`) gain `"device"` and `"page"`.
- `bind_remote`: the reissue above.

**Native text path**
- `PairingInvite.parse` (`client/Sources/SwitchboardProtocol/Info.swift:114`)
  works like this:
  - Text that starts with `https://` is a URL. The host and port become
    the one address. The fragment's `code`, `key` and `id` become
    `code`, `spkiSha256` and `serverID`. A missing parameter is a parse
    error.
  - Any other text is decoded as JSON, as today.
  - `switchboard-remote pair` (`main.swift:111`), the headset's paste
    (`BrowserView.swift:883`) and `SWITCHBOARD_INVITE` (`:106`) all go
    through `parse`, so none of them changes.
- One QR, one text: the dialog no longer produces the JSON form.
  Accepting JSON in `parse` keeps old pasted invites and the fixture
  working.

## Browser trust (decision 1)

- **Choice: the click-through on the owner's own network is the trust
  step.**
  - It works in every browser.
  - A downloaded certificate works only on iOS, and only after a manual
    switch in Certificate Trust Settings (Facts, Settled, line 23).
    Whether Android trusts one is unresolved (Facts, Unresolved,
    line 37).
  - Offering a certificate download is a follow-up for the owner.
- **What the URL's `key` buys.** A genuine page sends the code only to a
  host whose `/info` key matches. That protects against an honest wrong
  host: a stale address, or a second instance on another port.
- **What it does not buy.** An impostor that wins the click-through
  serves its own script, and that script is not the genuine page.
  - The code never reached any server in the request line, because it
    sits in the fragment.
  - An impostor that relays `/pair` gets a reply with no token unless it
    terminates TLS itself, and the host refuses a key that is not its
    own (spec §5).
  - What is left is an active attacker on the LAN at the moment of the
    first scan. Accepting the warning is accepting that risk, and the
    dialog says so.
- **§5's rule, amended.** A non-loopback bind serves nothing before
  pairing except `/info`, `/pair`, and the three page files. The page
  files carry no state.

## The UI

**The Mac** (`src/ui/remote.rs`)
- The pairing window:
  - The QR and "Copy invite" (`:154-207`) carry `pairing_url(…)` instead
    of the JSON.
  - When there is no address, there is no QR, and the line reads "No
    network address; pair by typing the code".
  - When `!identity.listen.lan`, a line under the QR reads "A phone
    can't reach this Mac until Allow devices on the local network is
    on."
- Under the devices list, beside the redaction sentence (`:73-80`):
  "A browser can't check this Mac's key the way the headset does. The
  certificate warning you accept on a phone is the trust step. Accept it
  only on your own network, right after scanning."
- The devices list is otherwise unchanged. Browsers appear by the name
  they chose, with scope and Revoke as today.

**The page** (`web/index.html`, `web/app.js`, `web/app.css`). Plain DOM,
no inline script or style, and `<link rel="icon" href="data:,">` so the
browser asks for no favicon.
- **Header:**
  - the host's `server_name`;
  - the device's scope as a "Read-only" or "Control" chip;
  - "Live" or "Live updates unavailable".
- **On load:**
  1. Take `location.hash`, then `history.replaceState(null, "", "/")`
     before anything else runs.
  2. `GET /api/v1/info`.
  3. Choose the state:
     - With an invite: compare `key` and `id` with what `/info` presents,
       show "Pair as [name] · Pair", and post `/pair {code, device_name,
       spki_sha256: key, cookie: true}`. Store the public record and go
       on.
     - Without an invite: compare the stored key with `/info`'s, then
       `GET device`.
  4. Open the socket, and poll until it says `hello`.
- **Unpaired with no invite:** "Not paired. On the Mac, open Remote
  devices, choose Allow pairing, then scan the code with this device."
- **List:**
  - The page reads `projects`, `projects/{id}/sessions` for each
    project, and `waiting`.
  - Waiting sessions come first in a group of their own, each with a dot
    and its reason.
  - Below them, the sessions are grouped by project. Each row shows the
    name, the kind, the `card` word and liveness, and an excerpt where
    one was read.
- **Session view:**
  - A Claude Code session shows `last-response` (Codex has no parser;
    design.md "Known gaps"). Any other session shows `screen?lines=60`.
  - Over the socket, the page sends `watch {conversation|screen, id}` and
    fetches after `watching`, refetching on `changed`. With no socket,
    it reads every 5 s.
- **Reply box:**
  - Shown only when `scope == "control"`.
  - Posts `sessions/{id}/send {op: crypto.randomUUID(), text}`.
    `randomUUID` is available because the page is a secure context
    (Facts, Settled, line 12).
  - On a timeout, the page reads `ops/{op}` and follows §4.5's rule. It
    never resends blindly.
- Every `fetch` sends `X-Switchboard-Page: 1` and
  `credentials: "same-origin"`. All API paths live in one `API` object
  at the top of `app.js`, which is what the test reads.

## The tests

**Core** (`src/core/tests.rs`)
- `CertificateReissued` replaces the identity, keeps the devices, the
  open window and `identity_rotated`, and emits nothing.
- `pairing_url`:
  - IPv4 gives the exact string;
  - IPv6 is bracketed;
  - no address gives `None`.

**Adapter units** (`src/adapters/remote.rs`)
- `make_identity`'s certificate:
  - `certificate_facts` reads `127.0.0.1` in its SAN and a `not_after`
    no more than 825 days out;
  - its SPKI still equals `spki_sha256`.
- `reissued`:
  - an old-style certificate (no SAN), a near expiry, or a missing
    address each yield a new certificate with the same SPKI;
  - a current certificate yields `None`.
- `read_request` keeps `host`, `origin`, `X-Switchboard-Page` and only
  the `__Host-switchboard-` cookies, and drops a malformed `Host`.

**Integration** (`tests/remote.rs`)
- The `http` helper (`:245-296`) returns the response headers and takes
  extra request headers.
- `the_page_is_served_with_its_headers_and_references_only_served_paths`:
  - `/`, `/app.js` and `/app.css`, unpaired, on a listener, each get 200
    with:
    - the `Content-Type` above;
    - the CSP, with `wss://localhost` for `Host: localhost`;
    - `Cache-Control: no-store`, `nosniff` and `Connection: close`.
  - Every `src=` and `href=` in `index.html` (except `data:`) is one of
    the served paths.
  - Every `/api/v1/…` template in `app.js`'s `API` object, with seeded
    ids filled in, answers a paired request with something other than
    `404 not_found`.
- `a_path_outside_the_api_is_answered_without_the_frame`:
  - `/favicon.ico` gets 404 `not_found` with `Connection: close`;
  - `POST /` gets 405;
  - `port.poll` stayed empty.
- `a_browser_pairs_by_cookie_and_reads_and_subscribes_with_it`:
  - `/pair` with `cookie: true` has no `token` in the body, and its
    `Set-Cookie` matches the attributes above.
  - `GET device` with the cookie and the page header gets 200 and the
    scope; with the cookie and no page header, 401.
  - An upgrade with the cookie and `Origin: https://localhost` gets
    `hello`; with another origin, 401.
  - After a revoke, `GET device` gets 401 with a cleared cookie.
  - A bearer request still works.
- `device_reports_the_scope_as_the_dialog_sets_it`: `GET device` follows
  `SetDeviceScope`.
- `a_page_load_burst_leaves_room_for_pair`: four static GETs in flight at
  once, then `/info` and `/pair`, all succeed.

**UI** (`tests/ui.rs`)
- With the pairing window open and `listen.lan` false, the
  "can't reach" line is shown.
- The trust sentence is present.
- "Copy invite" copies text that starts with `https://`.

**Swift** (`swift test` in `client/`)
- `testInviteParsesTheQRText` stays.
- A new `testInviteParsesTheURL` covers an IPv4 URL and a URL missing
  `key` (an error).

**Fixtures:** none change. No wire view changed, and only the page reads
`/device`.

**Live acceptance.** The owner runs it, on a scratch data directory.
- Setup: `SWITCHBOARD_SCRIPT` with `remote-listen lan 0` and `pairing
  <code>`, plus `SWITCHBOARD_DATA_DIR` and `SWITCHBOARD_TMUX_SOCKET`.
- Steps:
  1. Scan the QR with an iPhone and accept the warning.
  2. Tap Pair and see the sessions.
  3. Run `echo '{"tool_name":"AskUserQuestion"}' | switchboard-hook
     PermissionRequest` for a session. It moves to the top within 5 s
     whether or not the socket connects.
  4. Send a reply from the phone and see it arrive in the pane.
  5. Revoke the phone in the dialog. The page says "no longer paired" on
     its next request.
- These steps also answer the owner questions in the facts file (Facts,
  For the owner, lines 44–49). Each "no" is already covered by the
  default.

**Docs, in the same commit**
- `docs/remote-client.md`:
  - §4.1: paths outside the API answered on the connection thread, with
    `Connection: close`.
  - §4.3: rows for `/`, `/app.js`, `/app.css` and `GET /api/v1/device`.
  - §5: the pre-pairing list, the cookie for the page, the URL with its
    fragment, the browser trust step, and the compliant certificate.
- `docs/design.md`, Remote client section: a "Web page on the port"
  bullet before "Known gaps" (`:3573`), listing what is left out.
- `README.md`:
  - Layout rows for `web/` and `src/adapters/web.rs`.
  - The dev-aid paragraph (`:270-279`): "Copy invite" is now the URL,
    and the URL opens in a browser.
- `CHANGELOG.md` Unreleased: "A web page on the remote port: pair a
  phone or tablet by scanning the QR, see waiting sessions, read the
  last response or screen, and reply."

## Follow-ups for the owner

- Offering the certificate for download, for iOS full trust.
- Choosing the interface the QR uses when a VPN is up.
- An excerpt field on `SessionView`, so every row has one.
- Renaming a device in the dialog.
- A `.local` SAN, if the owner's devices resolve it.

## Commit

Serve a web control surface from the remote port, paired from a URL in the QR

Contract additions:
- `AppAction::CertificateReissued(Identity)`.
- `core::remote::pairing_url(&Identity, &str, Option<SocketAddr>) -> Option<String>`.
- Wire, additive:
  - `GET /api/v1/device` → `{device_id, name, scope}`;
  - the `/pair` body's optional `cookie: true`, under which the reply
    leaves out `token` and sets `__Host-switchboard-<server_id>`;
  - `info.capabilities` gains `"device"` and `"page"`.
- No change to `model.rs`, `ports/`, or any schema version.
