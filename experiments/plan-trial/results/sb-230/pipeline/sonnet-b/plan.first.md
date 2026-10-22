# Plan: a web control surface on the remote port (issue #230)

Line references are to the worktree at `7fe8949` (branch
`dispatch/230-a-web-control-surface-the-remote-port-se`, clean).

## Where this departs from the issue

- **"The key in the URL lets a genuine page refuse to send the code to a
  host whose `/info` key differs."** The code does not bear out the
  strength of that. `/info` returns `spki_sha256` as a JSON string
  (`remote_info`, `src/app.rs:3568`) over the same TLS connection the
  page was loaded on, and a page script cannot see the connection's
  actual public key. A host that terminates TLS can serve a page and an
  `/info` that agree with each other. The comparison catches a stale or
  mistyped QR that points at a different Switchboard, and nothing else.
  The plan keeps the comparison, says what it covers in the spec and in
  the dialog, and makes the owner's click on the certificate warning,
  checked against the key the Mac shows, the trust step (decision 1).
- **"The one-line excerpt the card shows."** The excerpt is not on the
  wire. It is built in the UI from the parsed conversation
  (`agent_excerpt`, `src/ui/cards.rs:558`, called at `:178`) or from a
  pane caption. `SessionView` (`control/src/lib.rs:646` and the fields in
  `client/Fixtures/session.json`) carries `card`, the state label
  (`"starting"`, `"at a prompt (…)"`, `"exited (0)"`), plus
  `waiting_reason` and `notes`. Putting an excerpt in a listing means a
  transcript parse per session, which §4.1 keeps off the frame. The
  first cut shows the `card` label, and `waiting_reason` for a waiting
  session. The last response arrives on opening the session. An excerpt
  is a later, separate read-model item.
- **"A device's scope on the page."** The scope is only in the `/pair`
  reply, and a dialog change takes effect on the next request. After a
  reload the page cannot know it, and it cannot read its cookie. The
  plan adds `GET /api/v1/me`. The issue lists no endpoint, but the
  device's scope, "revoked" and "still paired" all need one.
- **"Any path outside `/api/v1/` is refused."** True today
  (`strip_prefix("/api/v1/")`, `src/app.rs:3023`, and the 404 above it).
  The plan serves exactly three fixed paths outside it, before the
  frame is involved. It does not widen the prefix match.
- **"The `/pair` reply"** is `{token, scope, server_id, device_id}`
  (`src/app.rs:3656`). Decision 2 needs the token kept from script, so a
  browser asks for a cookie and its reply has no `token`. The native
  clients' request and reply are unchanged, so `pair.json` still decodes.

## The record

No change to any record. A browser is a `PairedDevice` (`src/ports/remote.rs:96`:
id, name, scope, `token_sha256`, created, `last_seen`) in `devices.json`,
saved through `RemoteFiles::save_devices` (`src/adapters/remote.rs:83`)
and found by `device_by_token` (`src/core/remote.rs:145`). Its name is
`device_name` from the page ("Safari on iPhone", from `navigator.userAgent`),
which `remote_pair` already takes (`src/app.rs:3650`) and defaults to
"Device". No `kind` field is added, so none of these versions moves:
`SCHEMA_VERSION` (16, `src/core/model.rs:12`), `VIEWS_SCHEMA_VERSION`
(7, `src/core/model.rs:612`) and `IDENTITY_SCHEMA_VERSION` (1,
`src/ports/remote.rs:17`). `devices.json` has no version of its own, only
the array. Should an implementer find a device kind necessary after all,
the bump is the value in `model.rs` or `ports/remote.rs` at that time plus
one, with a step in `adapters::store::migrate`. Another ticket may have
taken the next number first.

**Rehydration.** The host side is nothing new. The page keeps no state on
the host, `devices.json` survives a restart, and the same token hash is
accepted. `epoch` changes on restart and the page refetches (§4.4). The
device side is new, and has two parts:

1. The token, in an `HttpOnly; Secure; SameSite=Strict` cookie that
   script never reads (decision 2).
2. A non-secret `localStorage` entry `{server_id, spki_sha256, name}`
   written at pairing, in a `try`. Without it the page cannot tell
   "never paired" from "revoked" or "identity changed", because it
   cannot read the cookie. A reload does this:
   - read the entry;
   - `GET /info`, and compare `server_id` and `spki_sha256`: a mismatch
     shows "identity changed, pair again" and sends nothing;
   - `GET /me`: 401 with an entry shows "revoked" and clears the entry,
     401 without one shows the unpaired screen.

   With `localStorage` unavailable the page still works for the session
   and shows the unpaired screen after a reload that finds no cookie.

The origin is `https://<ip>:<port>`, so cookie and entry belong to that
address. A Mac whose address changes is a new origin to the browser and
needs pairing again. The spec says so as a known gap, because a phone has
no Bonjour to follow the host.

## The signals

None new. The page is a fourth speaker of the existing protocol (§4.4):
it opens `wss://<host>/api/v1/events`, waits for `hello`, snapshots
`projects`, `projects/{id}/sessions` and `waiting`, applies buffered
events by revision, and refetches on `changed {session | sessions |
waiting | projects}`. A session that starts waiting changes the `waiting`
collection, so it moves to the top without a reload. For the open session
it sends `watch {conversation | screen}` when the view opens and
`unwatch` on leaving, and refetches on `changed`. On `resync_required` or
a new `epoch` it reloads everything, and it reconnects with back-off.
Redaction (§4.3) is applied before every cut on the host and is unchanged.
The page renders every host-supplied string with `textContent`, never
`innerHTML`, so a transcript cannot inject markup even if the CSP were
relaxed. The spec states that rule, and a test enforces it.

## The actions and effects

No `AppAction`, no `Effect`, no core state: serving a page and
authenticating a cookie are the port's job (`src/adapters/remote.rs`,
`serve_remote`), as `/info` is. Pairing, revoking and scope changes
already go through `Paired`, `RevokeDevice` and `SetDeviceScope`.

**Decision 1: the certificate.** Click-through, not a downloadable
certificate. A downloaded certificate must be installed and then trusted
in full on the device (on iOS a profile, then Certificate Trust Settings).
That flow is heavy and easy to get wrong. It also trusts the certificate
for every name the device reaches, a wider grant than a per-address
exception. The click-through grants one browser an exception for one
address, which is all this surface needs. What it buys is encryption on
the wire and a browser that remembers the exception. What it does not buy
is any proof of whose certificate it is: on a hostile network an impostor
can present its own, and the owner's click is what lets it. So the step
the owner performs is a comparison. The pairing window already shows
`Key xxxx xxxx xxxx xxxx` (`short_hash`, `src/ui/remote.rs:303`). The page
shows the same sixteen digits in its header from `/info`, before and after
pairing. The window's text and the paired-devices dialog say it in one
sentence: "A browser trusts this Mac by your click on its certificate
warning, on a network you trust. Check the key on the page against this
one." The certificate stays as `make_identity` makes it
(`src/adapters/remote.rs:95`, no SubjectAltName). The spike records
whether Safari and Chrome offer "proceed" for it. If either does not, the
fix is an IP SAN added when the certificate is made at serve time. The
pin is the key, so reissuing the certificate invalidates no pairing.
`a_change_of_address_rebinds_the_port` (`tests/remote.rs:1494`) is where
that would be tested.

§5's rule is kept and amended: **before pairing, no bind serves anything
but `/info`, `/pair` and the page's own three files.** Every other path is
refused, loopback or not.

**Decision 2: the token reaches the socket as a cookie.**
`POST /api/v1/pair` takes an extra body field `"session": "cookie"`. The
reply then omits `token` and sets
`Set-Cookie: __Host-sb=<token>; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=34560000`
(400 days, the cap browsers apply; the token does not expire and
revocation is the host's). A script never holds the token, `fetch` and
`new WebSocket` both carry the cookie, and the token is in no URL and no
`Sec-WebSocket-Protocol`. `localStorage` is readable by any script on the
origin and a cookie is not, which settles where the token lives. Native
clients send no `session` field and are unchanged.

`authenticate` (`src/app.rs:3127`) takes the token from `Authorization:
Bearer` and, absent that, from the cookie. Cookies are not isolated by
port, so another service on the same IP shares the jar, and the upgrade
request carries the cookie: both are request-forgery surfaces. So a
cookie-authenticated request that is not a `GET`, and every WebSocket
upgrade, must carry `Origin: https://<Host header>`. Otherwise it is
refused 403 `origin` before any command is translated. A bearer request
is never subject to this. Revocation needs no new path. The record is
gone, the cookie's hash is unknown, the next request is 401
`unauthorized`, and the socket closes as it does today.

*If the spike shows a browser refuses to store the `Secure` cookie from a
click-through origin, stop and take that to the owner before building on
it.* The fallback is a `localStorage` token and a single-use ticket for
the socket. That changes the threat model (decision 2 above), so it is not
built speculatively.

**Port changes** (`src/adapters/remote.rs`, `src/app.rs`):

- `RemoteRequest` (`:360`) gains `cookie`, `origin`, `host`
  (`Option<String>`), filled in `read_request` (`:444`) beside `bearer`
  (`:487`). The cookie header is parsed for `__Host-sb` only and dropped
  otherwise; none of the three is logged. `RemoteReply` (`:389`) gains
  `headers: Vec<(String, String)>`, which `respond` (`:530`) writes after
  `Content-Type`. That means adding `headers: Vec::new()` to the 30
  literals (28 in `src/app.rs`, 2 in the adapter), a mechanical edit.
- **Static paths are answered on the connection thread**, in `connection`
  (`:770`) before the request is handed to the frame, from a fixed `match`
  in a new `src/adapters/web.rs` (`pub fn asset(method, path) -> Asset`).
  The path is matched exactly and never joined to anything. They cost the
  frame nothing and do not use the 64-deep inbox. The set, `GET` only
  (anything else on these paths is 405 with `Allow: GET`; the query is
  ignored and never read):

  | Path | `Content-Type` |
  |---|---|
  | `/` | `text/html; charset=utf-8` |
  | `/app.css` | `text/css; charset=utf-8` |
  | `/app.js` | `text/javascript; charset=utf-8` |

  Anything else not under `/api/v1/` stays 404 `not_found` from
  `serve_remote`. Every static reply carries:
  - `Content-Security-Policy: default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'`.
    There is no `unsafe-inline` and no host. The HTML has no inline script
    or style, and carries `<link rel="icon" href="data:,">` so no favicon
    is requested. If the spike shows Safari does not match `'self'` to
    `wss:`, the explicit `wss://<Host>` source is added at serve time.
  - `X-Content-Type-Options: nosniff`.
  - `Referrer-Policy: no-referrer`. **This is load-bearing:** the page's
    own URL holds the code, and without it the browser sends that URL as
    `Referer` with the request for `/app.js`.
  - `Cache-Control: no-store`, so nothing is cached across versions and no
    version is put in a URL.
  - `Cross-Origin-Opener-Policy: same-origin`.
  - Never a `Set-Cookie`.
- **Pools and rate** (`:567-587`, `:631`). A static reply is
  `authenticated: false`, so a browser's connections sit in the pool of 4
  until its first authenticated call (the `events` upgrade or a read) moves
  them to the paired pool (`:835`). A browser may open up to six
  connections for a page. Past four, an accept is closed at once and the
  browser retries. The page is three files so keep-alive covers it with
  one or two. The pools are not changed: the spike counts the connections
  of a real page load, and only a measured failure changes the pool
  accounting. The only addition is `PAGE_PER_MINUTE = 60` (a new
  `RateWindow` in `Limits`, `:631`), shared by the three paths and
  answered 429 on the connection thread as `/info` is. A load is three
  requests, so twenty loads a minute.
- **`GET /api/v1/me`** (`serve_remote`, after `authenticate`): `{device_id,
  name, scope}`, any paired device. The cheapest "am I still paired" probe
  and the one the revoke test polls. It goes in the §4.3 table.
- **Unpaired with no code and no cookie**, the page shows the host name
  and key (`/info`), "Not paired", and "Open Remote devices on the Mac,
  allow pairing, and scan its QR code", with `pairing_open` from `/info`.
  With a code and pairing closed (403 `pairing_closed`) it says so. On
  `wrong_key` it says "this is not the host whose code was scanned". On
  `wrong_code` it says the code was wrong or used.

**Decision 3: one URL.** The pairing window's payload becomes
`https://<ip>:<port>/?code=<code>&key=<spki_sha256>&id=<server_id>` (the
address is IPv4, from `local_address`, `src/adapters/remote.rs:962`; the
alphabet needs no escaping). A pure
`core::remote::invite_url(address, server_id, spki, code)` builds it, so a
core test pins the shape. The QR, the text beside it and "Copy invite"
carry the URL. The JSON is dropped from the host. Nothing needs it once
the native text paths accept the URL, so there is one invite, not two. The
Swift side keeps decoding the JSON, for old pasted invites and the
fixture tests. `PairingInvite.parse` (`client/Sources/SwitchboardProtocol/Info.swift:114`)
reads JSON when the text starts with `{`, else a URL, taking `addresses`
from its host and port, `key` as `spkiSha256`, `id` as `serverID`. It is
the single entry for the CLI (`main.swift:107`) and for the headset's paste
(`BrowserView.swift:106`, `:883`), so neither changes beyond `main.swift`'s
usage text (`:10`). `switchboard-remote pair` keeps both forms: one
argument (URL or JSON) or three.

**The code in a GET.** The browser requests `/?code=…`. `read_request`
already parses a query (`:497`), the static arm never reads it, and
`refuse` logs method and path only. `app.js` begins with
`history.replaceState(null, "", "/")` after copying the three values into
locals, before any request. The code is single-use and lives five minutes,
which bounds what a camera app or browser history kept of the URL.

## The UI

**The host** (`src/ui/remote.rs`): `pairing` (`:141`) builds the payload
with `invite_url` instead of the JSON (`:160-166`). The text beside the
QR changes from "Scan, or type the key and the code" to "Scan with a
phone's camera, or type the key and the code in the headset", followed by
the warning sentence from decision 1. The paired-devices dialog (`:230`)
gains one line under its heading with that sentence. A browser appears in
the list by its name, with scope combo and Revoke as every device does.

**The page** (`web/index.html`, `web/app.css`, `web/app.js`; no framework,
no build step, no dependency; light and dark by `prefers-color-scheme`;
one column at phone width):

- *Header*: the host name, its short key, the device's scope
  (`read`/`control`) from `/me`, and a connection dot.
- *List*: sessions grouped by project, the waiting ones first and marked,
  each with its name, `kind`, `liveness` and `card` label, and for a
  waiting session `waiting_reason`.
- *Session view*: the last response (`last-response`, `text`, with
  `truncated` said) for a Claude Code session, or the screen
  (`screen?lines=200`, monospace) for anything else, refreshed on
  `changed`. If `trust_question` is true, a "Folder trust question" line
  and a Trust button for a `control` device.
- *Reply box*: shown for a `control` device only. It posts
  `sessions/{id}/send` with `{op, text}`, with `op` a UUID per click
  (`crypto.randomUUID`, falling back to `getRandomValues`, since a
  click-through origin may not be a secure context). A timeout is followed
  by `GET ops/{op}`, never a blind resend (§4.5).
- *States said plainly*: unpaired, pairing closed, wrong key, "identity
  changed, pair again", "revoked", host unreachable (retrying), and the
  429 and 503 refusals.

## The tests

The tests are named in the order of the pieces above. The existing
`the_allowlist_refuses_everything_else_with_no_side_effect`
(`tests/remote.rs:682`) must keep passing.

`tests/remote.rs`: `http()` (`:245`) drops response headers, so a second
helper returns `(status, headers, body)` and `http` wraps it. Then:

- **Static paths.** Each of the three answers 200 with its `Content-Type`
  and the full header set, with no `Set-Cookie`. The CSP contains no
  `unsafe-inline`, no `http:` or `https:` source and no `*`. A
  `POST` to `/` is 405.
- **Not in the set.** `/app.js.map`, `/web/`, `/index.html`, `//`,
  `/../Cargo.toml` and `/app.js/` are 404, and each is refused to an
  unpaired caller (no file read).
- **The page is self-consistent.** Every `src` and `href` in the embedded
  HTML is a path the host serves. Every `"/api/v1/…"` literal in `app.js`
  matches a row of the §4.3 allowlist (the test keeps a table), so a typo
  is caught. `app.js` contains no `innerHTML`, `outerHTML`,
  `document.write`, `eval` or `new Function`.
- **Rate.** The 61st page request in a minute is 429, in the shape of
  `pair_is_rate_limited_per_listener` (`:1406`).
- **Pairing end to end, as the CLI does it.** Build the URL with
  `invite_url`, parse it back, compare `key` with `/info`, `POST /pair`
  with `"session":"cookie"`. Assert the `Set-Cookie` attributes
  (`__Host-`, `Secure`, `HttpOnly`, `SameSite=Strict`, `Path=/`, no
  `Domain`) and no `token` in the body. Then `GET /me` and a read with the
  cookie and no bearer succeed. A bearer-only pairing and read behave as
  before.
- **Origin.** A cookie `POST …/send` with a foreign or missing `Origin` is
  403 `origin` and sends nothing to the pane; with the right one it is
  typed once per op. The upgrade with a foreign `Origin` is refused; with
  the right one it gets `hello`. A bearer request with no `Origin` is
  unaffected.
- **Revoke** (extend `:658`): after `RevokeDevice`, the cookie's `GET /me`
  is 401 `unauthorized`, and a subscribed cookie device's socket closes.
  A scope change shows in `/me` on the next call.
- **Fixtures** (`write_fixtures`, `:1587`): none added. `/me` is read only
  by the page, and no Swift type decodes it.

`tests/ui.rs`: extend `remote_devices_dialog_opens_pairing_and_revokes_a_device`
(`:7424`). Its clipboard assertions (`:7484-7492`) parse the invite as JSON
today. They become: the text starts with `https://`, and its `code`, `key`
and `id` query values are the window's code, the identity's `spki_sha256`
and its `server_id`. The dialog text shows the certificate sentence.

`src/core/tests.rs`: `invite_url` is exact for a known input.

Swift (`client/Tests/SwitchboardClientCoreTests/PairingTests.swift`):
`parse` accepts the URL form and the old JSON, and refuses a URL with no
`code`. Run with `swift test` in `client/`, which CI does not.

No JS test runner exists and the issue forbids a build step, so the page's
behavior is covered by the acceptance below, done by hand on a phone.

**Spike first** (CLAUDE.md): `spikes/23-web-surface/README.md` with
measured results, written before the page is relied on. It answers on
Safari on an iPhone and Chrome on a Mac:
- does a click-through certificate (as `make_identity` makes it) get a
  "proceed" option, and is a `__Host-` `Secure; HttpOnly; SameSite=Strict`
  cookie stored from it, sent on `fetch` and on `wss://`;
- does `connect-src 'self'` allow the same-origin `wss://`;
- how many connections a page load opens against the pool of 4;
- does the camera open the `https://ip:port/?…` URL, and does `replaceState`
  keep the code out of the history.

The spike needs a phone; an implementer without one records which rows
are unmeasured, and the owner's acceptance run closes them.

**Acceptance** (the issue's, by hand): open the pairing window, scan with a
phone, accept the warning, see the sessions; start a session waiting and
watch it move to the top; send a reply and see it arrive; revoke the phone
in the dialog and the page says "revoked" on its next request.

## Docs, in the same commit

- `docs/remote-client.md`:
  - §4.1: the page rate and the pool note;
  - §4.3: rows for the three static paths and `/me`;
  - §5: the pairing QR is a URL, the browser trust step and what it does
    not prove, the cookie (the "sent only as a bearer header" bullet gains
    "or the browser's cookie"), the Origin rule, the amended "nothing but
    `/info`, `/pair` and the page" rule, and the per-address known gap;
  - §11: the tests above;
  - a §0 revision note.
- `docs/design.md`, "Remote client (2026-10-12)" (`:3107-3584`): a bullet
  for the web surface, and an entry in its known gaps (`:3573`): no
  excerpt in the list, no Dispatch tickets, conversation, drafts or runs,
  pairing per address.
- `README.md`: Layout rows for `web/`, `src/adapters/web.rs` and
  `spikes/23-web-surface`; the `adapters/remote.rs` row (`:383`) gains the
  cookie, the Origin rule and the static paths; the `ui/remote.rs` (`:423`)
  and `tests/remote.rs` (`:437`) rows; the dev-aid paragraph (`:267-279`)
  says "Copy invite" puts a URL on the clipboard and that
  `switchboard-remote pair "$(pbpaste)"` still pairs. No new environment
  variable and no new script line.
- `CHANGELOG.md`, under Unreleased: "A web page on the remote port: scan
  the pairing QR code with a phone to see the sessions, which are waiting,
  and send a reply."

## Order inside the one commit (the app runs after each)

1. The spike.
2. The port: request and reply fields, the static arm with a placeholder
   page, the rate window.
3. The cookie, the Origin rule and `/me`.
4. The page.
5. `invite_url`, the dialog text and the Swift parse.
6. The docs.

Before committing: `cargo fmt --all`, `cargo test --locked --workspace`,
`cargo clippy --locked --workspace --all-targets -- -D warnings`, and
`swift test` in `client/`. The `dispatch` crate is untouched, so its Linux
clippy is not needed. Nothing in the code, tests, fixtures, docs, commit
message or pull request names a client or customer, and none of them
carries a tool-attribution line.

## Commit message

`A web page on the remote port, paired from a URL in the QR code`

**Contract additions** (copy into the commit body):

- `RemoteRequest` gains `cookie`, `origin` and `host` (`Option<String>`);
  `RemoteReply` gains `headers: Vec<(String, String)>`.
- New module `adapters::web` (`asset`) and the `web/` files it embeds.
- Port paths: `GET /`, `GET /app.css` and `GET /app.js` outside `/api/v1/`,
  answered on the connection thread; `GET /api/v1/me` for any paired
  device; `POST /api/v1/pair` accepts `"session": "cookie"` and then sets
  the `__Host-sb` cookie and omits `token`; cookie-authenticated unsafe
  requests and upgrades need a matching `Origin`, else 403 `origin`.
- `PAGE_PER_MINUTE` (60) in the port's limits.
- `core::remote::invite_url`. The pairing QR and "Copy invite" carry a URL
  (`?code=&key=&id=`), and `PairingInvite.parse` accepts the URL or the
  JSON.
- No `AppAction`, no `Effect`, no model change, and no schema bump:
  `SCHEMA_VERSION`, `VIEWS_SCHEMA_VERSION` and `IDENTITY_SCHEMA_VERSION`
  stay as they are.
