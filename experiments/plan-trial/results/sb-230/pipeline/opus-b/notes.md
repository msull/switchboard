# Issue #230 — a web control surface on the remote port: investigation notes

Read: CLAUDE.md, docs/design.md (Durable store, Trust boundary, the
"Remote client (2026-10-12)" status at 3107–3584), docs/lessons.md,
docs/remote-client.md (§4.1 bounds, §4.3 endpoints, §4.4 events, §5
pairing, §11 tests). Line numbers are as they stand in this worktree
(branch `dispatch/230-…`, base 7fe8949 "Release 0.4.1").

## 1. Record and rehydration first

**Host, on disk.** Nothing new is needed in `model.rs`, so there is no
`SCHEMA_VERSION` bump.
- `devices.json` (`ports::remote::PairedDevice`, `src/ports/remote.rs:96-109`,
  written through `RemoteFiles::save_devices`, `src/adapters/remote.rs:83`,
  the store's atomic path). A browser is one more `PairedDevice`: name,
  scope, `token_sha256`, created, last_seen. The token itself is never
  stored, as today. An optional additive field
  (`#[serde(default)] client: Option<String>`, e.g. `"browser"`) is the
  most this needs, and only if the dialog should label browsers or say
  "pair again in the browser". I'd leave it out of the first cut:
  the device name ("Safari on iPhone") already says it.
- `identity.json` (`Identity`, `src/ports/remote.rs:26-42`) does not change
  unless decision 1 picks certificate download. That option would need a
  reissued certificate with SANs (see §3, D1). A reissue over the same
  key keeps `spki_sha256`, so every pin survives.
- The page itself is not a record. It is `include_str!` constants
  compiled into the binary: `web/index.html`, `web/app.js`,
  `web/app.css`. A new release replaces it.
- The pairing window stays transient core state (`PairingWindow`, opened
  by `AppAction::OpenPairing`, `src/core/remote.rs:43-51`). It is never saved.

**Device, in the browser.**
- The token lives in an `HttpOnly; Secure; SameSite=Strict` cookie that
  the host sets. The page's script never sees it (D2).
- `localStorage` holds only what is not secret: `{server_id, spki_sha256,
  server_name}` from the pairing. It is used to tell "identity changed"
  apart from "revoked" (§3, D6). No host content is ever stored there,
  which keeps §5/F5's rule that a client keeps host bodies in memory only.

**Rehydration of the page** is the headset's §4.4 sequence:
1. `GET /api/v1/info`. If `spki_sha256` or `server_id` differs from the
   stored pair, show "identity changed, pair again" and stop.
2. `GET /api/v1/device` (new, D6/§4 below) for the current scope and
   name. A 401 shows "revoked".
3. Open `wss://<same origin>/api/v1/events` and wait for `hello {epoch, revision}`.
   Buffer the events that arrive in the meantime.
4. Snapshot: `GET /projects`, then `GET /projects/{id}/sessions` per
   project, then `GET /waiting`. Each reply carries its own `revision`.
5. Apply the buffered events by revision, then enable the reply box.

Live updates after that:
- `changed {session,id}`: refetch that session.
- `changed {sessions,project}`: refetch that project's sessions.
- `changed {waiting,""}`: refetch `/waiting`.
- `removed`: drop the item.
- A new `epoch`: refetch everything.
- `resync_required`: reconnect.
- The open session view sends `watch {conversation|screen, id}`, waits
  for `watching`, then fetches. It refetches on `changed`, and every 30 s
  as §4.4 says.

Only after all that comes how the pane looks.

## 2. The code the issue touches

**Adapter (`src/adapters/remote.rs`)**
- `RemoteRequest` (360-373) keeps method, path, query, body, `bearer`
  and `upgrade` only. Cookie, Origin, Host and User-Agent are all
  discarded in `read_request` (444-515; header lookup at 474-494).
  Cookie auth needs `cookie`, `origin` and `host` carried through. The
  cookie must be taken out of the headers the way the bearer is, so it
  is never logged.
- `RemoteReply` (389-393) is `{status, body, authenticated}`. `respond`
  (530-553) hard-codes `Content-Type: application/json` and writes no
  other headers. Static files, `Set-Cookie`, CSP and `Cache-Control`
  need the reply to carry a content type and extra headers.
  `app.rs` builds `RemoteReply { … }` literally in 28 places
  (`grep -c`). So add the fields with a `Default`, or a constructor plus
  `..`, rather than editing 28 literals. `RemoteReply` is an adapter
  type, not the shared contract, so renaming is not at issue. Still,
  extend it additively.
- `respond_upgrade` (519-527) writes no `Sec-WebSocket-Protocol`. That
  matters only if D2 picks the subprotocol route.
- Pools: `MAX_UNAUTHENTICATED = 4` and `MAX_PAIRED = 12` (564-568). The
  accept loop drops past 4 (699-707). A connection moves to the paired
  pool on its first reply with `authenticated: true` (835-848).
- Deadlines: `HANDSHAKE_TIMEOUT` 5 s, `IDLE_TIMEOUT` 30 s, `WRITE_TIMEOUT`
  10 s (570-574).
- Rate windows: `/pair` 6 per minute and `/info` 60 per minute per
  listener (582-587, `Limits::limited` 627-641). Every other path is
  unlimited at this layer.
- `connection` (770-866), `subscriber` (878-927).
- Certificate: `make_identity` (95-127) makes a P-256 self-signed
  certificate with **no SANs** (`CertificateParams::new(Vec::new())`),
  CN `Switchboard <id>`, `not_after` year 4096, and no EKU. That decides
  D1 (below).
- `local_address()` (962) is the IP the QR carries.

**App (`src/app.rs`)**
- `serve_pending` (1572-1600) drains remote requests into `serve_remote`.
- `serve_remote` (3017-3122): `strip_prefix("/api/v1/")` at 3023 answers
  404 for anything else, and that is where the static routes go. `info`
  and `pair` come before auth (3027-3031). `authenticate` (3127-3157)
  reads `request.bearer` only. `events` → `remote_subscribe` (2723-2758).
- `remote_info` (3568-3600), `remote_pair` (3606-3674). The pair reply
  body carries `token` (3657-3664), so a browser variant must set the
  cookie and leave the token out of the body.
- `remote_command` (3159-3220): `send` needs `{op, text}`, control scope
  checked first; a read device gets 403 `scope`.
- `refuse` (3754-3772) logs method and path only, never the query. Static
  routes must keep that.
- Revoke closes subscribers in `dispatch_inner` (480-499).
- `envelope`/`envelope_at`/`error_json` (3730-3750).

**Core (`src/core/remote.rs`)**: `OpenPairing`, `PairingFailed`, `Paired`,
`DeviceSeen`, `RevokeDevice`, `SetDeviceScope` (20-104). `new_device`
(163-174) takes the name as given and caps it at 64 chars. Nothing here
needs to change for a browser.

**Pairing window (`src/ui/remote.rs`)**: the payload JSON is built in
`pairing()` at 160-169. The QR comes from `qr_texture(payload)` (275-295).
"Copy invite" copies the payload (205-212). The help line is at 196-202.
The redaction sentence is in `show` (69-77). The UI test is
`tests/ui.rs:7454-7503`, which clicks "Allow pairing for 5 minutes" and
then "Copy invite".

**Swift text path**: `PairingInvite.parse` (JSON only) is in
`client/Sources/SwitchboardProtocol/Info.swift:92-117`. It is called from
`client/Sources/switchboard-remote/main.swift:111`,
`client/Sources/SwitchboardUI/BrowserView.swift:106` (the
`SWITCHBOARD_INVITE` launch aid) and `:883` (the paste field), and tested
in `client/Tests/SwitchboardProtocolTests/FixturesTests.swift:157-165`.

**Wire views (`control/src/lib.rs`)**: `SessionView` (637-693) has `card`,
`waiting`, `waiting_reason`, `kind` (`SessionKind` 537: claude, codex,
shell, command, service), `liveness` and `project`. **It has no excerpt
field.** The card's one-line excerpt is computed in the UI
(`src/ui/cards.rs:176-181`): it comes from the parsed transcript
(`agent_excerpt`, 558) for agents, and from the caption poll for the
rest. remote-client.md §4.3 says that excerpt is "`last_message` …
a field of the session's metadata", but no such field exists on the
wire. The headset instead reads `last-response` per open window
(`BrowserView.swift:149-153`). See D7.

**Tests**: the helpers in `tests/remote.rs` are `http` (245-296: raw
HTTP/1.1 over a pinned rustls client, returns status and body, parses
only `Content-Length`), `call`/`get` (299-330, which also parse the body
as JSON) and `pair` (406). Existing tests to extend:
`info_is_served_to_anyone_and_every_other_read_is_refused_unpaired`
(426), `revoke_and_a_scope_change_take_effect_on_the_next_request` (658),
`the_allowlist_refuses_everything_else_with_no_side_effect` (682),
`pair_is_rate_limited_per_listener` (1406), fixtures (`write_fixtures`
1587).

## 3. Decisions, with a recommendation each

### D1. Browser trust: click-through on the local network is the trust step

Pick the click-through, and say so in the dialog. Why:
- The certificate has no SAN, a validity to the year 4096 and no
  `serverAuth` EKU. iOS and macOS refuse to *trust* such a certificate
  even when it is installed: they require a SAN, at most 825 days of
  validity and the EKU. A user-installed self-signed leaf also does not
  appear under iOS's Certificate Trust Settings unless it is a CA.
- Making download work therefore means reissuing the certificate. The
  key can stay, so pins survive. But the reissue needs SANs for an
  address that DHCP moves, a CA flag or a separate CA, a renewal
  schedule, and a `.mobileconfig` profile walk-through on iOS. That is
  far more machinery and more owner steps than the feature is worth in a
  first cut.
- A browser exception for a certificate-name error is still offered by
  Safari (iOS and macOS) and Chrome.

What the click-through buys:
- An encrypted channel to whichever host answered first at that
  address. It is trust on first use, on the LAN.
- A genuine page compares `key` with `/info`'s `spki_sha256` before
  posting `/pair`. That catches a stale QR and the wrong host or port.
  It does not catch an impostor that serves its own page.

What it does not buy:
- An active intermediary at the first visit can show its own warning,
  which the owner clicks through just the same, and serve its own page.
  Because a browser cannot pin, a later certificate swap also only shows
  a new warning.
- The dialog should say: "Your browser warns once that it does not
  know this Mac's certificate. Accept it only on the first visit. If it
  warns again for a device you already paired, do not continue: the
  key changed or something is in between." It should also print the
  certificate's SHA-256 (the browsers' certificate viewers show the
  certificate hash, not the SPKI hash) so a careful owner can compare it
  in the viewer before accepting. That is the same comparison by eye the
  headset makes.

**Spike first (CLAUDE.md: a new risky mechanism gets a spike
directory).** `spikes/23-browser-surface`, on Safari iOS, Safari macOS
and Chrome:
- Does the click-through exception cover same-origin `fetch` *and* `wss`?
  Older iOS Safari failed WSS to an accepted self-signed host.
- Is an `HttpOnly; Secure` cookie set under the exception stored and sent
  on the WebSocket upgrade?
- How long does the exception last? Chrome forgets a certificate decision
  after about a week; Safari's lifetime is to be measured. If it is short,
  the owner re-clicks periodically, but the cookie survives, so the device
  needs no re-pairing.
- Does CSP `connect-src 'self'` match `wss:` on each browser?
- Does a phone camera open an `https://<ip>:43187/#…` URL in the browser?
- Is a `.local` host name usable as the origin (see D3)?

§5's rule, amended: before pairing, a non-loopback bind serves `/info`,
`/pair` and the fixed static set (D4) and nothing else. The static
files carry no host data.

### D2. Token to the WebSocket: the cookie

Pick the `HttpOnly; Secure; SameSite=Strict` cookie. Why:
- The script never holds the token, so nothing injected or left in
  `localStorage` can read it.
- `fetch` and the WS handshake carry it with no code in the page.
- `Sec-WebSocket-Protocol` would put the token in script memory and in
  `localStorage` across reloads. Its value is also logged by some
  proxies and devtools.

Details the plan must state:
- **Name the cookie per installation**: `sb_<server_id simple>`. Cookies
  ignore the port, so two instances on one Mac (two data dirs, two
  ports) would otherwise overwrite each other's token.
- Use `Path=/api/v1` and `Max-Age=34560000` (400 days, Chrome's cap). The
  token never expires and Revoke ends it. A server-set cookie is not
  under Safari ITP's 7-day cap for script-written cookies; the spike
  confirms.
- `/pair` gains an opt-in, `{"cookie": true}` in the body. The reply
  then sends `Set-Cookie` and leaves `token` out of the JSON. Native
  clients send nothing new and keep getting `token`.
- `authenticate` takes the bearer header first, then the cookie.
- **CSRF and cross-site WebSocket hijacking**: any request
  authenticated *by cookie* must carry `Origin` equal to
  `https://<Host header>`, or it is refused 403 `origin`. That covers
  every POST and the WS upgrade. `SameSite=Strict` already withholds the
  cookie cross-site; the Origin check is the second lock, and it also
  covers DNS rebinding, since a rebinding page's origin is its own host
  name and the cookie is scoped to the IP or `.local` name anyway.
  Bearer requests are not Origin-checked, because native clients send
  no Origin.
- A 401 on a cookie request answers `Set-Cookie: …; Max-Age=0` too, so a
  revoked browser drops its dead token. That needs no logout endpoint.
- The spec's "never in a URL" rule stands. The rule "sent only as a
  bearer header" becomes "a bearer header, or for a browser an
  HttpOnly cookie".

### D3. One QR: a URL. Put the code in the fragment, not the query

The issue's URL puts the code in the query. **The fragment is better**:
`https://<addr>:<port>/#code=…&key=…&id=…`. A query string is sent on
the first request, so the code reaches whoever answers at that address
before any key check. That is exactly what §5 forbids ("the key is
verified before the code is sent"). A fragment never leaves the browser.
Even an impostor's genuine-looking page only gets it through its own
script, which is the page-impostor case D1 already concedes.
`history.replaceState` still clears it from the address bar and history
before anything else runs.

Native text path:
- `PairingInvite.parse` accepts either the JSON (unchanged) or this URL.
  For the URL, `addresses = [host:port]`, `code`, `key` → `spki_sha256`
  and `id` → `server_id`. It also reads a query form, to be lenient.
- The headset's paste, `SWITCHBOARD_INVITE` and
  `switchboard-remote pair <text>` then all take the URL for free.
- Keep "Copy invite" copying the URL. The JSON is needed only for old
  clients' paste, so drop it from the QR; nothing scans JSON (the
  headset has no camera, and the CLI takes text). The Swift fixture test
  at `FixturesTests.swift:157` gains a URL case.

Address:
- The QR uses `local_address()` today. A browser keys both the
  certificate exception and the cookie on the host name, so a DHCP move
  of the IP loses both and means pairing again.
- Prefer the Mac's mDNS name (`<LocalHostName>.local`) when it resolves.
  iOS resolves `.local` in Safari, and mdns-sd already publishes
  `default_server_name().local.` (`src/adapters/bonjour.rs:41`), though
  that is `hostname -s` and may not equal the system LocalHostName. Fall
  back to the IP.
- The spike settles which. If the IP is kept, the dialog says "if the
  Mac's address changes, pair the browser again".

### D4. Static paths and headers

Routes come before the `/api/v1/` strip in `serve_remote` (3023), `GET`
only (`HEAD` optional):

| Path | Content-Type |
|---|---|
| `/` | `text/html; charset=utf-8` (`web/index.html`) |
| `/app.js` | `text/javascript; charset=utf-8` |
| `/app.css` | `text/css; charset=utf-8` |
| `/favicon.ico` | `204 No Content` (browsers ask for it unasked; serving nothing avoids an image and a 404 log line) |

Anything else outside `/api/v1/` stays 404.

Headers on every static reply:
- `Content-Security-Policy: default-src 'none'; script-src 'self';
  style-src 'self'; connect-src 'self'; img-src 'self'; base-uri 'none';
  form-action 'none'; frame-ancestors 'none'`. Add an explicit
  `wss://<Host>` to `connect-src` if the spike finds `'self'` does not
  match WebSocket on Safari. No inline script or style; the HTML only
  links the two files.
- `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`,
  `X-Frame-Options: DENY`.
- `Cache-Control: no-store`. The assets are a few tens of KB, and Chrome
  does not cache under a certificate error anyway. no-store is simpler
  than versioned asset URLs and guarantees no cross-version mix.
  `/api/v1/*` replies also get `no-store`, since they carry transcripts.

Pool and rate:
- A browser opens up to 6 parallel connections per host. The
  unauthenticated pool is 4 and drops the rest at accept, so a cold page
  load can lose a file.
- Keep the page to three requests plus favicon, and load `app.js` with
  `defer` so the HTML, CSS and JS ride few connections.
- Count a static request carrying a valid cookie as `authenticated: true`,
  which moves its connection to the paired pool.
- Unpaired static requests count against the `/info` window (60 per
  minute): `Limits::limited` maps the static paths to `info`. A page load
  costs about 4 of those 60. The `/pair` window is untouched.
- The static constants are answered on the frame like everything else,
  in microseconds, so pool moves and auth stay in one place.

What the page shows with neither a fragment nor a cookie:
- "Not paired. On the Mac: settings menu → Remote devices → Allow
  pairing, then scan the code."
- Optionally, the typed-code path the headset has: show `/info`'s key in
  the dialog's short form for comparison by eye, plus a code field. It is
  cheap and helps when the camera won't open the URL.
- With a cookie and no fragment: rehydrate (§1).

### D5. Where the page lives and how it is tested

- `web/index.html`, `web/app.js` and `web/app.css` at the repo root,
  pulled in by `include_str!("../web/…")` in a small `src/adapters/web.rs`,
  or as constants beside `serve_remote`. That is a new README Layout line.
- Plain ES modules, no framework, no build.
- `tests/remote.rs`:
  - (a) Every `src=`/`href=` in the embedded `index.html` and every
    `fetch('/…')`/`new WebSocket` path literal in `app.js` is in the
    served set or under `/api/v1/`. Parse the constants; do not grep files
    at runtime.
  - (b) Each static path answers 200 or 204 with the Content-Type, CSP,
    `no-store` and `nosniff` named above, unpaired, on a LAN-style bind.
    The `http` helper (245) has to return the headers, since today it
    keeps only `Content-Length`.
  - (c) The browser pairing end to end with the host's own client:
    `/info` → `/pair {cookie:true}` → `Set-Cookie` present, `token`
    absent → `GET /api/v1/sessions…` with `Cookie:` and a matching
    `Origin` → 200. A missing or foreign `Origin` → 403. The WS upgrade
    with cookie and Origin → 101, and without Origin → 403. Revoke → 401
    with the clearing `Set-Cookie`.
  - (d) Two instances: distinct cookie names.
  - (e) Unpaired, the old allowlist test still refuses every
    `/api/v1/*` read.
- `tests/ui.rs:7454-7503`: "Copy invite" yields a URL with `#code=`.
- A unit test for the payload builder; move it out of `ui/remote.rs`
  into a pure function, since logic does not belong under `ui/`.
- Swift: `PairingInvite.parse` URL cases.
- No view changes, so the fixture set grows only if `/api/v1/device` is
  added. In that case add its fixture to `write_fixtures`.
- JS has no runner in this repo. Keep `app.js` logic thin and test the
  contract from the Rust side. The acceptance run (scan, accept, see,
  reply, revoke) is a manual live check, recorded in the spike README.

### D6. The device record

A browser is an ordinary `PairedDevice`:
- **Name**: the page builds it from `navigator.userAgent`
  ("Safari on iPhone", "Chrome on Android", "Safari on Mac") and posts it
  as `device_name`. The host does no user-agent parsing; it already caps
  the name at 64 chars (`core/remote.rs:167`).
- **Scope**: chosen in the pairing window as today.
- Listed and revocable in the dialog unchanged.

The page has to learn its scope on every load, and the scope can change
on the host. Native clients keep the pair reply's scope and learn a
lowering from a 403 `scope`. A reloaded page holds no pair reply. Hence
one small additive read: `GET /api/v1/device` (read) → `{device_id, name,
scope}`. That is the one new read the page cannot do without; it gets a
§4.3 row and an `info` capability, `device`. Re-read it on each
`hello`, and on any 403 `scope`.

Revoked versus identity changed: both answer 401 "the token is not
known". The page tells them apart with what it stored: the same key at
`/info` means "This device was revoked on the host. Pair again from the
Mac", and a different key means "This host's identity changed. Pair
again". No new error code is needed.

### D7. The card excerpt is not on the wire

The issue asks the list to show "the one-line excerpt the card shows".
`SessionView` has no such field (see §2), and remote-client.md §4.3's
claim that `last_message` is session metadata does not match the code.
The candidates:
- **(a) First cut, recommended.** The list line is `waiting_reason`
  when the session waits, else the `card` word. The session view shows
  the last response or screen.
- **(b) Fetch `last-response` per Claude session in the list.** Each one
  enters the 128-key watch set and costs a content-worker parse per
  session. Too much for a phone list.
- **(c) An additive masked `excerpt` on `SessionView`.** This is content
  inside metadata: it would move the fingerprint whenever the transcript
  or captions move, and it needs redaction in the diff path. It needs a
  separate design.

Recommend (a), and correct the §4.3 sentence. Raise this with the owner,
since it narrows a stated requirement.

## 4. Spec and doc amendments the plan carries

- **remote-client.md**:
  - §4.1: the static set and its pool and rate rule.
  - §4.3: the static rows, `GET /device`, the `/pair {cookie}` variant,
    and the Origin rule; fix the excerpt sentence.
  - §5: browser trust by click-through (what it buys and does not), the
    cookie as the browser's token carrier, the fragment URL as the QR,
    and "before pairing: `/info`, `/pair`, and the page".
  - §11: the new tests.
  - §13: the table rows for Authentication and Client storage.
- **design.md**: a new status entry under "Remote client (2026-10-12)",
  and the trust-boundary paragraph (the token now also lives in a
  browser cookie).
- **README**: the Layout lines for `web/*` and any new adapter file.
- **CHANGELOG**: the Unreleased line.
- **client/README.md**: the invite is a URL.

## 5. Risks and open points

- WSS and cookies under a certificate exception on iOS Safari are
  unverified. That is the spike's first question, and the feature
  depends on it. If WSS fails there, the fallback is polling
  `GET /waiting` and the open session on a timer, which the spec already
  tolerates (a 30 s refetch).
- Exception lifetime: the owner may have to click through again after
  Chrome forgets the decision. The cookie survives, so this is not a
  re-pair.
- An IP-based origin is lost on a DHCP move, which drops both the
  exception and the cookie. `.local` avoids that if it works on the
  devices.
- The query-versus-fragment change is a deviation from the issue's
  literal URL. It is argued in D3 and should be confirmed with the
  owner.
- `local_address()` is called from `ui/remote.rs` at draw time, through
  the adapter. Moving the URL building into a pure function, with the
  address passed in, also makes it testable.
- Keep `refuse` and every static route from logging the query or the
  `Cookie` header.
- Hard rules hold:
  - no keystroke injection, because the reply goes through
    `session.send`'s existing path;
  - secrets are masked before every cut, because the page reads the same
    endpoints;
  - agents are never resumed by the page, because the first cut has no
    resume button;
  - no client names anywhere.
