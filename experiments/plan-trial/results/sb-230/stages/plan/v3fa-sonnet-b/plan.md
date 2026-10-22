# Plan: a browser page on the remote port (#230)

One commit, at 7fe8949. The app runs at every step: the page, the
certificate reissue and the URL invite land together, and a host with
the old `identity.json` still listens and still pairs native clients.

"Facts" below is the facts file for this issue. A decision that rests on
a line under Settled cites it. One that rests on a line under Unresolved
or For the owner says so where it is made, and the plan's default is the
design that works whichever way that answer goes. The paths that need a
favourable answer are listed last, under "Built behind the default", and
nothing in the page's basic behaviour or the acceptance steps waits on
them.

## Where this departs from the issue

- **"The one-line excerpt the card shows" is not on the wire.**
  `SessionView` (`control/src/lib.rs:636-688`) has the card word,
  `waiting_reason` and the times, and no excerpt; facts (Settled, tree)
  find no `last_message` anywhere. The page shows the card word and the
  waiting reason on every row, and the last response's first line only
  for the waiting sessions, read with `last-response` and capped at 16
  reads per list load. A read enters the watch set for a minute
  (§4.4, "The watch poll"), so a page that read every session could push
  the headset's watches out of the 128-key set
  (`src/app.rs:1789-1795`); the cap keeps the page's share small whatever
  the unmeasured eviction cost turns out to be (facts, For the owner 9).
  No `SessionView` change, no fixture change.
- **"The device's scope in the header" has no source after pairing.** No
  endpoint returns it, so a scope changed in the dialog is invisible
  until a command is refused. The plan adds `GET /api/v1/device`
  (authenticated, `{device_id, name, scope}`) and the capability word
  `device`. The page cannot do without it, which is the issue's own test
  for a new read.
- **"Revoked" needs no new refusal code.** `authenticate`
  (`src/app.rs:3127`) answers `unauthorized` both for no token and for an
  unknown one. With the token kept by the page (decision 2) the page
  knows which it is: a 401 on a request that carried a token is revoked
  or rotated; no token is never paired. A cookie would have hidden that
  from the page.
- **The `key` check is weaker than the native pin.** A page cannot read
  the certificate it is talking over, so comparing the URL's key with
  `/info`'s proves only what the host claims. It catches a genuine page
  on the wrong genuine host (another installation at that address, a
  rotated key); it does not catch a relay, because the page that would
  post the host's key to `/pair` is the relay's. So
  `docs/remote-client.md` §5's sentence "The host refuses a pairing whose
  `spki_sha256` is not its own, so a relayed request through a
  substituted certificate fails" does not hold for a browser, and the
  plan rewrites it to say so. The certificate the browser trusts is the
  trust step (decision 1).
- **The certificate is not browser-ready.** `make_identity`
  (`src/adapters/remote.rs:95`) builds `CertificateParams::new` with no
  names, so no subjectAltName, a year-4096 `not_after`, and a new key
  each call. The issue treats the warning as a click-through. Whether
  Safari or Chrome offer one for such a certificate is Unresolved
  (facts); Apple's rules (SAN, ≤825 days, `serverAuth`) are Settled for
  certificates they trust. The plan reissues the certificate over the
  stored key (decision 1).
- **Decision 2 does not take the cookie.** The issue's first candidate
  rests on `SameSite=Strict` cookies reaching a same-origin WebSocket
  handshake, which is Unresolved (facts), and on `HttpOnly` hiding the
  cookie from script, which the facts file did not fetch. The subprotocol
  carrier is Settled (facts: WHATWG, RFC 6455 §4.2.2, §4.1).
- **"The spec forbids the token in a URL"** is `docs/remote-client.md:312`
  ("sent only as a bearer header over the pinned connection, never in a
  URL"; facts, Settled, tree), not a standard. A subprotocol value is a
  header, not a URL, but the line also says "pinned connection", which a
  browser's connection is not. The plan amends that line.
- **"Any path outside `/api/v1/` is refused"** is true
  (`src/app.rs:3023`) but on the frame, after the request took an inbox
  slot. The static paths are answered on the connection thread, before
  the inbox, so a page load costs the frame nothing.

## The record

No new record kind, no `model.rs` change (`SCHEMA_VERSION` stays 16,
`VIEWS_SCHEMA_VERSION` stays 7). A browser is one more `PairedDevice` in
`devices.json` (`src/ports/remote.rs:96-107`), named by the page itself
in `/pair`'s `device_name` ("Safari on iPhone", built from
`navigator.userAgent`; the host already takes the name and cuts it at 64,
`src/core/remote.rs:166-175`). Scope is chosen in the pairing window as
today. The device record gets no `kind`: the name is enough (decision 6),
and `devices.json` has no version to carry one.

`identity.json` changes, additively. `Identity` (`src/ports/remote.rs:26-42`)
gains `sans: Vec<String>` (the names the certificate was issued for) and
`issued: Option<SystemTime>`, both `#[serde(default)]`, so an older file
reads with `sans` empty and `issued` `None`. `IDENTITY_SCHEMA_VERSION`
(1 at `src/ports/remote.rs:17` today) is bumped to its value at
implementation time plus one; nothing reads the field to refuse a file
today, and readers accept older ones.

Rehydration: at launch `load_identity` (`src/app.rs:2782`), after the key
check passes and the instance is not read-only, asks
`remote::reissue_if_stale(identity, key_pem, wanted_sans, now)`. It
reissues when `wanted_sans` (the outward-route address from
`local_address()`, `127.0.0.1`, `::1`, `localhost`) is not within
`identity.sans`, or `issued` is `None` or older than 700 days. The new
certificate is `self_signed` over the key loaded with `KeyPair::from_pem`
(the same call as `make_identity`'s, with a loaded key: facts, Settled,
tree), with the names as SANs, EKU `serverAuth`, `not_before` a day back
and `not_after` 800 days out, inside Apple's 825-day limit (facts,
Settled; the "Does the limit bind a user-trusted certificate" question
is Unresolved, so the plan stays inside it and the answer does not
matter). The key, and so `spki_sha256`, is unchanged, so every native pin
holds (`docs/remote-client.md` §5, "What the client pins"). It is saved
through the existing atomic `save_identity`. A failed reissue logs, keeps
the old certificate, and the port listens as before. Cost the owner
should know: a certificate the owner installed on a device is that
certificate, so it stops being trusted when it is reissued (address
change, or about every two years); the dialog says so (UI).

The page keeps in `localStorage` (per origin, readable only by the
page's own script, which is the only script the CSP allows; facts,
Settled) one JSON value: the token, `server_id`, `spki_sha256`,
`device_id`, `name`, `scope`. Nothing else is stored on the device, and
the code is never stored.

## The signals

None new. The page speaks §4.4 as the headset does: waits for `hello`,
snapshots, reconciles events by `revision`, resends `watch` after a
reconnect, and handles `resync_required` and a changed `epoch` by
refetching everything. It watches one key at a time, the open session's
`conversation` (Claude Code; the last response is stamped at the
conversation's revision) or `screen` (anything else).

If the socket cannot open or keeps dropping (whether a click-through
covers `wss://` and `fetch` is Unresolved, facts), the page falls back to
fetching `waiting` and the visible sessions every 5 s and the open view
every 30 s, and retries the socket with backoff. Live updates are the
default path, not a precondition for the page working.

## The actions and effects

**Static paths, answered on the connection thread** in `connection`
(`src/adapters/remote.rs:770`), after the rate check at `:788` and before
the request is queued. New `src/adapters/web.rs` holds
`asset(path) -> Option<Asset { content_type, body }>` over
`include_str!` of `web/index.html`, `web/app.js`, `web/app.css`. Exactly
these, `GET` only; anything else, `HEAD` included, falls through to the
frame's 404 as today.

| Path | Content-Type |
|---|---|
| `/` (any query) | `text/html; charset=utf-8` |
| `/app.js` | `text/javascript; charset=utf-8` |
| `/app.css` | `text/css; charset=utf-8` |

Every static reply carries `Content-Security-Policy: default-src 'none';
script-src 'self'; style-src 'self'; connect-src 'self'; img-src data:;
base-uri 'none'; form-action 'none'; frame-ancestors 'none'`,
`Cache-Control: no-store` (the page is small; no ETag),
`Referrer-Policy: no-referrer`, `X-Content-Type-Options: nosniff`, and
`Connection: close`. The index links `<link rel="icon" href="data:,">`, so
there is no favicon request. `Connection: close` frees the static
request's unauthenticated slot at once, which is how the page load
competes with the pool of 4 (`MAX_UNAUTHENTICATED`, `:567`) without
knowing how many connections Safari opens (Unresolved, facts). The
page's later fetches carry the token, and a connection moves to the
paired pool on its first authenticated reply as today (`:834-849`); the
page keeps at most two requests in flight so it does not fill the 12.
Static paths get a rate window of their own, `STATIC_PER_MINUTE = 60`
beside `PAIR_PER_MINUTE` and `INFO_PER_MINUTE` (`:583-587`, extending
`Limits::limited` at `:629`), answered 429 from the connection thread.
Neither the connection thread nor the router logs a query, so the code in
`/?code=…` reaches no log (`refuse`, `src/app.rs`, logs method and path
only; the static arm logs nothing). `respond` gains a sibling,
`respond_static`, so `RemoteReply` and its ~20 literals are untouched.

**Token on the socket.** `read_request` (`:444`) also keeps `host`,
`origin` and, on an upgrade only, `Sec-WebSocket-Protocol`. When the
request upgrades and carries no `Authorization`, a protocol value of the
form `bearer.<token>` fills `bearer` (base64url is within the token
characters, facts, Settled); the token is taken out of the header list so
it is never in `Debug` output. If the client offered `switchboard.v1`,
`respond_upgrade` (`:519`) echoes exactly that value, because a browser
fails the handshake if protocols were offered and none came back (facts,
Settled). Native clients offer none and get none. `fetch` sets
`Authorization: Bearer` itself.

**Origin.** On an upgrade and on every `POST`, a request that carries
`Origin` must have `Origin == "https://" + Host`, else 403 `bad_origin`;
no `Origin` (the native clients) passes. This is RFC 6455 §10.2's
"SHOULD" (facts, Settled); the allowlist is the owner's (For the owner 4),
and the default is the rule that needs no list and holds whether the
device used the IP, a `.local` name or loopback. The token is not ambient
(no cookie), so a cross-site page has nothing to hijack; the check is
defence in depth.

**Device read.** `serve_remote` (`src/app.rs:3017`) answers
`GET device` after `authenticate`, before the body match; `remote_info`
(`:3568`) adds `device` to `capabilities`.

**Certificate export.** `AppAction::ExportCertificate` →
`Effect::ExportCertificate(Identity)` → a new `RemoteStore` method
`export_certificate(&Identity) -> Result<PathBuf, String>` writing the
DER to `<data dir>/switchboard-<short server id>.cer` (atomic, public
data), then the opener's `reveal` (`src/ports/opener.rs`) so the owner
can AirDrop it. The fake records the call. Failure is `AppAction::Failed`.

**Core.** `src/core/remote.rs` gains two pure functions:
`pairing_url(identity, window, address: Option<SocketAddr>) -> Option<String>`
(`https://<addr>/?code=<code>&key=<spki_sha256>&id=<server_id>`;
`SocketAddr` brackets an IPv6 address) and
`certificate_covers(identity, address) -> bool`. The QR payload moves out
of `ui/` into the first (`src/ui/remote.rs:160-166`).

**The page** (`web/`, no build step, no dependency):

1. On load, read `location.search`, then `history.replaceState(null, "",
   location.pathname)` before any other work (settled: it changes the
   current entry without a load; whether the code is in history before
   this runs is Unresolved, and the code is single-use, `src/core/remote.rs:65-66`,
   so what leaks is a spent code).
2. With a code: `GET /api/v1/info`; if `spki_sha256` differs from `key`
   or `server_id` from `id`, show "this is not the host that made the
   code" and send nothing. Otherwise `POST /api/v1/pair
   {code, device_name, spki_sha256}`, store the record, show the sessions.
3. With neither code nor token: show "Open Remote devices on the Mac,
   choose Allow pairing, and scan the code", and a form with the host's
   key read from `/info` in the dialog's short form and a code field. The
   owner compares the key by eye, as the headset does (§5, pairing by
   the typed code). The `/pair` body carries that key.
4. With a token: `GET /api/v1/info` first; a key or `server_id` different
   from the stored one shows "identity changed, pair again" and sends the
   token nowhere. Then `GET device`: 401 shows "this device was revoked
   on the host", a scope fills the header, and the reply box is shown
   only for `control` and hidden otherwise.
5. List: `projects`, then `projects/{id}/sessions` for each, grouped by
   project, waiting first (the host's rank: `CardState::rank`), each row
   with kind, card word, waiting reason and the capped excerpt.
6. Session view: `last-response` for a Claude Code session, `screen`
   otherwise, refetched on `changed` of the watched key and every 30 s.
7. Reply: `POST sessions/{id}/send {op, text}`. The op is 128 random bits
   from `crypto.getRandomValues`, hex, not `randomUUID`, which needs a
   secure context and whether a bypassed page is one is Unresolved
   (facts); uniqueness is all §4.5 asks. On a timeout the page queries
   `/ops/{op}` and never resends blind. A session at the folder-trust
   question, a `prompt_refusal`, and `resume` are shown as text with a
   line "answer this on the Mac" and no control; owner call (notes), cut
   from this commit.

The page's logic is small named functions in one file with no framework.

## The UI

Only `src/ui/remote.rs`.

- The QR encodes `pairing_url`; the text beside it is unchanged (key,
  code, address). "Copy invite" copies the URL.
- A "Save certificate" button next to it dispatches `ExportCertificate`.
- Trust text under the QR, plainly (decision 1, the owner's call: the
  default is the one that works whichever way a browser answers). "The
  page opens over a certificate only this Mac vouches for. Save the
  certificate and install it on the device (AirDrop it, then trust it in
  Settings), or accept the browser's warning if it offers one. The key
  in the link stops a genuine page from pairing with the wrong host; it
  cannot tell you the page itself is genuine, so check the Key line
  against the page. Reissuing the certificate, which happens when this
  Mac's address changes, makes an installed copy stale." Click-through
  is therefore not the only trust step and is not required by anything.
- A line when `certificate_covers` is false for the shown address:
  "This certificate does not name <address>; relaunch Switchboard to
  reissue it."
- The paired-devices list is unchanged; a browser is listed and revoked
  like any device. The redaction sentence (`src/ui/remote.rs:72`) gains
  "A page in a browser keeps its token in that browser's storage."

## The tests

- `src/core/tests.rs`: `pairing_url` (IPv4, IPv6 bracketed, no address →
  `None`, code and key in the query); `certificate_covers`.
- `src/adapters/remote.rs` unit tests: `reissue_if_stale` keeps the SPKI,
  adds the IP and `localhost` SANs, reissues on a new address and on a
  700-day-old `issued`, does nothing otherwise, and leaves the old
  certificate on error; `read_request` takes the token out of
  `bearer.<token>` on an upgrade only, never off a plain GET, and echoes
  `switchboard.v1`; `Limits` throttles the static window.
- `tests/remote.rs`: extend `http` (`:245`) to return the headers.
  - Each of the three paths answers 200 with the table's Content-Type,
    the CSP, `no-store` and `Connection: close`; `/?code=x` answers
    the same; `/other`, `HEAD /` and `POST /` answer 404 as before; no log
    line holds the code.
  - The embedded page references only paths the host serves: every
    `src`/`href` in `web/index.html` is a table path or `data:`; every
    literal `/api/v1/…` in `web/app.js` matches an allowlisted route.
  - Pairing end to end with the host's own client as the CLI does: `/info`,
    `/pair` with the URL's key and code, then `GET device` with the token;
    a wrong key is `wrong_key`, a spent code `pairing_closed`.
  - The events socket with `Sec-WebSocket-Protocol: switchboard.v1,
    bearer.<token>` and no `Authorization`: 101 with
    `Sec-WebSocket-Protocol: switchboard.v1`, `hello`, then a `changed`
    when a session starts waiting. The same token as a subprotocol on a
    non-upgrade request is 401. A bad token as a subprotocol is 401.
  - `Origin` on an upgrade and a POST: matching passes, foreign is 403
    `bad_origin`, absent passes.
  - Revoking the device: the next `GET device` is 401 and the socket
    closes (extends `revoke_closes_the_devices_socket`).
  - Static requests do not hold a slot: with the pool of 4 full of
    stalled strangers (the model is `stalled_strangers_fill_their_own_pool…`,
    `:3226`) a static page load is dropped at accept like any
    stranger, and ten sequential page loads leave the pool empty.
  - `GET device` reads the scope after a scope change, with no re-pair.
- `tests/ui.rs`, extending `remote_devices_dialog_opens_pairing_and_revokes_a_device`
  (`:7424`): the pairing window's Copy invite dispatches nothing and puts
  an `https://…/?code=…&key=…&id=…` text on the clipboard; "Save
  certificate" dispatches `ExportCertificate`; the stale-certificate line
  shows only when the address is uncovered.
- Swift (`client/Tests/SwitchboardProtocolTests/FixturesTests.swift:157`
  is the model): `PairingInvite.parse` of the URL gives the key, code, id
  and `addresses == ["192.168.1.9:43187"]`, and of the JSON as before. It
  tries a URL first (`URLComponents`, `code`, `key`, `id`) and JSON
  second, so the headset's paste (`BrowserView.swift:106, 883`) and
  `switchboard-remote pair '<text>'` take either; its usage text in
  `client/Sources/switchboard-remote/main.swift` says "invite URL or
  JSON". Runs with `swift test` in `client/` on a Mac; CI does not.
- Not testable here: the page's own logic (query removal, key compare,
  reconcile) has no runner in the tree and the issue rules out a
  dependency. It is checked by hand, and the design.md status says so.
  The device questions in the facts file (click-through, `wss://`, pool,
  history) need a phone and are recorded there as not run until one is.

## Docs

- `docs/remote-client.md`: §4.1 (static paths answered on the connection
  thread, the static rate window), §4.3 (`device`, the page), §5 (the
  pre-pairing rule at line 306 gains "and the page's three files"; the
  relayed-pairing sentence and line 312 amended as above; the URL invite
  replaces the JSON QR; the certificate's SANs and validity).
- `docs/design.md`: a dated entry at the end of the "Remote client"
  section (it ends at "Known gaps at the first merge", before "Versioning
  and updates"): what is built, the known gaps (page logic untested, no
  device run, excerpt only for waiting sessions, trust-question and
  resume not on the page).
- `README.md`: Layout rows for `web/` and `src/adapters/web.rs`, the
  `tests/remote.rs` row, and the dev-aid text near l.277 ("Copy invite"
  is a URL; `switchboard-remote pair "$(pbpaste)"` still works). No new
  environment variable; the script line `pairing <code>` opens a window to
  drive the page by hand against a test data dir.
- `CHANGELOG.md`, under Unreleased: "A browser can open the remote port's
  page, pair from the QR code's link, list sessions and send a reply; the
  host's certificate is reissued over the same key with the address in it."

## Built behind the default

Each needs an answer the facts file does not have. None is built here.

- A `HttpOnly; Secure; SameSite=Strict` cookie as the token carrier, once
  a device shows Safari sends it on a same-origin WebSocket handshake.
- A `/switchboard.cer` download path, so a device that can click through
  can fetch the certificate without AirDrop.
- A `SessionView` excerpt field (additive in `control/`, fixtures on both
  sides) in place of the page's capped `last-response` reads.
- A `.local` name in the URL and the SANs, once iOS Safari is known to
  resolve it.
- An owner-set Origin allowlist.

## Commit message

`Serve a browser page on the remote port, paired from a URL invite, with a certificate that names the host`

## Contract additions

- `ports::remote::Identity`: `sans: Vec<String>`, `issued: Option<SystemTime>` (`#[serde(default)]`); `IDENTITY_SCHEMA_VERSION` from the value in `src/ports/remote.rs` at implementation time, plus one.
- `ports::remote::RemoteStore::export_certificate(&Identity) -> Result<PathBuf, String>`, with its fake.
- `AppAction::ExportCertificate`; `Effect::ExportCertificate(Identity)`.
- `adapters::remote::RemoteRequest`: `host`, `origin`, `subprotocol` (adapter type, built only by `read_request`).
- Wire: `GET /api/v1/device` → `{device_id, name, scope}`; `device` in `info.capabilities`; error code `bad_origin` (403); unauthenticated static `GET` of `/`, `/app.js`, `/app.css`; `Sec-WebSocket-Protocol` `switchboard.v1` / `bearer.<token>` on the events upgrade.
- `core::remote::pairing_url`, `core::remote::certificate_covers`; the pairing invite is a URL (the JSON form is still parsed by `PairingInvite.parse`).
