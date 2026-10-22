# Plan: a web control surface on the remote port (#230)

The host serves three embedded files from the remote port. A browser pairs from a URL the pairing window shows as a QR, holds its token in an `HttpOnly` cookie, and reads and replies over the API the headset already speaks. One commit.

## Where this departs from the issue

- **"The one-line excerpt the card shows" is not on the wire.** The card's excerpt is UI state: `agent_excerpt` over the conversation cache for an agent (`src/ui/cards.rs:177`) and `UiState.captions` for anything else. `remote-client.md` §4.3 (line 207) says it is `last_message` on the session's metadata, but `last_message` is only a field of the hook event (`src/ports/events.rs:24`) and `SessionView` (`control/src/lib.rs:637`) has no such field. The list row therefore shows what the view carries: the card word (`card`), `waiting_reason` when waiting, the kind, and the time of the last Stop. The excerpt appears as the first lines of the session view's last response. Putting a redacted excerpt on `SessionView` means a masked field, a fingerprint input and an event per Stop, which is its own issue. I amend §4.3's sentence to say the excerpt is not a wire field.
- **"The host answers with the device's scope" (§5, line 316) is not true.** Only the `/pair` reply carries `scope` (`src/app.rs:3662`), and the Swift client keeps what it was told then (`ServerConnection.swift:232`). A scope changed in the dialog (`SetDeviceScope`) is visible to a client only as a 403 `scope`. The header's scope and the hidden reply box cannot work with an `HttpOnly` token, so the page cannot stash anything at pairing and expect it to stay true. Plan: one small read, `GET /api/v1/device` (below). This is the "new read model the page cannot do without".
- **A trusted certificate is not on offer, so the download option is closed.** `make_identity` builds the certificate with no names (`CertificateParams::new(Vec::new())`, `src/adapters/remote.rs:102`). No browser will accept it for `https://192.168.x.x`, installed or not, and a name-bearing certificate would have to follow the DHCP address. See decision 1.
- **"The page reads the query … `key` … compares with `/info`" is sound only as a mistake check.** The issue says so itself; the plan makes the dialog say it (decision 1).
- **`GET /` is not refused today in a way a new route can ignore.** `serve_remote` answers every path outside `/api/v1/` with 404 on the frame (`src/app.rs:3023`). The page paths are answered earlier, on the connection thread, so the frame never sees them. Everything else outside `/api/v1/` still reaches the frame and still gets 404.

## The record

No record change. `SCHEMA_VERSION` is 16 (`src/core/model.rs:12`), `VIEWS_SCHEMA_VERSION` is 7 (`model.rs:612`), `IDENTITY_SCHEMA_VERSION` is 1 (`src/ports/remote.rs:17`); `devices.json` is a bare, unversioned array. None changes. A browser is a `PairedDevice` like any other: its name is a string, its scope the pairing window's, its token's hash the same SHA-256. Nothing marks it as a browser, and the dialog needs nothing for it beyond the name. If the implementer finds a record field is needed after all, the bump is the value in `model.rs` or `ports/remote.rs` at that time, plus one, with a step in `adapters::store::migrate`.

## The signals (the wire)

### Decision 1: trust in the self-signed certificate

**The click-through is the trust step. The host does not offer its certificate.**

- Why not the download: with no names in the certificate (above) an installed copy still fails the hostname check on an IP address, and the owner would trust a key for the whole device to reach one page. The click-through trusts one host and port in one browser.
- What it buys: encryption on the local network, and a certificate the browser remembers for that address, so the WebSocket opens without a second warning. I have not run this in a browser. Acceptance step 1 includes Safari on iPhone and Chrome on Android; if either refuses `wss://` or a `Secure` cookie after a click-through, the plan stops there and the issue is reopened.
- What it does not buy: any proof of who answered. A browser cannot read the certificate, so it cannot pin the SPKI. The `key` and `id` in the URL make a **genuine page** refuse to send the code to a host whose `/info` differs (a stale address, another Switchboard on the LAN, a reused DHCP lease). A page served by an impostor is the impostor's page and skips the check. An attacker in the middle at the moment of pairing could therefore take the code and the token, and the browser path is weaker than the native path on exactly this point. The pairing dialog says so in one sentence beside the QR: "A browser cannot pin the key. Pair a browser only on a network you trust."
- §5's rule is kept and restated: before pairing, a non-loopback bind serves `/api/v1/info`, `/api/v1/pair` and the three page files, and nothing else. The files hold no secret, no per-installation value and no version.

### Decision 2: how the token reaches the WebSocket

**An `HttpOnly; Secure; SameSite=Strict` cookie. Scripts never see the token.**

- `POST /api/v1/pair` accepts an additive body field `"cookie": true`. The reply then carries `scope`, `server_id` and `device_id` but **no `token`**, and sets `sb_<first 12 hex of server_id>=<token>; HttpOnly; Secure; SameSite=Strict; Path=/api/v1/; Max-Age=34560000`. The name carries the server id because cookies are per host, not per port, and a second instance on another port (`SWITCHBOARD_REMOTE_PORT`) would overwrite a shared name. Max-Age is 400 days, the longest Chrome keeps; tokens do not expire, so a lapsed cookie means pairing again.
- A request without `cookie: true` is answered as before, so the CLI and the headset are untouched.
- Authentication accepts `Authorization: Bearer` first and the `sb_` cookie second. Cookies that are not `sb_`-prefixed are dropped when the request is read.
- The token never touches `localStorage`. The page keeps in `localStorage` only non-secret pairing facts: `server_id`, the host's key hash, the host name and a `paired` flag, to word "identity changed" and "revoked" (below).
- **A cookie makes the port a CSRF target, so cookie-authenticated requests are guarded.** The cookie's `SameSite=Strict` is the first layer, but it is not enough alone (a different port on the same host is same-site). Rules, applied in `authenticate` only when the token came from the cookie:
  - a non-GET request needs the header `X-Switchboard-Web: 1`; a cross-origin page cannot add a custom header without a preflight, and the host answers none. Refused 403 `csrf`;
  - the WebSocket upgrade needs `Origin` equal to `https://` plus the request's `Host`, compared case-insensitively. A WebSocket is not subject to CORS, so this is the only check there. Refused 403 `origin`.
- The page's op ids (`send`) come from `crypto.getRandomValues` formatted as a UUID, not `crypto.randomUUID`: a page past a certificate click-through may not count as a secure context. Unverified; the `getRandomValues` form works either way.

### Decision 3: one URL, not two

**The QR is the URL.** `https://<address>:<port>/?code=<code>&key=<spki_sha256>&id=<server_id>`, with an IPv6 address bracketed. The JSON form is no longer written: the QR, the text beside it and "Copy invite" are all the URL. What still needs the JSON is a newer client pasted an invite from an older host, so `PairingInvite.parse` (`client/Sources/SwitchboardProtocol/Info.swift`) reads **both**: a string starting `https://` is split with `URLComponents` into the address (`host:port`), `code`, `key` and `id`; anything else decodes as JSON as today. The headset's paste field and `switchboard-remote pair '<invite>'` (`client/Sources/switchboard-remote/main.swift:107`) go through that one parser, so neither changes beyond the usage text. The three-argument CLI form is unchanged. An older CLI cannot read a URL; the README says to update it.

With no usable address the dialog shows the code and key and no QR. With "Allow devices on the local network" off the port answers only this Mac, so the URL uses `127.0.0.1` and the dialog says a phone needs the setting on (today it shows an unreachable LAN address).

### Decision 4: the static paths

Three paths, `GET` only (any other method on them: 405), query ignored and never logged. They are answered on the connection thread from `include_str!` bytes, before the request is handed to the frame.

| Path | Content-Type | Source |
|---|---|---|
| `/` | `text/html; charset=utf-8` | `web/index.html` |
| `/app.js` | `text/javascript; charset=utf-8` | `web/app.js` |
| `/app.css` | `text/css; charset=utf-8` | `web/app.css` |

Three paths. `/favicon.ico` stays an ordinary 404 on the frame; `index.html` carries `<link rel="icon" href="data:,">` so browsers do not ask for it.

Every page response carries:

- `Content-Security-Policy: default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'`. No inline script, no inline style, no external resource. If Safari does not match `wss:` to `'self'` (older versions did not), the fallback is `connect-src 'self' wss://<Host>`, with `Host` checked against `[A-Za-z0-9.:\[\]-]+` before it goes in. Only if the manual check shows the need;
- `Cache-Control: no-store`, so a page never outlives the binary that served it;
- `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`, `Cross-Origin-Resource-Policy: same-origin`;
- `Connection: close`.

JSON replies gain `Cache-Control: no-store` too: a cookie-carried transcript must not sit in a browser cache.

**Pool and window.** An unpaired page load counts against the **unauthenticated pool of 4** (§4.1), because the connection is unauthenticated until a request with a valid token is answered. `Connection: close` frees the slot as soon as the response is written, so a load holds at most the two or three parallel fetches (`/app.js`, `/app.css`, the HTML) a browser makes at once. A fifth connection is closed at accept as today, and the browser retries. Page paths get their **own rate window**, `PAGE_PER_MINUTE = 60` per listener (about twenty loads), answered 429 on the connection thread, beside `PAIR_PER_MINUTE` and `INFO_PER_MINUTE` (`src/adapters/remote.rs:583`), so page loads never spend the `/pair` or `/info` budget. After a successful cookie request the connection moves to the paired pool as for any device; a page uses about two paired connections (keep-alive fetches, the socket) of 12.

**Opened with no code and no cookie** the page calls `GET /api/v1/info` (unauthenticated), then `GET /api/v1/device`, which answers 401. It shows the host name and one paragraph: "Not paired. On <host>, open Settings → Remote devices → Allow pairing, and scan the QR code with this device's camera." If `info.pairing_open` is true it adds "A pairing window is open now." There is no field to type a code: a typed code has no key to check against, and the page's one safe path is the scanned one.

### The page's states

- **URL with `code`, `key` and `id`:** read them, `history.replaceState(null, '', '/')` as the **first** statement of `app.js` (before any `fetch`), then `GET /api/v1/info`. If `info.spki_sha256 !== key` or `info.server_id !== id`: stop, send nothing, "This is not the host this code was made for." Otherwise `POST /api/v1/pair {code, spki_sha256: key, cookie: true}`; on 200, store the non-secret facts and go to the list. The host names the device (below), so the page sends no `device_name`. A pair failure says the host's reason (`pairing_closed`, `wrong_code`).
- **Cookie and 200 from `/device`:** the list.
- **401 from `/device` with `paired` in `localStorage`:** "This device was revoked, or its pairing was removed. Pair again from the host." The flag is cleared on the next successful pairing.
- **`/info` key differs from the remembered one for the same `server_id`:** "The host's identity changed. Pair again." and the page sends nothing further (no `/device`, which would carry the cookie).
- **A later 401 mid-session, or a socket that closes and whose reconnect gets 401:** the revoked state above, said on the next request, as the acceptance asks.
- **403 `scope` on a send:** re-read `/device`, hide the reply box.

### Protocol use by the page

The page follows §4.4's order exactly: open `wss://<host>/api/v1/events`, wait for `hello {epoch, revision}`, buffer events, fetch the snapshots, apply a reply only when its `revision` is at or past the one held for that resource key, refetch a resource an event names with a revision past the snapshot's, then enable the reply box. An `epoch` change or `resync_required` refetches everything; a closed socket reconnects with a backoff of 1, 2, 4, up to 15 s.

- List: `GET /projects`, then `GET /projects/{id}/sessions` per project, at most three in flight (a browser keeps six connections per host and the pool of four is shared with other strangers). `changed {session}` refetches that session; `changed {sessions}` the project's collection; `changed {projects}` and `{waiting}` their collections. Sorted: projects holding a waiting session first, waiting sessions first within a project, the rest by name. Each row: name, kind, `card`, `waiting_reason`, time since `last_stop_at_ms`. A session that starts waiting emits `changed {session}` and `changed {waiting}` and re-sorts without a reload.
- Session view: a Claude session reads `GET /sessions/{id}/last-response` and sends `watch {kind:"conversation", id}`; every other kind reads `GET /sessions/{id}/screen?lines=60` and watches `screen`. A `changed` on the watched key refetches it, and a visible view refetches every thirty seconds (§4.4). `unwatch` on leaving. The text is set with `textContent` in a `pre-wrap` element; the page renders no markup from the host.
- Reply box, shown only when `/device` says `control`: `POST /sessions/{id}/send {op, text}` with `X-Switchboard-Web: 1`. A timeout is followed by `GET /ops/{op}`, never a blind resend (§4.5); `interrupted` is said in words. A trust question (`trust_question` on the view) shows the "Trust this folder" button, `POST …/trust`.
- Header: host name from `/info`, the device's name and scope from `/device`, refreshed on each reconnect, on a 403 `scope`, and when the tab becomes visible, since a scope change emits no event.

### The new read: `GET /api/v1/device`

Authenticated (bearer or cookie), `read` scope. Body, in the usual envelope: `{id, name, scope}`. `info.capabilities` gains `"device"`. Route arm in `serve_remote` (`src/app.rs:3044`), answered on the frame from `self.core.devices()`. No event: a scope or name change does not emit, as above.

### Naming the device

`remote_pair` (`src/app.rs:3606`) names a `cookie: true` pairing from the `User-Agent` header through `adapters::web::device_name(user_agent) -> String`: `Safari on iPhone`, `Chrome on Android`, `Safari on iPad`, `Safari on Mac`, `Chrome on Mac`, `Firefox on Windows`, falling back to `Browser`. The existing 64-character cut in `new_device` (`src/core/remote.rs:169`) stands. Two identical phones show identically; the dialog already shows the paired time and last-seen.

## Actions and effects

None new. `Paired`, `RevokeDevice`, `SetDeviceScope` and `DeviceSeen` do all of it. The invite URL is a pure function of core state and an address, so it moves out of `ui/remote.rs:160` into `AppCore::invite_url(&self, address: SocketAddr) -> Option<String>` (`src/core/remote.rs`), which the dialog calls with the address `remote::local_address()` finds (I/O stays in the UI's caller, not the core).

## The UI

Two parts: the dialog and the page.

**The dialog** (`src/ui/remote.rs`): the QR texture and "Copy invite" use `invite_url`; the line "Scan, or type the key and the code" gains "A browser cannot pin the key. Pair a browser only on a network you trust." and, with LAN off, "A phone needs 'Allow devices on the local network'." The paired-devices list is unchanged (a browser is a row, revocable). Revocation already closes the device's socket (`src/app.rs:486`).

**The page** (`web/index.html`, `web/app.js`, `web/app.css`): one HTML shell, one script, one stylesheet, no framework, no build step, no dependency. All API routes live in one `const API = [...]` table at the top of `app.js`, one `{method, path}` per line, so the test can read them. The script uses `textContent` only, never `innerHTML`.

## The tests

- **Adapter unit tests** (`src/adapters/remote.rs`, `src/adapters/web.rs`): `read_request` fills `cookies` (only `sb_`), `origin`, `host`, `user_agent` and the guard header; the page window slides apart from `/pair`'s; `device_name` over the real user-agent strings above; each asset has its content type and carries every header named above.
- **Core test** (`src/core/tests.rs`): `invite_url` builds the exact URL, brackets IPv6, and is `None` without an identity or address.
- **`tests/remote.rs`**, over the real TLS listener with the pinned client, extending `http` (`tests/remote.rs:245`) with extra headers:
  - the page paths answer 200 with the types and headers of the table, a POST to one is 405, an unknown path outside `/api/v1/` is still 404, and page loads do not spend `/pair`'s window (the existing `pair_is_rate_limited_per_listener` still holds);
  - **embedded page references only served paths:** every `src` and `href` in `index.html` is one of the three paths; every row of `API` in `app.js`, with a real project and session substituted, answers something other than 404 "not available on this port" for its method (paired token, `control` scope);
  - **lint on `app.js`:** no `innerHTML`, `outerHTML`, `insertAdjacentHTML`, `document.write`, `eval(`, `new Function`, and no `localStorage` write whose argument names a token; `replaceState` appears before the first `fetch(` in the file; `index.html` has no inline `<script>` body, `style=` or `on…=` attribute;
  - **pairing end to end, as the CLI does:** open a window, `POST /pair {code, spki_sha256, cookie:true}` with a Safari-on-iPhone `User-Agent`: the reply has no `token`, has a `Set-Cookie` with `HttpOnly; Secure; SameSite=Strict; Path=/api/v1/`, the device is named `Safari on iPhone`; a `GET /device` with that cookie answers `{name, scope: read}`; a POST with the cookie and no guard header is 403 `csrf`, with it a `control` device's `send` goes; a WebSocket upgrade with the cookie and a foreign `Origin` is 403 `origin`, with the right one it gets `hello`; the bearer path is unchanged;
  - a wrong key or code through the cookie path counts against the window like any other (`a_pairing_with_another_hosts_key…` at `tests/remote.rs:581`);
  - revoke in the dialog's action: the cookie's next request is 401 and the socket closes (extends `revoke_closes_the_devices_socket`, line 2891); a scope lowered by `SetDeviceScope` shows in `/device` and a send is 403 `scope`;
  - `info` lists `device`; the stalled-strangers test (line 3225) still fills only its own pool with page requests in the mix.
- **Fixtures:** `write_fixtures` (line 1587) writes `client/Fixtures/device.json` and the regenerated `info.json` (new capability); `the_client_fixtures_decode_on_both_sides` (line 1828) asserts the Rust side. The Swift side needs no `DeviceInfo` type until a Swift speaker reads the route, so no Swift decoder is added for it.
- **Swift** (`client/Tests/SwitchboardProtocolTests`): `PairingInvite.parse` over the URL form (IPv4, IPv6, missing parameter refused) and over the old JSON. Run with `swift test` in `client/` if the toolchain is there; say so in the PR if it was not.
- **UI test** (`tests/ui.rs:7474`): "Copy invite" now puts a URL on the clipboard whose `code`, `key` and `id` parameters match; the JSON assertions there are replaced.
- **Not tested:** a real browser. The implementer drives acceptance by hand: scan the QR from the dialog on an iPhone and an Android phone, click through the certificate warning, see the sessions; start a session waiting with `echo '{"tool_name":"AskUserQuestion"}' | switchboard-hook PermissionRequest` and watch it rise; send a reply under a `control` pairing and see it arrive; revoke the phone and see "revoked" on its next request; confirm the address bar and history hold no code. With `SWITCHBOARD_DATA_DIR`/`SWITCHBOARD_TMUX_SOCKET` set as `CLAUDE.md` says.

## Documents

- `docs/remote-client.md`: §4.1 (pools: page paths and their window, `Connection: close`), §4.3 (the table gains `GET /api/v1/device`; the excerpt sentence at line 207 corrected; the allowlist paragraph at line 194 names the page paths), §5 (the QR is the URL; the cookie, its guards and why; the browser's weaker pairing; line 316 corrected), §6/§7 untouched.
- `docs/design.md`: a bullet under "Built so far" in "Remote client" (line 3107) saying what is built, with known gaps: no excerpt on the list, no pin in a browser, the page not driven by any automated browser, the Safari `connect-src` and secure-context questions as checked by hand.
- `README.md`: Layout rows for `web/`, `src/adapters/web.rs`; the `adapters/remote.rs` row (line 383) mentions cookie auth, page paths and the page window; the dev-aid paragraph at line 266 says the invite is a URL, `switchboard-remote pair "$(pbpaste)"` still works, and a browser opens it too (`curl` example unchanged).
- `CHANGELOG.md`, under `[Unreleased]`: "The remote port serves a web page: scan the pairing window's QR code with a phone to see your sessions, which are waiting, and reply. The QR code is now a link."

## Commit message

Serve a web control surface from the remote port, paired from the QR's URL by cookie

## Contract additions

- `GET /`, `/app.js`, `/app.css` on the remote port, answered on the connection thread from embedded files, with the headers in decision 4; `PAGE_PER_MINUTE = 60` rate window.
- `GET /api/v1/device` → `{id, name, scope}`; `info.capabilities` gains `"device"`.
- `POST /api/v1/pair` body gains optional `cookie: bool`; with it the reply omits `token` and sets the `sb_<12 hex of server_id>` cookie (`HttpOnly; Secure; SameSite=Strict; Path=/api/v1/; Max-Age=34560000`).
- Cookie-authenticated requests: `X-Switchboard-Web: 1` on non-GET (403 `csrf`), matching `Origin` on the upgrade (403 `origin`).
- `Cache-Control: no-store` on JSON replies.
- `adapters::remote::RemoteRequest` gains `cookies`, `origin`, `host`, `user_agent`, `web_guard`; `RemoteReply` gains `headers: Vec<(String, String)>` (28 literals in `app.rs` take an empty vec or a constructor).
- `adapters::web` (new): `asset(path)`, `device_name(user_agent)`, the CSP constant.
- `AppCore::invite_url(&self, SocketAddr) -> Option<String>`.
- The pairing invite is the URL `https://<address>:<port>/?code=&key=&id=`; `PairingInvite.parse` reads the URL and the old JSON.
- No change to `model.rs`, `AppAction`, `Effect`, `SCHEMA_VERSION`, `VIEWS_SCHEMA_VERSION` or `IDENTITY_SCHEMA_VERSION`.
