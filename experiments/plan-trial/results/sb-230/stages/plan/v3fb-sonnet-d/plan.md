# Plan: a browser page on the remote port (issue #230)

Line numbers are as they are in this worktree at 7fe8949. "facts" is
`inputs/facts.md`; a citation is `facts:N` for its line, or "Unresolved
N" / "O1" for its numbered entries.

One commit. The app runs at every step, and the page is added behind
paths that were 404 before, so a host without the page's files in use
behaves as it does today.

## Where this departs from the issue

1. **The socket is the enhancement, not the transport.** The issue has the
   page open `/api/v1/events` directly. On iOS a click-through does not
   carry `wss://` (facts:8, two sources), and whether iOS offers a
   click-through at all is open (Unresolved 1). The page therefore works
   with `fetch` alone, polling, and uses the socket when it connects. The
   acceptance step "a session that starts waiting moves to the top
   without a reload" holds in both modes; polling makes it a few seconds
   rather than immediate.
2. **No cookie.** The issue's first candidate for the token (an `HttpOnly`
   cookie that `fetch` and the socket carry) rests on Unresolved 4 and 5
   and on a Safari bug that drops `SameSite=Strict` cookies from
   WebSockets in restored tabs (facts:17); cookies are also shared across
   ports on one host (facts:14). The plan uses the second candidate: the
   bearer header for `fetch`, `Sec-WebSocket-Protocol` for the socket.
3. **"The one-line excerpt the card shows" is not on the wire.** The Stop
   hook's `last_message` reaches `EventKind::Stopped` but the core keeps
   only `pending` (`src/adapters/hooks.rs:196`, `src/ports/events.rs:24`;
   facts:18), and `SessionView` (`control/src/lib.rs:636`) has no excerpt
   field. §4.3's sentence that the excerpt is a metadata field of the
   session is wrong today. Fetching `last-response` per listed session
   would put one watch per session against a cap of 128 (`src/content.rs:42`).
   The list line is therefore `card`, `kind` and, for a waiting session,
   `waiting_reason`, all on `SessionView`. The excerpt is a later issue; the
   spec sentence is corrected in this commit.
4. **No endpoint tells a device its scope after `/pair`** (`remote_info`,
   `src/app.rs:3568`, is unauthenticated). The header's scope and the
   reply box would go stale after a scope change in the dialog. The plan
   adds one small authenticated read, `GET /api/v1/device`. It is the only
   new read; the rest of the page uses endpoints that exist.
5. **§5's relay sentence does not hold for a browser.** The spec says the
   host's `spki_sha256` check makes a relayed pairing fail. That is true
   for a client that pins the connection. A page cannot read the key of the
   TLS connection it is on; the key it compares is the one `/info` *says*,
   and under a relay the body of `/pair` is written by the relay's page.
   The check catches a genuine page talking to the wrong genuine host
   (a second installation at that address, a rotated key) and nothing
   more. The spec text (§5) and the dialog say so.
6. **"Revoked" needs no new refusal code.** `authenticate` gives the same
   `unauthorized` code for "no token" and "unknown token"
   (`src/app.rs:3125-3153`). The page tells them apart itself: it keeps a
   non-secret record of what it paired with, and a 401 while holding a
   token and a matching `/info` identity means revoked.
7. **Pairing is already single-use.** The issue's QR "stays open five
   minutes or three wrong codes"; `AppAction::Paired` also closes the
   window (`src/core/remote.rs:61-63`), which is what makes a URL that
   leaked into browser history worthless after the first pairing (see
   Decision 1's note on Unresolved 8).
8. **The certificate is not browser-grade today and must be reissued.**
   `make_identity` (`src/adapters/remote.rs:95-127`) sets no
   subjectAltName, no EKU and no validity; the "year 4096" comment at `:91`
   is unverified (facts:31). iOS needs a SAN, `serverAuth` and at most 825
   days (facts:7); Chrome and Safari match on SAN (facts:10). The tree also
   answers the half of Unresolved 7 the facts file left open:
   `rcgen::KeyPair::from_pem` exists and is used at
   `src/adapters/remote.rs:139` and `:146`, so a certificate can be reissued
   over the stored key, and the SPKI pin is unchanged (spec §5).

## The record

No new record kind, no `model.rs` change; `SCHEMA_VERSION` (`src/core/model.rs:12`,
16 today) and `VIEWS_SCHEMA_VERSION` (`:612`, 7 today) stay.

- **A browser is one more `PairedDevice`** (`src/ports/remote.rs:96-107`)
  in `devices.json`, which has no schema version. Its name is the page's
  `device_name` in the `/pair` body (`"Safari on iPhone"`; cut to 64
  characters in `AppCore::new_device`, `src/core/remote.rs:166-175`).
  Scope is the window's, as today. No `kind` field: the name is enough
  (Decision 6), and listing and Revoke in the dialog already exist.
- **`Identity` (`identity.json`) gains two `#[serde(default)]` fields**:
  `names: Vec<String>`, the subjectAltName entries the stored certificate
  was issued with, and `issued: Option<SystemTime>`, when. Both are needed
  to decide whether to reissue without parsing the certificate. Bump
  `IDENTITY_SCHEMA_VERSION` (`src/ports/remote.rs:17`, 1 today) to the
  value there at implementation time, plus one: a ticket landing in
  parallel may take the next number first. Nothing reads that number (`RemoteFiles::load`,
  `src/adapters/remote.rs:55-64`, deserializes without checking it), so the
  bump is the contract note, and an older file loads with empty `names`,
  which means "reissue". Literals of `Identity` to update: `src/adapters/remote.rs:117`,
  `src/core/tests.rs:11769`, `tests/ui.rs:7436`.
- **Rehydration.** The reissued certificate is written through the existing
  atomic `save_identity` and comes back with `IdentityLoaded`. The key
  stays in the Keychain under `remote-key:<server_id>` and is not touched.
  The page itself is not a record: it is bytes compiled in.
- **On the device** (not a host record): `localStorage` under one key holds
  `{server_id, spki_sha256, device_id, name, token}`, per origin. The token
  is readable by any script on the origin; the CSP below is what makes
  "any script" mean the two files the host ships. If the browser drops the
  storage the page is simply unpaired and says so (facts Unresolved 3: the
  design assumes nothing survives a launch, including the certificate
  exception).
- **The certificate file for the device.** Public data, written on a button
  click to `<data dir>/switchboard-certificate.crt` (DER) through
  `JsonStore::write_atomic`, then revealed in Finder so it can be AirDropped
  or served to the device. Not a record, never read back.

## The signals

No new event kind, no new `changed` kind. The page consumes what §4.4
already emits.

- **Socket handshake (changed).** `read_request` (`src/adapters/remote.rs:444-513`)
  discards every header but `Authorization` and `Sec-WebSocket-Key`.
  `RemoteRequest` (`:359-372`) gains `origin: Option<String>`,
  `host: Option<String>` and `subprotocol: Option<String>`; the literal is
  built only at `:506` (facts:20), so no test breaks.
  - *Token transport (Decision 2).* A browser cannot set a header on
    `new WebSocket()` and the token never goes in a URL (§5). The page opens
    `new WebSocket(url, ["switchboard.v1", token])`. `read_request`, on an
    upgrade request only, takes the `Sec-WebSocket-Protocol` list: when it
    contains `switchboard.v1`, the other entry becomes `bearer` (unless an
    `Authorization` header already set it) and `subprotocol` is
    `Some("switchboard.v1")`. `authenticate` is unchanged. The token is
    base64url, a legal `token` value (facts:12). `respond_upgrade`
    (`:519-527`) echoes `Sec-WebSocket-Protocol: switchboard.v1` when
    `subprotocol` is set and **never the token**; a browser fails a
    handshake that offered protocols and got none back (facts:11). The
    header is dropped after extraction, so it is never logged.
  - *Origin (facts:13).* `remote_subscribe` (`src/app.rs:2723`) refuses a
    request that carries an `Origin` unless it equals `https://` plus the
    request's own `Host`, case-insensitively: 403 `bad_origin`. This is
    same-origin by construction, so it holds for the IP, a `.local` name or
    loopback without a list. A request with no `Origin` (every native
    client) is accepted as before. POSTs need no check: the credential is a
    header a cross-site page cannot set, and the host answers no
    `OPTIONS`.
- **Page behaviour that follows from the signals.** After `/device`
  succeeds the page opens the socket and waits five seconds for `hello`.
  No `hello`, or any close, puts it in polling mode (the list every ten
  seconds, an open session every five); it retries the socket every sixty
  seconds. In socket mode it follows §4.4 as the headset does: wait for
  `hello`, then snapshot, apply a reply only at or past the revision held
  for that resource key, refetch the list on `changed` of `session`,
  `sessions`, `waiting` or `projects` (debounced 300 ms), send
  `{"type":"watch","resource":{"kind":"conversation"|"screen","id":<session>}}`
  on opening a session and fetch after `watching`, `unwatch` on leaving,
  and resend watches after a reconnect (`subscriber_frame`,
  `src/app.rs:1700-1745`). `resync_required` reconnects and refetches
  everything.
- **Decision 4's pool and rate answer.** A page load is `/`, `/app.js`,
  `/app.css`: three requests, sequential on the first connection and at
  most two more connections once the HTML names the other two. They sit in
  the unauthenticated pool of 4 (`MAX_UNAUTHENTICATED`, `:567`) until the
  first request with a token moves the connection to the paired pool
  (`:834-849`). The page ships `<link rel="icon" href="data:,">` so no
  `/favicon.ico` request is made. Browser connection counts are
  unmeasured (Unresolved 6, O5): the design is "three files, no more", and
  the constant stays 4 until someone measures a phone. Static paths get a
  rate window of their own, `STATIC_PER_MINUTE = 120` per listener
  (`Limits::limited`, `:629-640`), 429 past it, like `/pair` and `/info`.

## The actions and effects

- **Page files (Decision 4).** `web/index.html`, `web/app.css`,
  `web/app.js`, no build step, no dependency. New `src/web.rs` holds the
  table: path, content type, bytes from `include_str!`, plus
  `CERTIFICATE_PATH` and the CSP string. Exact paths:
  `/` (`text/html; charset=utf-8`), `/app.js` (`text/javascript; charset=utf-8`),
  `/app.css` (`text/css; charset=utf-8`),
  `/switchboard.crt` (`application/x-x509-ca-cert`, the DER of the identity's
  certificate; the only one that is not a constant). `GET` only; any other
  method is 405 `method_not_allowed`; any other path outside `/api/v1/` stays
  404 `not_found` JSON. A query on `/` is ignored.
- **`serve_remote`** (`src/app.rs:3017`): a new arm *before* the
  `/api/v1/` prefix check (`:3023`) answers the table. It runs on the
  frame, not the connection thread, because the certificate needs the
  identity; the static rate window above is what bounds its cost. It logs
  method and path, never the query (as `refuse`, `:3754`, does), so a
  `?code=` that reaches the host is not in the log. The files do not vary
  by host, so serving them before pairing leaks nothing; the certificate
  is public (`Identity::certificate` is documented as such).
- **Response headers** (`respond`, `:530-552`). A new `respond_page` writes
  the file's content type and, on every page response,
  `Cache-Control: no-store` (a page must not outlive the host version that
  matches its API), `X-Content-Type-Options: nosniff`,
  `Referrer-Policy: no-referrer` (the URL that carried the code is never
  sent on), and
  `Content-Security-Policy: default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'`.
  No inline script, no inline style, no external resource. `connection()`
  chooses `respond_page` by looking the request's path up in `web.rs`
  (the path is cloned before the request moves into `RemoteIncoming`,
  `:762`). `respond` also gains `Cache-Control: no-store` for JSON, since a
  `/pair` reply carries a token. `RemoteReply` (`:388`) does not change,
  so its literals in `src/app.rs` stay.
- **`GET /api/v1/device`** (authenticated, any scope): the device behind
  the token as `authenticate` sees it now: `{device_id, name, scope}` in
  the usual envelope. New arm after `authenticate` in `serve_remote`; the
  allowlist test list gains it. §4.3's table gains the row.
- **Pairing URL (Decision 3: one form).** A pure function in
  `src/core/remote.rs`, `pairing_url(ip, port, code, spki_sha256, server_id)`,
  gives `https://<ip>:<port>/?code=<code>&key=<spki_sha256>&id=<server_id>`,
  bracketing an IPv6 address. Code, hash and UUID need no percent-encoding.
  The QR and "Copy invite" carry it; the JSON form is no longer produced.
  Swift keeps reading it because eight test files and the fixtures build
  invites from JSON, and an older pasted invite should still work:
  `PairingInvite.parse` (`client/Sources/SwitchboardProtocol/Info.swift:93-116`)
  tries `URLComponents` first (`code`, `key`, `id`; `addresses` is the URL's
  host and port, bracketed for IPv6) and JSON second. The `switchboard-remote`
  usage text (`main.swift:12`) says "invite URL". `ConnectionManager` and
  `BrowserView` call `parse` and do not change.
- **Certificate reissue (Decision 1's mechanism).** `make_identity` and a new
  `reissued(identity, key_pem, current_ip, now) -> Option<Identity>` share one
  issuing function that sets every field explicitly (facts:31): SAN entries,
  `not_before` now, `not_after` 800 days on (inside iOS's 825, facts:7),
  `serverAuth` EKU, and `is_ca` so the same file works as an installed
  profile (facts:8, the code-server guide). The SAN set is `localhost`,
  `127.0.0.1`, `::1`, the outward address from `local_address()`
  (`:962-970`), and up to four earlier addresses kept from `names`, so a
  DHCP lease that returns does not invalidate an installed certificate.
  `reissued` returns `Some` when the current address is not in `names`, or
  `issued` is absent or older than 400 days; the key is `KeyPair::from_pem`
  and the new certificate's SPKI is asserted equal to `identity.spki_sha256`
  before returning (a mismatch returns `None`, never a new pin). The rcgen
  0.14 API (`subject_alt_names`, `not_after`, `extended_key_usages`,
  `is_ca`, whether `CertificateParams::new` turns a string that parses as an
  IP into an IP SAN, and whether `time` is re-exported) is not read in this
  session; compile it before relying on it.
  - *Where it runs.* In `bind_remote` (`src/app.rs:2936`) after the key is
    read (`:2951-2956`) and before `tls_config` (`:2957`), the one place
    that runs at launch and on every rebind. The app saves the file first,
    then dispatches a new `AppAction::CertificateReissued(Identity)`, which
    only sets `self.identity` and emits **no effect**, the order
    `make_identity` and `IdentityLoaded` already use. Dispatching
    `SaveIdentity` here would recurse: `run_remote_effect` rebinds when
    `remote_bound` differs from the listen, and `bind_remote` has just set it
    to `None` (`:2938`). A failure to reissue is logged and the old
    certificate is served: the native clients pin the key and are unaffected.
  - *What an address change costs.* A new address means a new certificate,
    so a certificate already installed on a device stops matching until the
    owner installs the new file. The dialog says so.
- **`AppAction::ExportCertificate`** and `Effect::ExportCertificate(Identity)`:
  the dialog's "Save certificate" button. The arm in `app.rs` writes the DER
  through a new `RemoteStore::export_certificate(&self, der: &[u8]) ->
  Result<PathBuf, String>` (real: `write_atomic` into the data directory;
  fake: records it), then runs `Effect::Reveal(path)` (exists,
  `src/core/action.rs:881`). Not emitted when `read_only`, as `save_devices`
  does (`src/core/remote.rs:121-125`).
- **Script line** (`src/script.rs`, `["pairing", code]` at `:730` exists):
  no new line. `SWITCHBOARD_SCRIPT` with `pairing ABCD2345` and
  `remote-listen lan <port>` already puts a window and a LAN bind in place
  to drive the page by hand; the README says so in one sentence.

### Trust, settled (Decision 1)

Pick: **the host offers its certificate, and the click-through is accepted
where a browser offers one**; the dialog recommends installing the file
first on iOS. Why: a click-through cannot carry the socket on iOS (facts:8)
and may not exist there at all (Unresolved 1, O1), so a design that needs
it is gated on an open fact; the installed certificate is the only route
that gives iOS the SAN-and-profile trust it requires (facts:7, facts:8).
Neither branch is required for the page to work: with a click-through and
no socket, the page polls; with the certificate installed, the socket
works too. The default is the design that works whichever way O1 and O2
answer. What each buys: the `key` in the URL stops a genuine page sending
the code to a host whose `/info` key differs (item 5 above). It does not
authenticate the page: an impostor serving its own page defeats it. The
trust step is the owner's act of choosing the certificate they saved on the
Mac, or of clicking through on their own network, and the dialog says that
in these words. §5's rule becomes: no non-loopback bind serves anything but
`/info`, `/pair`, the page's own static files and the public certificate
before pairing.

*Unresolved 8 / O4* (does the URL stay in history): not relied on. The page
runs `history.replaceState` as the first statement of `app.js`, before it
reads anything else, and keeps the three values in a closure, never in
storage. If the browser recorded the URL first, the code is single-use
(item 7) and expires with the window.

*Unresolved 2 / O2* (`isSecureContext` after a click-through): not relied
on. Operation ids come from `crypto.getRandomValues` formatted as a UUID
(facts:16), 36 characters against `OP_CAP` of 64 (`src/app.rs:4260`).

## The UI

### The Mac (`src/ui/remote.rs`)

- The pairing window builds `pairing_url` (`:155-173`) instead of the JSON
  and uses it for the QR and for "Copy invite" (`:199-206`). The line under
  the code shows the page address and says: "Scan with the device's camera.
  The first visit shows a certificate warning; clicking through is your
  trust step on this network. On iOS, install the certificate first
  (Save certificate…): a click-through does not carry live updates."
  A "Save certificate…" button dispatches `AppAction::ExportCertificate`.
- The paired-devices dialog's footer sentence (`:72`) gains: "A browser
  holds its token in the page's local storage; revoke it here. The page is
  served by this Mac, so a device that reached a different host at this
  address would show a different page: check the key line before pairing."
- The IPv6 and wrong-interface cases (the route-chosen address is not the
  phone's) are shown, not solved: the address line is what the QR holds,
  and Copy invite lets the owner edit the host.

### The page (`web/`)

Plain HTML, CSS and JS, text set with `textContent` only (the screen and
last response are agent output; the CSP is the second line, not the first).

- **Opening.** With `?code&key&id`: `replaceState`, then `GET /api/v1/info`;
  `spki_sha256` or `server_id` differs from the URL: stop, say "This is not
  the host whose code was scanned", send nothing. Otherwise `POST
  /api/v1/pair {code, device_name, spki_sha256: key}`. Refusals are named:
  `pairing_closed`, `wrong_code`, `wrong_key`, `rate_limited`. On success
  store the record and show the sessions. Without a code and without a
  stored record: "Open the pairing window on the Mac and scan its code with
  this device's camera." No code-entry form (the typed path is the headset's
  and the CLI's).
- **Later visits.** `GET /info` first. A `server_id` that differs, or the
  same one with a different `spki_sha256`: "identity changed, pair again";
  the token is deleted unsent. Same identity: `GET /device`. A 401 there:
  "revoked, or this host forgot this device; pair again".
- **Header.** The host's `server_name` (from `/info`) and the device's scope
  (from `/device`, re-read at every poll or reconnect and after a refused
  send).
- **List.** `GET projects`, then `GET projects/{id}/sessions` for each (whole
  replies, §4.2); no `scope=all`. Projects holding a waiting session first,
  waiting sessions first within each, marked; each row `name`, `kind`,
  `card`, and `waiting_reason` when waiting.
- **Session.** A Claude Code session shows `last-response` (`final_text`),
  anything else `screen?lines=200`, refreshed as above. `trust_question`
  and `prompt_refusal` are shown as text ("answer the folder question on the
  Mac"); trust and resume are not offered (Not in this cut).
- **Reply.** A text box and Send when `/device` says `control`, absent
  otherwise. `POST sessions/{id}/send {op, text}`; on a timeout `GET ops/{op}`
  and act per §4.5: `done` use it, `interrupted` never resubmit, `unknown`
  resubmit with the same op. A 403 re-reads `/device` and hides the box.
- **Not here:** Dispatch, the conversation beyond the last response, drafts,
  dictation, runs, pinned files, relay or tunnel, a framework or build step.

## The tests

- **`tests/remote.rs`, the host's own HTTP client** (the helper at `:245-296`
  reads only `Content-Length`; it gains a variant returning headers and raw
  bytes, and `http` calls it):
  - the four static paths unpaired: 200, the content types above, the exact
    CSP, `no-store`, `nosniff`, `no-referrer`; `/?code=X&key=Y&id=Z` returns
    the same bytes as `/`; `POST /` is 405; `/favicon.ico` and `/app.js/` are
    404 JSON; JSON replies carry `no-store`;
  - **every path the page references is served**: scan `index.html` for
    `src`/`href`, assert each is in the table and answers 200; scan `app.js`
    for `api/v1/...` literals and assert each matches an allowlisted route
    (so a typo in the page fails CI); assert no inline `<script>`, no
    `http://` or `https://` literal, and no `innerHTML`, `eval(` or
    `document.write` in the three files;
  - `/switchboard.crt` returns the DER whose SPKI hash equals the identity's
    (`spki_sha256_of_cert`);
  - the 121st static GET in a minute is 429, and `/info` is still served
    (separate window);
  - **pairing end to end as the CLI does it, in the page's order**: GET `/`,
    GET `/info`, POST `/pair`, GET `/device` (scope), GET `projects`, POST
    `send` under `control`, and the send is typed once per op; the same code
    is then `pairing_closed` (single-use); a wrong `spki_sha256` is
    `wrong_key` and counts toward the three; revoking the device makes
    `/device` 401 and closes its socket (extends
    `revoke_closes_the_devices_socket`, `:2892`);
  - `/device` follows `SetDeviceScope`;
  - the socket: a handshake with `Sec-WebSocket-Protocol: switchboard.v1, <token>`
    succeeds, the 101 echoes `switchboard.v1` and not the token, `hello`
    arrives; a wrong token is refused; the bearer header still works; an
    `Origin` equal to `https://<Host>` is accepted, any other is 403
    `bad_origin`, none is accepted (extends `subscribe_at`, `:1960`);
  - the allowlist test (`:682`) gains `/api/v1/device` as allowed and
    unauthenticated `GET /api/v1/device` as 401; the unpaired-reads test
    (`:426`) gains the page paths as served.
- **`src/adapters/remote.rs` unit tests**: `reissued` keeps the SPKI, puts
  the address and the fixed names in the SAN, sets `serverAuth`, CA and the
  800-day validity (checked with `x509-parser` as a dev-dependency, already
  locked at 0.18.1; `Cargo.lock`'s root entry changes and is committed),
  returns `None` for an unchanged address and a fresh `issued`, `Some` for a
  new address, an absent `issued` or one over 400 days, and caps the kept
  addresses at four; a pinned client still connects to the reissued
  certificate. `read_request` parses the subprotocol list and `Origin` and
  `Host`; the pool and rate tests keep passing.
- **Through the app** (`tests/remote.rs` with the fakes): a host started on
  an old identity (empty `names`) reissues once, saves once, serves the new
  certificate, and a second rebind changes nothing.
- **Core tests** (`src/core/tests.rs`): `CertificateReissued` sets the
  identity with no effect; `ExportCertificate` emits its effect and, when
  read-only, nothing; `pairing_url` for IPv4 and IPv6.
- **`tests/ui.rs`** (the dialog test at `:7424`): "Copy invite" now yields
  an `https://` URL whose `code`, `key` and `id` match the window and
  identity (the JSON assertions at `:7478-7486` are replaced); "Save
  certificate…" dispatches `ExportCertificate`.
- **Swift** (`client/Tests`, run by `cd client && swift test`, which is not
  in the hook or CI): `PairingInvite.parse` reads the URL (IPv4, IPv6),
  still reads the JSON, and refuses garbage. Fixtures are unchanged: no
  wire view changed.
- **What CI cannot test:** the page's own logic (query handling,
  `replaceState`, the revision rule, the polling fallback) has no JS runner
  and the issue rules out a dependency for one. The scans above catch dead
  paths and unsafe constructs; the logic is checked by hand.

### Acceptance, by hand

Drive a test data dir with `SWITCHBOARD_SCRIPT` (`pairing ABCD2345`,
`remote-listen lan <port>`) and short `SWITCHBOARD_DATA_DIR`.

1. Open the pairing window, scan the QR with a phone. Where the browser
   offers a warning, accept it; where it offers none, use Save certificate…,
   AirDrop the file, install and trust it, scan again. The sessions list
   appears.
2. Start a session waiting (`switchboard-hook PermissionRequest` per
   CLAUDE.md): it moves to the top and is marked, within a poll interval at
   worst, immediately when the socket is up.
3. Open a session, send a reply from a `control` device, watch it arrive in
   the pane.
4. Revoke the phone in the dialog: its next request shows "revoked".
5. Record which browsers offered the warning, whether the socket connected,
   and the connection count of one page load, as the answers to O1, O2 and
   O5. Nothing above waits on them.

### Documents

- `docs/remote-client.md`: §4.1 (static files, the extra window, the page
  in the pool of 4), §4.3 (rows for the page paths and `device`), §4.3
  excerpt sentence corrected (item 3), §4.4 (the subprotocol, the `Origin`
  rule), §5 (the pre-pairing list, the URL invite, the relay sentence, the
  bearer-or-subprotocol token, the certificate fields).
- `docs/design.md`: a "Browser page (2026-10-22)" bullet in the "Remote
  client" status section (`:3107`), and the known gaps (`:3560-3583`) updated:
  excerpt on the wire, trust and resume on the page, the owner questions O1
  to O5. "Trust boundary" (`:219`) gains the pre-pairing sentence.
- `README.md`: Layout rows for `src/web.rs` and `web/`; the `adapters/remote.rs`
  and `ui/remote.rs` rows; the Development text at `:277-278` (Copy invite is
  now the URL; `switchboard-remote pair "$(pbpaste)"` still pairs it); the
  data directory contents (`switchboard-certificate.crt`).
- `CHANGELOG.md` under Unreleased: "The remote port serves a web page; the
  pairing code's QR is now a URL a phone's camera opens."
- Nothing in any of these names a client, a customer or their repositories.

## Commit message

`Serve a web page on the remote port, paired from a URL, with a reissued certificate and the token in the socket's subprotocol`

## Contract additions

- `GET /api/v1/device` → `{device_id, name, scope}` (envelope as the others).
- Static `GET /`, `/app.js`, `/app.css`, `/switchboard.crt` served before pairing.
- `Sec-WebSocket-Protocol: switchboard.v1, <token>` accepted on the events
  upgrade, `switchboard.v1` echoed; `Origin` checked against `Host`
  (403 `bad_origin`); `Cache-Control: no-store` on every reply.
- Pairing invite text is a URL (`code`, `key`, `id` query); Swift
  `PairingInvite.parse` reads URL or JSON.
- `Identity.names: Vec<String>` and `Identity.issued: Option<SystemTime>`
  (`#[serde(default)]`); `IDENTITY_SCHEMA_VERSION` from the value in
  `src/ports/remote.rs` at implementation time, plus one; the certificate
  is reissued over the same key (SPKI unchanged).
- `AppAction::CertificateReissued(Identity)`, `AppAction::ExportCertificate`,
  `Effect::ExportCertificate(Identity)`.
- `RemoteStore::export_certificate(&self, der: &[u8]) -> Result<PathBuf, String>`
  (real and fake).
- `RemoteRequest.origin`, `.host`, `.subprotocol` (`src/adapters/remote.rs`).
- `src/web.rs`, `web/index.html`, `web/app.js`, `web/app.css`;
  `core::remote::pairing_url`.
- `SCHEMA_VERSION` and `VIEWS_SCHEMA_VERSION`: unchanged.
