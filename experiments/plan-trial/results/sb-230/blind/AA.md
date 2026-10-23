# Plan: issue #230, a web control surface on the remote port

The host serves a small page from the remote port it already has; a
phone's camera opens it from the pairing window's QR, pairs, and lists
the sessions with the waiting ones first. Read against the worktree at
HEAD `7fe8949`; "Settled", "Unresolved" and "For the owner" name
sections of the facts file.

## Where this departs from the issue

- **"The browser accepts the certificate warning."** The certificate has
  no subject alternative name (SAN) and a common name only
  (`src/adapters/remote.rs:101-107`), and Chrome has ignored the common
  name since 58 (Settled, The world). The served certificate is issued
  with the host's address as an IP SAN over the same key, so the SPKI
  hash and every pin are unchanged (Settled, The tree:
  `src/adapters/remote.rs:113`). Whether a click-through is then offered
  is Unresolved and is a stop condition (decision 1).
- **"Open `/api/v1/events` directly."** WebKit does not apply a
  clicked-through exception to `wss` (Settled, The world: WebKit bug
  41419). The page polls `/api/v1/waiting` and the open session; the
  socket is not in this commit (follow-ups). "A session that starts
  waiting moves to the top without a reload" holds by the poll.
- **"The device's scope in the header."** Nothing returns the scope after
  pairing (`/pair` reply only, `src/app.rs:3662`) and an `HttpOnly`
  cookie leaves the page no copy, so `GET /api/v1/me` is added.
- **"Sessions grouped by project."** There is no all-sessions listing
  (`docs/remote-client.md:169`), so the page reads `projects`, then each
  project's `sessions`, and orders by `waiting`. No new read model.
- **"One QR carrying addresses."** With no network `local_address()` is
  `None` (`src/adapters/remote.rs:958-970`) and a URL has no host; the
  window then shows the code and key as text only.
- **"The token is sent only as a bearer header over the pinned
  connection, never in a URL"** (`docs/remote-client.md:312`, §5). A
  browser gives up both halves. That sentence is amended in place:
  native clients keep the rule; a browser holds the token only as an
  `HttpOnly` cookie over an unpinned connection, and it is still never
  in a URL.
- **"`?code=...` in the URL."** The invite uses a fragment so the code
  never reaches the host's request line or a `Referer` (decision 3).

## Decisions

1. **Trust: the click-through is the trust step, not a certificate
   download.** A download is a second pre-pairing path served before
   pairing, and a profile install on iOS. The dialog says, next to the
   QR: "A browser cannot check the host's key. Pair a browser only on a
   network you trust; the invite is spent once used." Pairing scope
   still defaults to read. §5's rule widens by one case: before pairing
   a non-loopback bind serves only `/info`, `/pair` and the three page
   files. Whether a click-through is offered for an IP-only SAN is
   Unresolved (facts, Unresolved 1 and 2), so it is a **stop
   condition**, tested first in spike 23: if a real iPhone or iPad
   Safari, or current Chrome, offers none to the spike's IP-SAN
   certificate, the implementer stops and reports before building
   anything else, and the owner picks the certificate download (not
   built here).
2. **Token: an `HttpOnly; Secure; SameSite=Strict` cookie, only that.**
   A cookie script cannot read beats `localStorage` any script on the
   origin can. `sb_token=<token>; Max-Age=31536000; Path=/api/v1/;
   Secure; HttpOnly; SameSite=Strict`. `Strict` costs nothing: the page
   needs no cookie to load and `fetch` is same-origin (Settled, The
   world: SameSite). No sign-out in this cut: revoke in the dialog.
   Whether a clicked-through origin stores a `Secure` cookie and sends
   it on `fetch` is Unresolved and is the second stop condition in
   spike 23; the fallback token store is a follow-up for the owner.
3. **One QR, a URL with the secrets in its fragment:**
   `https://<ip>:<port>/#code=<code>&key=<spki>&id=<server_id>`. A
   browser never sends a fragment, so the first `GET /` carries nothing
   secret; the script still clears it with `replaceState`. The JSON
   form is no longer produced. The native text path
   (`PairingInvite.parse`, `client/Sources/SwitchboardProtocol/Info.swift:94-117`,
   used by the CLI and the headset's paste field) accepts the URL as
   well as the JSON, so `testInviteParsesTheQRText`
   (`client/Tests/SwitchboardProtocolTests/FixturesTests.swift:157-165`)
   stands.
4. **Static paths.** Exactly `GET /` (`text/html; charset=utf-8`),
   `GET /app.css` (`text/css; charset=utf-8`), `GET /app.js`
   (`text/javascript; charset=utf-8`), answered on the connection
   thread, not the frame. Every page response carries
   `Cache-Control: no-store` (Unresolved whether a browser caches a
   certificate-error origin, so no conditional request is relied on),
   `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`
   and `Content-Security-Policy: default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'`.
   The page links `rel="icon" href="data:,"`. Another method on those
   paths is 405; any other path stays the 404 `not_found` it is today
   (`src/app.rs:3023`). Opened with no fragment and no pairing the page
   says "This device is not paired. On the Mac: Switchboard, Remote
   devices, Allow pairing, then scan the QR code." Sources are
   `web/index.html`, `web/app.css`, `web/app.js`, embedded with
   `include_str!`.

## The record

No record changes, no schema bump. A browser is a `PairedDevice`
(`src/ports/remote.rs:96`) unchanged: the cookie's token hashes to the
same `token_sha256`, so revocation (`src/app.rs:486`) and key rotation
work as they do. The page names it "Safari on iPhone", "Safari on iPad"
(Macintosh user agent with `maxTouchPoints > 0`, a heuristic; Settled,
The world), "Chrome on Android", else "Browser".

- **Certificate.** `identity.json` keeps the original SAN-less
  certificate; the served one is derived at bind from the Keychain key,
  the server id and the address, in memory only, so the pin is untouched.
- **Browser side.** The page remembers `{server_id, spki_sha256,
  server_name}` in `localStorage` (a convenience, not a credential, in a
  try block) so a 401 can read "revoked" and a changed `/info` key
  "identity changed, pair again"; without it both read "not paired".
- **Pools** (`src/adapters/remote.rs:567-568`, accept loop `:700-730`).
  Every accept counts in the 4 unauthenticated slots today, so a page
  load would crowd `/info` and `/pair`. A third pool, `MAX_PAGE = 6`,
  takes connections whose first request is a page file (503 `busy` when
  full); one that later authenticates moves to the paired pool as an
  unauthenticated one does. Total 22. Page files get a rate window of
  their own, 120 a minute per listener, 429 past it (`Limits::limited`,
  `:583-640`, is false for unlisted paths today). Idle stays 30 s.
  §4.1 is amended.

## The signals

No new wire signal and no event. Every 3 s the page reads
`/api/v1/waiting`; when the reply envelope's `revision` differs from the
held one it refetches `projects`, each project's `sessions` and the open
session's content, applying a reply only if its revision is at or past
the held one (§4.2). A Claude Code session shows `last-response`; on
`400 no_conversation` (Settled, The tree: `src/app.rs:1869-1886`, also
the answer for a non-Claude session) it shows `screen?lines=60`. Text
is placed with `textContent`. Errors in the page's words: a 401 is
"revoked or never paired" per the remembered record; `/info` key
different from the remembered one, "This host's identity changed. Pair
again." with nothing sent, cookie included; the URL's key different
from `/info`'s, "This is not the host this invite was made for." with
`/pair` not posted (this proves a wrong Switchboard at the address, not
an impostor); 403 `pairing_closed`, `wrong_code`, `wrong_key`, 429 and
503 each have a sentence.

## The actions and effects

No `AppAction`, no `Effect`. One pure function in `src/core/remote.rs`:
`invite_url(address, key, code, server_id) -> String`, used by
`ui/remote.rs` in place of the JSON built at `:156-166`.

- **Certificate.** `tls_config_for(identity, key_pem, addresses)` in
  `src/adapters/remote.rs`; `tls_config` (`:144`) delegates with none.
  Same common name and key, `SanType::IpAddress` for `127.0.0.1` and
  each address; its SPKI hash must equal `identity.spki_sha256`.
  `bind_remote` (`src/app.rs:2936-2990`) passes `local_address()` when
  `listen.lan` and keeps it as `remote_cert_ip`; the first bind computes
  the SAN and no more. A rebind happens **only if** an `OpenPairing`
  finds `local_address()` different from `remote_cert_ip`
  (`refresh_remote_certificate()` in `SwitchboardApp::dispatch`,
  `src/app.rs:452`, returns when equal). A rebind clears `self.remote`
  and drops every paired socket; that is once per address change, not
  per window. After an address change, cookie-paired devices keep
  working and a fresh QR needs the rebind to get the new address into
  the SAN. The pairing UI reads the address from the app (`DrawCtx` gains
  `remote_lan_ip`, `src/ui/mod.rs:401,420`), so the QR carries the
  address the certificate was issued for and the draw no longer opens a
  UDP socket per frame.
- **Request.** `RemoteRequest` (`:360-385`) gains `cookie` (the
  `sb_token` value only, kept out of logs as the bearer is), `origin`
  and `host`, read in `read_request` (`:444-517`).
- **Reply.** `RemoteReply` (`:389`) gains `set_token: Option<String>`;
  `respond` (`:530-552`) becomes `respond_with(w, status, content_type,
  headers, body)`, every response carries `Cache-Control: no-store` and
  `X-Content-Type-Options: nosniff`, and `set_token` becomes the
  `Set-Cookie`.
- **Page.** In `connection` (`:770-875`), before the rate check, a `GET`
  of one of the three paths is answered from a const table of
  `(path, content_type, include_str!(...))`, outside the frame and
  `INBOX`.
- **Authenticate.** `authenticate` (`src/app.rs:3128-3160`) takes
  `request.bearer` else `request.cookie`. A cookie-authenticated
  non-`GET` needs `Origin` equal to `https://` plus the `Host` header,
  else 403 `origin` (`authenticated: false`), beside `SameSite=Strict`.
  Native clients send a bearer and are not checked.
- **`/pair`.** `remote_pair` (`:3606-3665`) reads an optional boolean
  `cookie`. When true the reply sets `set_token` and omits `token` from
  the body, so the page never sees it. Absent or false is today's reply,
  byte for byte.
- **`GET /api/v1/me`.** Read scope; `{device_id, name, scope}`. One arm
  in `serve_remote` (`:3017-3127`), a row in §4.3, and `"web"` added to
  `info`'s capabilities.

## The UI

`src/ui/remote.rs`, `pairing` (`:141-227`): the QR encodes
`invite_url(...)` and `Copy invite` copies the URL; the text gains the
address line, the sentence from decision 1, and that the invite is
spent once used; "Scan, or type the key and the code" stays for the
headset. With no address (`remote_lan_ip` is `None`, or loopback-only)
there is no QR, only the code and key as text and "No network address to
put in a QR code."

The page: no framework, no build, no dependency; no inline script or
style. `app.js` reads `location.hash` first and calls
`history.replaceState(null, "", location.pathname)`, then `/info` →
compare key and id → `POST /pair {code, spki_sha256, device_name,
cookie: true}` → `GET /me` → lists. The reply box is shown only when
`/me` says `control`; it posts `/sessions/{id}/send` with `{op, text}`,
`op` from `crypto.getRandomValues`, and a failed or timed-out send is
followed by `GET /ops/{op}`, never a blind resend (§4.5). Sessions are
grouped under their project, waiting first then by `last_activity_ms`,
each showing `kind`, `liveness`, `card` and `waiting_reason`. The
excerpt the issue names is not on the wire (Settled, The tree), so the
last response is shown in the session view. `app.css`: system fonts,
light and dark, 16 px gutters, no horizontal scroll at phone width, tap
targets of 44 px.

## The tests

First, before any other code, **spike 23**
(`spikes/23-web-cookie/README.md`, added to the README's `spikes/`
line): a `python3` `http.server` over TLS with an `openssl` certificate
carrying an IP SAN, a path that sets the cookie above and a path that
echoes the `Cookie` header, opened on the LAN from a real iPhone or iPad
Safari and Chrome. It records the commands and what was seen: whether a
click-through is offered (with and without the SAN), whether the cookie
is stored and sent by `fetch`, whether it survives a reload and a second
tab, and, as seen, whether a `wss` opens. The first two are the stop
conditions of decisions 1 and 2, in that order; the rest change nothing.

- Core: `invite_url` for a known input equals the literal the Swift
  test uses.
- Adapter: `read_request` extracts only `sb_token`, `origin`, `host`
  and rejects a head over `MAX_HEAD` with a long cookie;
  `respond_with` writes the headers; `tls_config_for` serves a
  certificate whose DER holds the IP SAN and whose SPKI equals the
  identity's; a page request lands in the page pool, the seventh is
  503, `/info` is still served meanwhile, the 121st page read in a
  minute is 429.
- `tests/remote.rs` (a helper returning response headers is added beside
  `http`, `:245`, which drops them): the three paths answer 200 with the
  exact `Content-Type`, CSP, `Cache-Control`, `Referrer-Policy` and
  `nosniff`; `POST /` is 405; `GET /x` is 404 `not_found`; `GET /` is
  answered without the test polling the frame (beside
  `the_allowlist_refuses_everything_else_with_no_side_effect`, `:682`).
  Pairing with `cookie: true` as the CLI does: no `token` in the reply,
  `Set-Cookie` has each attribute, `GET /me` with only the cookie
  answers the scope, a bearer still works, a cookie `POST` with a
  foreign `Origin` is 403 `origin`, a revoked device's cookie is 401,
  `pair` without `cookie` is unchanged.- UI (`tests/ui.rs`): with `remote_lan_ip` seeded the window shows the
  URL and the browser sentence; with `None`, "No network address".
- Swift: `testInviteParsesTheURL` with the same literal; a non-`https`
  URL or a missing `code` throws.

The acceptance steps need a device and a tap: `remote-listen lan <port>`
and `pairing <code>` from `SWITCHBOARD_SCRIPT` (both exist,
`src/script.rs:729-737`), scan, click through, see the sessions, start a
waiting session and watch it move up, send a reply, revoke in the
dialog. The result goes in design.md's status section as seen.

## Docs

One commit; the app runs at every step. `docs/remote-client.md` §4.1
(22 connections, the page pool and rate), §4.3 (`/me`, the page files),
§5 (the URL with its fragment; line 312 amended in place; the browser
trust paragraph; "before pairing only `/info`, `/pair` and the page
files"; the key in the URL and its limit) and §11; `docs/design.md`
remote status section (line 3107): a bullet for the page and its gaps
(no socket, no sign-out, no excerpt, the unresolved items as seen);
README Layout (`web/`, the changed rows, the spikes line) and the
dev-aid note at `:277` (the clipboard text is a URL;
`switchboard-remote pair "$(pbpaste)"` still pairs it); `CHANGELOG.md`
Unreleased.

## Follow-ups for the owner

- If spike 23 shows no click-through with the IP SAN: serve the
  certificate for download (For the owner 1).
- If the `Secure` cookie is not stored: the fallback store
  (`sessionStorage` header) is the owner's call on the weaker store.
- The events socket, once spike 23 shows a `wss` opening: the page
  opens it, slows the poll, and watches the open session (§4.4), with
  an `Origin` check on the upgrade.
- A sign-out that clears the cookie.
- Dispatch tickets, the full conversation, drafts and dictation on the
  page, as the issue says.

## Commit message

Serve a web control surface from the remote port: a browser pairs from the QR's URL, holds an HttpOnly cookie, and lists, reads and answers sessions

## Contract additions

- `RemoteRequest`: `cookie`, `origin`, `host` (`Option<String>`); `RemoteReply`: `set_token: Option<String>`.
- `adapters::remote`: `tls_config_for`, `respond_with`, the page table and `MAX_PAGE` pool.
- `core::remote::invite_url`; `DrawCtx::remote_lan_ip: Option<IpAddr>`.
- HTTP only, no schema bump: `GET /api/v1/me`; optional `cookie` boolean on `POST /api/v1/pair`; `"web"` in `info`'s capabilities; 403 `origin`; `Cache-Control: no-store` and `nosniff` on every response.
- Swift `PairingInvite.parse` accepts the URL form.
- New files: `web/index.html`, `web/app.css`, `web/app.js`, `spikes/23-web-cookie/README.md`.
