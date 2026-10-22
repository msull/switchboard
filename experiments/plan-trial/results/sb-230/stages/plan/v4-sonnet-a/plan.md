# Plan for #230: a web control surface on the remote port

One commit. The app runs at every step: the page starts as a stub
`index.html`, and each later step adds to a port that already serves it.
Line references are to this worktree at `7fe8949`. "Facts" is the facts
file for #230; each decision that rests on it names the line.

## Where this departs from the issue

- **"The first cut shows the one-line excerpt the card shows."** The
  excerpt is not on the wire, and the facts file's Settled line that
  `last_message` "exists on the session record" is wrong for this tree.
  `last_message` is a field of the hook event (`src/ports/events.rs:24`,
  `src/adapters/hooks.rs:71`) and of nothing durable: the core drops it
  (`src/core/events.rs:132`, `EventKind::Stopped { pending, .. }`) and
  `SessionRecord` has no such field (no match in `src/core/model.rs`).
  The card's excerpt is computed from the transcript cache
  (`refresh_captions`, `src/app.rs:1093`), and `SessionView`
  (`control/src/lib.rs:636`) carries none. **Plan:** no new read model.
  The page fetches `sessions/{id}/last-response` for the sessions that
  are waiting and for the one open, cuts the first line at 140
  characters, and refetches only when that session's `last_stop_at_ms`
  moves. It never watches those keys, so the watch set of 128
  (`docs/remote-client.md` §4.1) is not pressed. Sessions that are not
  waiting show the card's word and the time since their last stop. An
  `excerpt` field on `SessionView` is a follow-up (end of plan).
- **"The device's scope in the header."** No endpoint returns it after
  `/pair`, though §5 says "the host answers with the device's scope" on
  every later request; the tree does not (`remote_info` is
  unauthenticated, `src/app.rs:3568`). A scope changed in the dialog
  would stay invisible until a send is refused. **Plan:** an additive
  `GET /api/v1/device`, under pairing, answering the caller's own
  `{device_id, name, scope}`.
- **"'Revoked' said plainly."** The host cannot tell revoked from never
  valid: revoking deletes the record and keeps only hashes of live ones,
  and both refusals in `authenticate` (`src/app.rs:3127`) carry the code
  `unauthorized`. **Plan:** the page keeps a non-secret marker
  (`server_id`, `spki_sha256`, `device_id`, name) after pairing. A 401
  with the marker present says "This device is no longer paired. It was
  revoked, or the host forgot it. Pair again." A 401 without one says how
  to pair. No new refusal code.
- **"The events socket for live changes."** The issue assumes a
  click-through browser can open `wss://`. The facts file's Settled
  lines say otherwise for Safari: WebKit bug 41419 is open, and
  code-server's iPad doc says Safari blocks WebSocket unless the
  certificate is installed as a profile. Whether it works with this
  certificate on the owner's phone is Unresolved 1 and For the owner A.
  **Plan:** the page works by REST polling alone, the socket is an
  upgrade it may lose, and the header says "live updates unavailable"
  when it does (decision 2).
- **§5's "a relayed request … fails".** That sentence holds for a native
  client, which compares the key of the connection. It does not hold for
  a browser: the `spki_sha256` the page posts comes from the page, and a
  relay's page posts the genuine key. The issue says roughly this; the
  spec text must say it too (docs section below).
- **"Any path outside `/api/v1/` is refused."** True (`src/app.rs:3023`),
  but only after the request has been queued to the frame. The page needs
  the thread that reads the request to answer (decision 4), so for
  paths outside the prefix the refusal moves to the connection thread and
  the `app.rs` arm stays as a backstop.

## The record

No new record kind. `SCHEMA_VERSION` (`src/core/model.rs:12`, 16 today)
and `VIEWS_SCHEMA_VERSION` (`:612`, 7 today) do not move: nothing in
`model.rs` changes.

- **A browser is a `PairedDevice`** (`src/ports/remote.rs:96`) in
  `devices.json`, which is an unversioned bare list. The name is the
  page's own `device_name` (`Safari on iPhone`, built from
  `navigator.userAgent`), cut to 64 by `AppCore::new_device`. No `kind`
  field and no user-agent field: the name is enough to tell the rows
  apart in the dialog (decision 6).
- **`identity.json` gains two additive `#[serde(default)]` fields on
  `Identity`**: `certificate_names: Vec<String>` (the subjectAltNames the
  certificate was issued with) and `certificate_not_after:
  Option<SystemTime>`. They exist so launch can tell whether the
  certificate is due for reissue without parsing DER. `IDENTITY_SCHEMA_VERSION`
  (`src/ports/remote.rs:17`, 1 today) becomes the value there at
  implementation time plus one, written by `make_identity`. Nothing
  checks that constant on read, so no migration step exists or is
  added; an older file reads with the defaults, which makes its
  certificate due once.
- **Reissue, why and when.** The certificate has no subjectAltName, a
  common name only, no `serverAuth` EKU and a `not_after` in the year
  4096 (`make_identity`, `src/adapters/remote.rs:95`). Apple's rules want
  a SAN, `serverAuth` and at most 825 days (facts: Settled, "Apple's TLS
  server certificate rules"), and a native client never noticed because
  it pins the key. A reissue over the same key keeps `spki_sha256`
  (facts: Settled, "The certificate can be reissued over the same key";
  spec §5 already says "a certificate reissued from the same key (a new
  expiry) still verifies"), so no pairing is lost. Whether Apple's 825
  days applies to a manually trusted leaf and whether an IP SAN
  satisfies Safari are Unresolved 3 and 2; the certificate is built to
  satisfy both readings: SANs `localhost`, `<hostname -s>.local` (DNS),
  `127.0.0.1` and `local_address()` (IP), `serverAuth`, 825 days.
  `load_identity` (`src/app.rs:2782`), in the arm where the Keychain key
  matches the identity, reissues over that key when `certificate_names`
  differs from the names now wanted, when fewer than 60 days remain, or
  when the field is empty, saves through `save_identity` (the atomic
  path), and passes the result to `IdentityLoaded`. A read-only instance
  never reissues. An address that changes while the app runs is not
  followed (follow-up): the browser shows the warning again at the new
  address, which it would for any new address.
- **What persists on the device** is the cookie (decision 2) and the
  non-secret marker in `localStorage`. The page is not a record.

## The signals

- **Nothing new in the diff pass, the watch poll or the hook log.** The
  page reads the events the headset reads: `hello`, `changed`,
  `removed`, `watching`, `resync_required` (`docs/remote-client.md` §4.4),
  and the content keys `conversation` (a Claude Code session) or `screen`
  (anything else) for the open session only.
- **Subscribe first, then snapshot, in both modes.** Socket mode:
  open the socket, wait for `hello`, send
  `{"type":"watch","resource":{"kind":…,"id":…}}` for the open session
  (`subscriber_frame`, `src/app.rs:1704`), wait for `watching`, fetch,
  then apply buffered events by revision. Polling mode: every 5 s refetch
  `projects`, `sessions` per project and `waiting`, and every 5 s the
  open session's content, each applied only when its revision is at or
  past the one held for that request path (§4.2). The page treats an
  `epoch` change and `resync_required` as "refetch everything".
- **A revocation reaches the page two ways:** the socket closes
  (`revoke_closes_the_devices_socket`, `tests/remote.rs:2892`), or the next
  poll gets 401. Both end in one `GET device` probe; a 401 there is the
  "no longer paired" state.
- **The unauthenticated pool of 4** (`MAX_UNAUTHENTICATED`,
  `src/adapters/remote.rs:567`) is the unresolved part (Unresolved 8; the
  connection count of iOS Safari is not known). The default that works
  either way: static replies carry `Connection: close`, so a page load
  holds a slot only for its handshake and one request; the page is three
  files, of which two load in parallel; a connection dropped at accept is
  retried by the browser as any failed GET. Whether a real page load
  still loses files is the owner's phone run (For the owner A).
- **Rate window.** A third `RateWindow` beside `pair` and `info`
  (`Limits`, `:623`) for the page's files, 120 a minute per listener
  (40 page loads), answered 429 from the thread. `/pair` and `/info` keep
  their own windows, so no number of page loads can starve a pairing.
- **The code stays out of logs.** `refuse` logs method and path only
  (`src/app.rs:3754`), and the static arm lives on the connection thread,
  where `read_request` has already split the query off the path and
  nothing logs the target. `RemoteRequest` keeps `query` for the API, so
  the static arm must never format it; a test asserts the reply for
  `/?code=X` is byte-identical to `/`.

## The actions and effects

None. This is the port's path, not the core's: no `AppAction`, no
`Effect`, no `core/` change, so no core test. `serve_remote`
(`src/app.rs:3017`) already answers requests on the frame and `Paired`
and `DeviceSeen` already carry the device. The adapter and the router
change as follows.

### Decision 1: browser trust in the certificate

**The click-through on the local network is the trust step, and the
dialog says so.** The alternative (the host offering its certificate for
download) adds a fourth thing served before pairing, which §5 allows
only as `/info`, `/pair` and now the page; it also rests on what iOS
requires of a manually trusted certificate (Unresolved 3, and whether a
year-4096 or no-SAN certificate would be refused, which the reissue
addresses but cannot prove). That work is a follow-up that needs the
owner's answers (end of plan). The decision does not rest on an
unresolved line: the click-through needs nothing from the host.

What it buys: the page refuses to send the code to a host whose `/info`
`spki_sha256` or `server_id` differs from the URL's `key` and `id`. That
catches a second Switchboard at the same address or port, and a rotated
key. What it does not buy: a page served by an impostor is not a genuine
page, a relay forwards `/info` faithfully, and the host's own
`wrong_key` check (`remote_pair`, `src/app.rs:3639`) is fed by the page.
The dialog says: "A browser cannot check the key itself: it relies on
the certificate warning you accept. Accept it only for this address on
your own network. The link carries the code; it works once and closes
with the window." The last sentence is the answer to Unresolved 6 (the
code in history, tab lists or iCloud tab sync before `replaceState`
runs): the plan assumes yes, and the code is dead after the first
pairing, three failures or five minutes (facts: Settled, "A leaked
pairing code is short-lived", `src/core/remote.rs:46-72`). The dialog
also says the certificate is renewed (key and pin unchanged) and that a
new address shows the warning again (Unresolved 7).

### Decision 2: the token's transport

**An `HttpOnly; Secure; SameSite=Strict; Path=/` cookie, named
`__Host-sb-<server_id>`, set by `/pair`.** The token stays out of every
store a script can read, which the safer-store rule decides: a
`Sec-WebSocket-Protocol` value would need the token in page storage, and
`localStorage` is readable by any script on the origin. The name carries
`server_id` because cookies are not isolated by port (facts: Settled,
"Cookies are not isolated by port"), so two instances on one Mac keep
separate cookies; `__Host-` stops another subdomain from overwriting
it. `Max-Age` is 400 days, the browsers' cap. Another HTTPS service on
the same host name still receives the cookie; that is accepted and named
as a follow-up. The token alone is only useful against this port.

The pairing request gains `"transport": "cookie"`. The reply then sets
the cookie and **omits `token` from the body**, so the page's script
never holds it. Without the field the reply is today's, unchanged, for
the native clients. The host keeps accepting `Authorization: Bearer`;
bearer wins when both arrive. No second browser transport is built.

**The implementer's first step, before the page exists:** with a test
data dir, run `pairing ABCD2345` and, in desktop Safari and Chrome on the
Mac against `https://127.0.0.1:<port>/`, accept the warning and check, with
a throwaway page the dev tools can run, that the cookie is stored after a
`fetch('/api/v1/pair', …)`, is sent on a same-origin `fetch`, and is sent
on `new WebSocket('wss://…')` (Unresolved 5; the mechanics are lead-level
evidence only). Record the three results in the design status section. If
the cookie is not stored or not sent on `fetch`, **stop and report**: the
fallback is a follow-up decision for the owner, not a localStorage token
built here. If it is sent on `fetch` and not on the socket, nothing more
is needed: the polling default already covers it (Unresolved 1 and 5; For
the owner A and D are the phone's results, run after the commit lands).

**The socket and cross-site abuse.** A cookie rides every request a
browser sends to the host, so a cookie-authenticated request has to prove
it is same-origin. In `serve_remote`, after `authenticate`, when the
token came by cookie and the request is not a GET or is an upgrade, its
`Origin` must equal `https://` plus its own `Host` header
(case-insensitive); otherwise 403 `bad_origin`. Comparing with the
request's own `Host` works whether the phone used the IP, a `.local`
name or loopback, which is what makes it settleable without the owner.
`SameSite=Strict` already withholds the cookie from cross-site requests;
this is the second layer, needed because the WebSocket handshake is the
one request a page of another origin can aim at the host. A bearer
request is not subject to it (a header cannot be forged cross-site).

**The router and the adapter.**
- `RemoteRequest` (`src/adapters/remote.rs:360`; built only inside that
  file, and no test builds it) gains `host`, `origin` and `cookies`.
  `read_request` (`:444`) keeps `host`, `origin`, and from `Cookie` only
  the pairs whose name starts `__Host-sb-`, so another service's cookie
  never rides in a struct that derives `Debug`.
- `RemoteReply` (`:389`) gains `set_cookie: Option<String>`. The literal
  sites (about 11, in `src/app.rs` at `:1664`, `:1679`, `:2365-2376`,
  `:2754`, `:3570`, `:3723`, `:3736`, `:3767` and `:829` of the adapter)
  each take `set_cookie: None`; the compiler lists them. Only
  `remote_pair` sets one.
- `authenticate` (`src/app.rs:3127`) takes the token from the bearer or,
  failing that, from `request.cookies` under this installation's name,
  and reports which. `respond` (`:530`) writes `Cache-Control: no-store`
  on every reply and, when present, `Set-Cookie`.
- `GET device` joins the allowlist after `authenticate`, built inline as
  `remote_info` is, from the device already in hand. The body is
  `{device_id, name, scope}` in the usual envelope. No `control/` type,
  no fixture: the Rust test asserts its shape and no Swift client reads
  it.

### Decision 3: one QR

**One link.** `https://<address>/?code=<code>&key=<spki_sha256>&id=<server_id>`,
where `<address>` is `SocketAddr::new(local_address(), port)` formatted by
`Display`, which brackets an IPv6 address. All three values are hex,
uuid or Crockford, so none needs percent-encoding. When there is no
network address the link names `127.0.0.1:<port>`, which still opens on
this Mac. `adapters::remote::pairing_url(address, code, key, id)` is the
one place the string is made, used by the UI and by the end-to-end test.
The JSON form is no longer produced. `PairingInvite.parse`
(`client/Sources/SwitchboardProtocol/Info.swift:109`) tries a URL first
(`URLComponents`: `code`, `key`, `id`, host and port into `addresses` as
one `host:port`) and the JSON second, because an older host's invite
text can still be pasted into a newer client; the JSON branch is kept
only for that. `switchboard-remote pair '<invite>'`
(`client/Sources/switchboard-remote/main.swift:107`) already routes the
one-argument form through `parse`, so it takes the link with no change
except the usage text. The `pair <host:port> <code> <spki>` form and the
headset's typed-code path are untouched. The implementer checks that
`Address` accepts the bracketed IPv6 form `main.swift` and
`ConnectionManager.swift:483` hand it.

### Decision 4: the static paths

- **Paths:** `GET /` (`text/html; charset=utf-8`), `GET /app.js`
  (`text/javascript; charset=utf-8`), `GET /app.css`
  (`text/css; charset=utf-8`). Nothing else. `/favicon.ico` and every
  other path outside `/api/v1/` get a 404 JSON `not_found` from the
  thread; a non-GET to a page path does too. Three files, no favicon.
- **Headers on all three:** `Content-Security-Policy: default-src 'none';
  script-src 'self'; style-src 'self'; connect-src 'self'; img-src
  'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'`
  (no inline script, no inline style, no external resource);
  `Cache-Control: no-store` (the bytes change with the binary, and an
  ETag buys nothing for three small files); `X-Content-Type-Options:
  nosniff`; `Referrer-Policy: no-referrer`; `Connection: close`.
  `connect-src 'self'` is expected to cover `wss://` on the same host;
  older Safari did not always treat it so. That is part of the phone run;
  if the socket is blocked the page is already in polling mode.
- **Where they are answered:** on the connection thread, before the
  frame, as §4.1's "the frame does core state only" reads. In
  `connection` (`:770`), before the `pair`/`info` window check, a GET to a
  page path is checked against the new window and answered; the thread
  then returns, closing the connection. The bytes are constants, so
  nothing about the host rides in them and the §5 rule "no non-loopback
  bind serves anything but `/info`, `/pair` and the page" holds.
- **New file `src/adapters/web.rs`:** the asset table (`ASSETS`, each
  `include_str!("../../web/…")`), `asset(path)`, and the header block.
  No build step, no bundler, no package manager, no dependency.
- **Opened without a code and without a token:** the page asks `GET
  device`; a 401 with no marker shows "Open the pairing window on your
  Mac (Settings, Remote devices) and scan its QR code with this device's
  camera." with the host name from `/info`. No code-entry form in this
  cut: a typed code would pair on the key `/info` just presented, which a
  browser cannot check, so the form would be the weak path with a keypad
  (follow-up).

## The UI

**The page** (`web/index.html`, `web/app.js`, `web/app.css`; plain
JavaScript, no framework):
1. If the query holds `code`, take `code`, `key` and `id` into variables
   and call `history.replaceState(null, '', location.pathname)` as the
   script's first statement, before any request.
2. `GET /api/v1/info`. If `spki_sha256 !== key` or `server_id !== id`,
   say "This is not the host whose code was scanned" and send nothing.
   If a marker exists for this `server_id` and the key differs from it,
   say "Identity changed, pair again" and send nothing.
3. `POST /api/v1/pair` with `{code, device_name, spki_sha256: key,
   transport: "cookie"}`. On 200 store the marker and show the sessions.
   `wrong_code`, `wrong_key` and `pairing_closed` each say what to do
   (scan again; open a new window).
4. **Sessions:** projects (`GET projects`, then `sessions` per project;
   `GET waiting` for the order), waiting sessions first and marked, each
   with its name, `kind`, the card's word and, for waiting ones, the
   excerpt (departure 1). Grouped by project below the waiting block.
5. **Session view:** `last-response` for a Claude Code session, `screen`
   for anything else, refreshed on `changed` in socket mode and on the
   5 s poll otherwise. Operation ids use `crypto.randomUUID` when
   `window.isSecureContext`, else a hex id from `getRandomValues`
   (Unresolved 4: whether a click-through certificate is a secure
   context). A reply box posts `sessions/{id}/send` with `{op, text}`
   only when `GET device` says `control`, re-read on every list refresh
   so a scope changed in the dialog shows without a reload. A timeout is
   resolved by `GET ops/{op}` before the same `op` is sent again (§4.5).
   A session whose `prompt_refusal` is set shows the refusal instead of
   the box; one at the folder-trust question says to answer it on the
   Mac. Resume and trust are not offered on the page.
6. **Header:** host name, device name, scope, and "live" or "live updates
   unavailable". The states "no longer paired", "identity changed, pair
   again" and "pairing closed" replace the body and say what to do.
   There is no sign-out: the owner revokes in the dialog.

**The dialog** (`src/ui/remote.rs`, `pairing` at `:141`, `devices` at
`:230`):
- the QR and "Copy invite" (`:201`, now the link) use `pairing_url`
  instead of the `json!` at `:160`; the hover text says "The QR code's
  link";
- the line under the code (`:187`) gains the browser sentence from
  decision 1 and, when the port is bound to this Mac only
  (`identity.listen.lan` false), "Listening on this Mac only; allow the
  local network above for a phone";
- a line under the Pairing section says the certificate is renewed with
  the key unchanged (Unresolved 3).
The device list shows the browser row by its name; nothing else changes.

**Dev aid:** `pairing <code>` (`src/script.rs:730`) already opens a window
with a known code, which is how the page is driven by hand against a test
data dir. No new script line.

## The tests

Run `cargo test --locked --workspace`, clippy with `-D warnings` and
`cargo fmt --all` before the commit. The page's JavaScript has no runner
and the issue rules out a dependency for one; its logic (query read,
`replaceState`, reconcile by revision) is checked by hand, which the
status section says plainly.

**`tests/remote.rs`.** The helper `http` (`:245`) reads only
`Content-Length`; add `http_full` returning status, headers and body, with
`Host: localhost:<port>`, optional `Cookie` and `Origin`, and make `http`
call it.
- `the_page_is_served_to_anyone_with_its_headers`: unpaired, the three
  paths answer 200 with the Content-Type, CSP (no `unsafe-inline`, no
  `http:`, no `*`), `no-store`, `nosniff`, `no-referrer` and `close`;
  `/favicon.ico`, `/x` and `POST /` answer 404 JSON; `/?code=X` is
  byte-identical to `/`.
- `the_page_names_only_paths_the_host_serves`: every `src` and `href` in
  the embedded `index.html` is in `ASSETS`; every `/api/v1/…` literal in
  `app.js` (a template placeholder replaced by a seeded id) answers
  anything but the 404 "not available on this port" for a paired
  control device. This is what keeps the page and the allowlist in step.
- `page_loads_have_their_own_window_and_free_their_slots`: eight page
  loads in a row leave `/api/v1/info` answering (the slots free at
  `close`); past 120 in a minute the next is 429 while `/pair` and `/info`
  still answer, in the pattern of `pair_is_rate_limited_per_listener`
  (`:1406`) and `stalled_strangers_fill_their_own_pool_and_never_a_paired_clients`
  (`:3226`).
- `a_browser_pairs_from_the_link_the_dialog_shows`: the end-to-end flow
  through the host's own client, as the CLI does. Build the link with
  `pairing_url`, split its query as the page does, read `/info`, compare
  `key` and `id`, post `/pair` with `transport: "cookie"`; assert the body
  has no `token`, the `Set-Cookie` is exactly the name, `Path=/`,
  `Secure`, `HttpOnly`, `SameSite=Strict` and `Max-Age=34560000`, and
  that a second `/pair` with the same code is refused (single use). A
  wrong `key` in the link is refused and counts against the window.
- `a_cookie_reads_and_a_cookie_command_needs_its_origin`: with only the
  cookie, `GET device` answers the scope and name; a cookie named for
  another `server_id` is 401; a `POST send` with no `Origin` or a foreign
  one is 403 `bad_origin` and sends nothing; with `Origin:
  https://localhost:<port>` it sends once per op; a `GET` with the cookie
  and no `Origin` reads; the events upgrade with the cookie and a foreign
  `Origin` is refused, with the right one is 101 (`subscribe_at`, `:1960`,
  extended to take a cookie); a bearer request needs no `Origin`.
- `device_reports_a_changed_scope_and_a_revoked_device_is_refused`: a scope
  change in the dialog shows on the next `GET device`; a revoke makes it
  401 with the cookie, and the socket closes as in `:2892`.
- `a_legacy_certificate_is_renewed_at_launch_over_the_same_key`: seed an
  identity with empty `certificate_names` and a paired device; after
  launch the certificate differs, `spki_sha256` is equal, the device's
  token still reads, and a read-only instance does not rewrite
  `identity.json` (beside `a_missing_key_at_start_remakes_the_identity…`,
  `:1422`).
- Fixtures (`client/Fixtures`, `the_client_fixtures_decode_on_both_sides`):
  unchanged, because no view and no body the native clients read changes.

**Unit tests beside the adapter.** `certificate_params`: the SANs, the
`serverAuth` EKU, a validity of at most 825 days, and the same
`spki_sha256_of_cert` after a reissue over the same key; `certificate_due`
over empty names, changed names, under 60 days and not due;
`read_request` keeps `host`, `origin` and the `__Host-sb-` cookies and
drops the rest; the origin comparison, the cookie parse and `pairing_url`
(including an IPv6 address and the `127.0.0.1` fallback); `respond` writes
`no-store` and `Set-Cookie`.

**`tests/ui.rs:7424`** `remote_devices_dialog_opens_pairing_and_revokes_a_device`:
the copied invite is parsed as a URL (`https`, host and port, `code`,
`key`, `id`) in place of `invite["addresses"]`; the browser sentence is
found by its text.

**Swift** (`client/Tests/SwitchboardProtocolTests`): `PairingInvite.parse`
reads the link (host and port into `addresses`, a bracketed IPv6 host) and
still reads the JSON; run the client's tests as `client/README.md` says.

**By hand, recorded in the status section:** the implementer's cookie
check (decision 2) and the acceptance steps on the Mac in a desktop
browser: open the window, load the link, accept the warning, see the
sessions, start a session waiting and watch it move to the top, send a
reply as a `control` device, revoke the device and see the page's next
request say so; and the same with the socket blocked, which must still
pass by polling. The phone run (For the owner A to D) is the owner's and
goes under Known gaps until it is done; nothing in the acceptance steps
waits on it.

## Docs, in the same commit

- `docs/remote-client.md`: §4.1 Bounds (the page's window, and that
  static paths are answered on the connection thread and close); §4.3
  (the `device` row, the three page paths, `transport` on `/pair`); §5
  (the "before anything but `/info` and `/pair`" bullet gains the page;
  the QR bullet becomes the link; the relay sentence says it holds for
  native clients only; the token bullet says a browser's token is a cookie
  the page never reads, and that the code travels in the link and dies
  with the window; "the host answers with the device's scope" becomes
  `GET device`); a short §7.4 "Browser page". `docs/design.md`: a status
  paragraph at the end of "Remote client (2026-10-12)" (before the
  heading at `:3585`), with Known gaps: the phone run, the excerpt only
  for waiting sessions, no sign-out, no code-entry form, the certificate
  not following an address that changes while running.
- `README.md`: Layout table rows for `web/index.html`, `web/app.js`,
  `web/app.css` and `src/adapters/web.rs`; the dev-aid text (`:262-279`)
  for "Copy invite" now copying a link, `switchboard-remote pair
  "$(pbpaste)"` still pairing it, and opening the page with
  `pairing <code>`; the data directory entry for `identity.json` if it
  lists contents.
- `CHANGELOG.md`, under Unreleased: "A browser can control the host over
  the remote port: scan the pairing window's QR code with a phone, accept
  the certificate warning, and the page lists the sessions with the
  waiting ones first, shows a session's last response or screen and sends
  a reply."

## Left for the owner

Each of these would change a decision the spec or the design already
made, or needs an answer this plan does not wait on:
- **The certificate offered for download or as a profile**, if the phone
  run (A) shows Safari will not run the page after a click-through. It
  needs Unresolved 1 and 3 answered, and it adds a path served before
  pairing.
- **A different browser token transport**, only if the cookie check in
  decision 2 fails: a decision about where a credential lives.
- **An `excerpt` on `SessionView`**, fed from the conversation cache the
  card already uses: a contract field and fixtures on both sides.
- **Reissuing the certificate when the address changes while running**
  (a rebind), and the `.local` name in the link if B says it resolves.
- **The cookie reaching other HTTPS services on the same host name**
  (cookies are not isolated by port).
- **A distinct refusal code for "no token" against "unknown token"**, a
  sign-out that clears the cookie, and a code-entry form on the page.

## Commit

`A browser page on the remote port: a pairing link, cookie auth, the sessions and a reply box`

**Contract additions:**
- `GET /api/v1/device` (paired): `{device_id, name, scope}` in the usual
  envelope.
- `/api/v1/pair` request field `transport` (`"cookie"`); with it the reply
  sets the `__Host-sb-<server_id>` cookie and carries no `token`.
- Static `GET /`, `/app.js`, `/app.css`, answered before pairing on the
  connection thread; error code `bad_origin` (403).
- The pairing QR and "Copy invite" carry `https://<address>/?code=&key=&id=`;
  `PairingInvite.parse` accepts it as well as the JSON.
- `Identity.certificate_names` and `Identity.certificate_not_after`
  (additive, defaulted); `IDENTITY_SCHEMA_VERSION` from the value in
  `src/ports/remote.rs` at implementation time, plus one. `SCHEMA_VERSION`,
  `VIEWS_SCHEMA_VERSION` and the unversioned `devices.json` do not change.
- `RemoteRequest.host`, `.origin`, `.cookies` and `RemoteReply.set_cookie`
  (adapter types, not `model.rs` or `ports/`).
- `adapters::remote::pairing_url`, `adapters::web::{ASSETS, asset}`.
