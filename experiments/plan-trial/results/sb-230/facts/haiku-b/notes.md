# Issue #230: a web control surface on the remote port. Investigation notes

Worktree: `/Users/sbgragent/.dispatch/worktrees/340ccb28`, branch `dispatch/230-a-web-control-surface-the-remote-port-se`, base `7fe8949` (Release 0.4.1). Read-only investigation: no code, tests or builds were run, and no browser was used.

Sources read: `CLAUDE.md` (given in context), `docs/remote-client.md` (lines 1–396 in full; 397–553 only by grep and targeted reads of §10 and §13), `docs/lessons.md` (in full), `docs/design.md` (trust boundary 219–266; remote status 3107–3170; Open questions 3672), and the code listed below.

## 0. Short version

- The remote port is an authenticated JSON API and nothing else. `serve_remote` answers every path outside `/api/v1/` with 404 (`src/app.rs:3023-3026`). There is no static serving, no HTML, and no non-JSON reply anywhere in the port.
- The adapter cannot yet express what a browser needs. `respond()` hard-codes `Content-Type: application/json` and takes no headers (`src/adapters/remote.rs:530-553`). `RemoteReply` has no content type or header list (`:389-393`). `read_request` keeps only `Authorization`, `Upgrade` and `Sec-WebSocket-Key`; it drops `Cookie`, `Origin`, `Host` and `User-Agent` (`:444-511`). `respond_upgrade` writes no `Sec-WebSocket-Protocol` (`:519-527`).
- The token is bearer-only today, which is why CSRF does not arise. A cookie-carried token makes every `POST` under `control` (send, resume, trust, draft, dictation, ticket decide/resume/close, new session) ambient to any browser that holds it. That changes the threat model and the plan has to answer it (§4 below).
- `POST /api/v1/pair` returns the token in its JSON body (`src/app.rs`, `remote_pair`, reply built near line 3650). A browser that wants the token out of script reach must get a different reply shape (a `Set-Cookie` with no token in the body). The issue's sentence "the page's script never sees" depends on that change.
- Two claims in the issue's "What exists" are accurate and two are not quite (§6 below). The QR payload has no test anywhere in the tree.

## 1. The on-disk record and what rehydrates it, before the pane

Nothing a browser does changes any session, project or working-set record. The new state is the device row and a page that holds no state of its own.

**On disk**

- `identity.json` (in the data directory; `ports::remote::Identity`, `src/ports/remote.rs:26-40`): `server_id`, `server_name`, the DER certificate (base64), `spki_sha256`, `created`, and `listen {lan, port}` (`:57-61`). It is written through the store's atomic path (`src/adapters/remote.rs:43-86`). Its private key is in the Keychain under `remote-key:<server_id>`.
- `devices.json`: a bare JSON array of `PairedDevice` (`src/ports/remote.rs:96-107`: `id`, `name`, `scope`, `token_sha256`, `created`, `last_seen` with `#[serde(default)]`). It has no version field and is loaded as `Vec<PairedDevice>` (`src/adapters/remote.rs:80-85`). An additive field with a serde default is therefore safe, and the rule that `model.rs` types need a `SCHEMA_VERSION` bump does not cover this type. (Confirm the rule's reach: [tree] below.)
- The pairing window is **not persisted**. `AppAction::OpenPairing` sets `self.pairing` and emits no effect (`src/core/remote.rs:46-54`). The code, scope and `failures` live in memory. A restart clears the window, the QR and the code.

**Rehydration**

- Startup loads identity and devices (`src/app.rs` remote section near 2783–2840; `AppAction::IdentityLoaded` in `src/core/remote.rs:23-31`). Devices resume with their token hashes, so a phone whose cookie holds a token keeps working across a host restart. `epoch` changes on every launch, so the page must refetch everything after `hello` (spec §4.4).
- A browser device row is created by `/pair` and added through `AppAction::Paired` (`src/core/remote.rs:65-71`, `new_device` at `:163-175`). Its name is `name.trim().chars().take(64)`, taken from the request's `device_name` field, which the client supplies. `remote_pair` defaults it to `"Device"` when empty (`src/app.rs`, `remote_pair`).
- Each authenticated request calls `DeviceSeen` (`src/app.rs:3150`). It saves `devices.json` only when `last_seen` moved by 300 s (`src/core/remote.rs:15-18, 72-85`).
- Revocation deletes the row and saves (`src/core/remote.rs:86-92`). The next request's `device_by_token` misses, and `authenticate` answers 401 `unauthorized` with "the token is not known; pair again" (`src/app.rs:3127-3150`). A revoked device's socket closes (`tests/remote.rs:2892`, `revoke_closes_the_devices_socket`).
- The page itself has no record. Its files are embedded at compile time (the issue's plan; `web/` does not exist yet, `ls` confirms). Nothing rehydrates it; it is served again on every load.
- A send is a command through the control port's `serve` and its `operations.log` under the page's op id (`src/app.rs`, `remote_command`, around line 3170ff). Idempotency is the op id, so a page that reloads mid-send has to keep the op somewhere to ask `/ops/{op}` (spec §4.5). Where the page keeps it is an open question.

**The pane (what the page shows, and where each part comes from)**

- Sessions grouped by project: `GET /api/v1/projects` then `GET /api/v1/projects/{id}/sessions` (both routed in `serve_remote`, `src/app.rs:3040-3060`). Each `SessionView` (`control/src/lib.rs:637-690`) carries `waiting: bool` (`:657`), `waiting_reason` (`:658`), `kind`, `name`, `liveness`, `card: String` (`:646`), `trust_question` (`:662`), `resume_id`, `last_stop_at_ms` (`:650`), `last_activity_ms` (`:656`).
- "Waiting ones first and marked": from `waiting` and `waiting_reason`; or `GET /api/v1/waiting`, which is served (`src/app.rs`, `("GET", ["waiting"])`).
- "The one-line excerpt the card shows": the wire view has `card` (`control/src/lib.rs:646`) and no `last_message` field. The spec (`docs/remote-client.md` §4.3, the paragraph on `session.last_response`) names the excerpt `last_message` from the Stop hook. The name and its source do not match this tree. See [tree] in §8.
- Last response or screen: `GET /api/v1/sessions/{id}/last-response` and `/screen?lines=n` are routed to `remote_content_route` (`src/app.rs:3101-3104` and `:1947-2024`; the content kinds and their `ContentView::LastResponse` mapping are at `:1967`). The conversation page `GET /api/v1/sessions/{id}/conversation` is routed the same way.
- Live refresh: `GET /api/v1/events` (WebSocket). The subscription sends `hello`, then `changed`/`removed`. The page applies replies by `revision` (spec §4.2–§4.4). The host side exists (`src/app.rs`, `remote_subscribe` at 2723; the diff pass at 1584–1620 and `observe` at about 1640).
- Reply box: `POST /api/v1/sessions/{id}/send` is served (`src/app.rs:3066-3068`) and refuses a non-control device with 403 `scope` (see `remote_command`, which checks `device.scope != Scope::Control`, `src/app.rs:3166`). The page would hide the box from the scope, but nothing returns the caller's scope to the caller (§4 F-scope below).
- Redaction: every text the page renders goes through the same `redact`/`remote_reply` path as the headset (`src/app.rs`, `remote_reply` at 3695), so the page inherits the existing masking and its bounds (`STRING_CAP`, `NOTES_CAP_ALONE`). Nothing the page adds needs its own policy, as long as the page renders only these bodies.

## 2. Code map (as in this worktree)

Adapter, `src/adapters/remote.rs`:
- `RemoteRequest` 360–373 (`method`, `path`, `query`, `body`, `bearer`, `upgrade`). No cookie, origin, host or user-agent field.
- `RemoteReply` 389–393 (`status`, `body`, `authenticated`). No content type, no headers.
- `read_request` 444–511. Bearer 487–489; upgrade 491–493; path/query split 495–.
- `respond_upgrade` 519–527 (101 with `Upgrade`, `Connection`, `Sec-WebSocket-Accept` only).
- `respond` 530–553 (always `application/json`, `Connection: keep-alive`, one write).
- `MAX_UNAUTHENTICATED = 4`, `MAX_PAIRED = 12` 567–568; `HANDSHAKE_TIMEOUT` 5 s 570; `IDLE_TIMEOUT` 30 s 573; `PAIR_PER_MINUTE = 6`, `INFO_PER_MINUTE = 60` 583–584; `PAIR_PATH`, `INFO_PATH` 586–587.
- `Limits::limited` 631–640: only `/pair` and `/info` have windows; every other path returns "not limited" (so a static path is never rate-limited, only pooled).
- Accept loop: a connection past the unauthenticated pool is dropped at accept (`drop(stream)`, line 704). The pool is incremented per accepted connection and only moved to "paired" on the first authenticated reply (`connection`, 770–880; the move is at about 822–834).
- `ANSWER_TIMEOUT` 60 s 577; `DRAIN_PER_PASS = 8` 580.
- `make_identity` 95–129: `CertificateParams::new(Vec::<String>::new())`, so **no subject alternative names**. Validity is rcgen's default, which the doc comment (90–93) gives as "the year 4096". CN is `Switchboard <uuid>`.
- `tls_config` 144–158: `ring`, TLS 1.2 and 1.3, no client auth, no ALPN.
- `RemotePort::bind` 667–: `0.0.0.0` when `listen.lan`, otherwise `127.0.0.1` (the choice is in `src/app.rs`, `bind_remote`, about 2966–2972).
- Pinning helpers `pinned_client`/`Pin` 168–245 (for Rust clients and tests only; a browser has no such hook).
- `local_address()` 962–970: the routing address, which is returned even when `lan` is off.

Ports, `src/ports/remote.rs`: `Identity` 26–40, `Listen` 57–69 (default port 43187), `Scope` 76–88, `PairedDevice` 96–107.

App, `src/app.rs`:
- Drain: `serve_pending` pulls up to 8 remote requests per pass (1584–1596), then frames (1598–1606), then the diff/observe path.
- `serve_remote` 3017–3125: the `/api/v1/` strip at 3023 (non-matching → 404 `not_found`), `("GET", ["info"])` and `("POST", ["pair"])` answered before authentication (3028–3029), `authenticate` at 3031, `events` upgrade at 3035–3037, the routing match 3040–3110, `hide_workers` on every listing unless `scope=all`.
- `authenticate` 3127–3152: a missing bearer and an unknown bearer both answer 401 `unauthorized`, with two different messages (3132, 3145). No record of revoked tokens exists to tell them apart.
- `remote_command` 3159ff: checks `device.scope != Scope::Control` (403 `scope`, at 3166), then the op log and `serve`.
- `remote_info` 3568–3603: `capabilities` at 3580 (`read`, `control`, `draft`, `scope`, `tickets`, `evidence`, `create`); `pairing_open` 3600. There is no `web` capability and no device-identity endpoint.
- `remote_pair` 3606–3666: the body must be JSON (3615); the key check comes **before** the code check (`spki_sha256` mismatch → `PairingFailed` → 403 `wrong_key`, 3636–3645); a wrong code → `PairingFailed` → 403 `wrong_code` (3646–3650); `device_name` defaults to `"Device"`; the reply JSON is `{token, scope, server_id, device_id}` (3659–3665), so **the token is in the body**.
- `refuse` 3754–3772 logs `method` and `request.path` (no query string) and the code.
- `remote_info`'s `limits` (3585–3599) is what the page would read for its own bounds.
- Bind: `bind_remote` 2936–2990.

UI, `src/ui/remote.rs`:
- Dialog text: the redaction sentence at 71–72 is the only one about what a device sees; nothing about certificate trust.
- Pairing window: open with a scope at 213–226 (`OpenPairing` with `pairing_code()` and `draft.scope`).
- QR payload built inline at 160–166: `{server_id, spki_sha256, code, addresses}`, with `addresses` holding one entry, `local_address()` plus the port (157–159). The text form is "Copy invite" at 199–206, which copies the same JSON.
- The address line is shown at 188–190 whether or not the listener is on the LAN.
- `qr_texture` 280–299 (the `qrcode` crate, 4 px per module, 4-module quiet zone).
- The payload construction is **logic inside `ui/`**. CLAUDE.md says logic in `ui/` moves to the core, and there is no test for it (grep of `tests/remote.rs` for `qr`, `invite` and `addresses` finds nothing).

Core, `src/core/remote.rs`:
- `OpenPairing` 46–54 (code uppercased; `until = now.mono + PAIRING_WINDOW`), `ClosePairing` 55, `PairingFailed` 56–64 (three closes), `Paired` 65–71 (window closes on first success), `DeviceSeen` 72–85, `RevokeDevice` 86–92, `SetDeviceScope` 93–100.
- `pairing_at` 149–154; `device_by_token` 143–147 (matches on the token hash); `new_device` 163–175.

Swift, `client/`:
- `client/Sources/SwitchboardProtocol/Info.swift`: `ServerInfo` 4–39 (`capabilities`, `pairing_open`, `version`); `PairRequest` 47–64; `PairReply` 66–77 (token, scope, server_id, device_id); `KeyFingerprint.short` 82–89; **`PairingInvite` 94–117, which decodes JSON only** (`parse` at 114–116 uses `Wire.decode`).
- `client/Sources/switchboard-remote/main.swift`: `pair` with one argument parses the invite JSON (107–111); with three arguments (`host:port code spki`) it probes `/info` unpinned and builds an invite (112–124); usage text at 10–18.
- `client/Sources/SwitchboardUI/BrowserView.swift:103–107` (dev aid, `SWITCHBOARD_INVITE` split by newline and parsed) and `:849`/`:883` (the headset's paste field "Paste the QR code's text" and `PairingInvite.parse`).
- `client/Apps/SwitchboardVision/SwitchboardVisionApp.swift:56`: the environment variable's doc comment says the value is invites' JSON.
- `client/Sources/SwitchboardClientCore/ConnectionManager.swift:474–487`: builds an invite and pairs with `addresses.compactMap(Address.init).first`.

Tests, `tests/remote.rs`:
- `http` 245–297: a raw TLS client that reads the status line and `Content-Length` only, and returns the body as a `String`. It does not return headers.
- `call` 299–325 parses the body as JSON (`serde_json::from_str`, line 322), so a static HTML body would panic it. A new helper is needed for the page.
- `get` 327–330.
- `info_is_served_to_anyone_and_every_other_read_is_refused_unpaired` 426–475, which asserts `GET /other` → 404 `not_found` (about line 455). This still holds if `/` alone is added.
- `the_allowlist_refuses_everything_else_with_no_side_effect` 682–724.
- `a_wrong_pin_fails_at_the_handshake_before_any_request` 1395–1404.
- `pair_is_rate_limited_per_listener` 1406–1420: asserts six 403 then 429, and `/info` still 200.
- `revoke_closes_the_devices_socket` 2892.
- `stalled_strangers_fill_their_own_pool_and_never_a_paired_clients` 3226 (the pool test).
- Fixtures 1579–1700 (`write_fixtures`), decode check 1829–.
- No test of the QR payload.

Docs:
- `docs/remote-client.md`: §4.1 bounds 124–131 (the 4-unauthenticated pool at 125; `/pair` 6/min and `/info` 60/min at 126); §4.2 caps 147–153; §4.3 endpoint table 157–192; line 194 "v1 remote is the table above and nothing more"; §4.4 events 221–268; §5 pairing 302–319 (306 "Require pairing before anything but /info and /pair"; 309 QR and key-before-code; 312 "never in a URL"; 313 `devices.json`; 315 key rotation; 316 revocation); §11 table near 480 (revocation and rotation behaviour).
- `docs/design.md`: trust boundary 219–266 (251 "as redacted text on the remote port"); "Remote client (2026-10-12)" 3107–3170 (3157 "Remote devices dialog"); remote status continues to about 3260; Open questions 3672–3678.
- `README.md` (Layout table, dev aids, data directory) and `CHANGELOG.md` (Unreleased) were not opened; CLAUDE.md requires both to change with a user-visible feature.

## 3. Findings the plan has to reckon with

F1. **No static serving, no headers, no content type in the adapter.** `respond()` (`src/adapters/remote.rs:530-553`) and `RemoteReply` (`:389-393`) have no way to say `text/html`, a `Content-Security-Policy`, `Cache-Control`, or `Set-Cookie`. Every one of the issue's static-path requirements needs an adapter change. The change is small, but it touches the one function every reply goes through.

F2. **`read_request` drops every header the browser plan needs.** `Cookie` (cookie option), `Origin` (CSRF on POST and the WebSocket upgrade), `Host` (rebinding), `User-Agent` (device name), `Sec-WebSocket-Protocol` (subprotocol option). Only `authorization`, `upgrade`, `sec-websocket-key`, `content-length` are read (`:486-493`). Adding them is additive to `RemoteRequest`, which is `pub` and built only in `read_request` and tests (`tests/remote.rs` builds requests through the real socket, so no literal construction needs changing).

F3. **`respond_upgrade` omits `Sec-WebSocket-Protocol`** (`src/adapters/remote.rs:519-527`). If the page offers a subprotocol carrying the token, the handshake must echo one it accepts. Whether a browser fails the handshake without the echo is a [source] question (§7); the tree sends none.

F4. **The token is in the `/pair` reply body** (`src/app.rs`, `remote_pair`, the reply object `{token, scope, server_id, device_id}`, `"token": token` at line 3661). Any `fetch` in the page can read that body. The issue's "the page's script never sees [the token]" needs a browser-specific reply that sets the cookie and omits the token, or the claim does not hold for the pairing response itself.

F5. **Cookie-carried auth makes `control` POSTs ambient.** Today the only credential is an `Authorization` header, which a browser never attaches on its own, so a cross-site page cannot send a command. A cookie is attached automatically. `SameSite=Strict` is the browser's answer for cross-site requests (a [source] question about IP-address origins; §7), and an `Origin` check in the adapter or `serve_remote` is the second layer. Neither exists. The `control` endpoints the plan would expose include `send`, `resume`, `trust`, `draft` (including `send`), `dictation`, `POST /projects/{id}/sessions` (a new session with a prompt), and ticket `decide`/`resume`/`close`.

F6. **Unauthenticated pool and browser fan-out.** The pool is 4 connections (`src/adapters/remote.rs:567`). A connection counts as unauthenticated until its first authenticated reply, and is closed after 30 s idle (`:573`, and the first-request timeout at 5 s). A page needs at least `/` and a script, plus a favicon request (which the browser makes on its own), and a browser may open several connections in parallel. A fifth connection is dropped at accept (`:704`). The spec's CLI-shaped pool was sized for one client, not a browser's parallel loads. Whether the page as specified (inline script forbidden by CSP, so at least one separate script file) fits in four is a [run] question (§7).

F7. **A static path is pooled but never rate-limited.** `Limits::limited` answers "not limited" for any path other than `/pair` and `/info` (`src/adapters/remote.rs:631-640`). The page's own requests count only against the pool. That is the answer the issue asks for under item 4, but it means a stranger's unauthenticated `GET /` costs a pool slot and a frame pass (if served on the frame) with no rate window. Whether to serve static files on the connection thread or the frame is a choice the plan has to make. Both are possible: the page's bytes are constants, and `RemoteIncoming` already carries the channel.

F8. **Revocation and "never paired" look the same to the host.** `authenticate` returns 401 `unauthorized` with two different messages for a missing token and an unknown token (`src/app.rs:3127-3150`). Revoked tokens are deleted, not tombstoned (`src/core/remote.rs:86-92`). So the host cannot say "revoked" as opposed to "not paired". The issue's acceptance ("the page says so on its next request") needs either a tombstone (new state, retained hashes) or a page that says "not paired or revoked" for any 401. This is a plan decision, not a settled fact.

F9. **The page needs its own scope and nothing returns it.** No endpoint returns the caller's scope; `authenticate` returns the `PairedDevice` but nothing serialises it. `/info` has the capability list, not the caller's scope. The reply box must be hidden from scope, so the page either gets a new read endpoint (§4.3 says "v1 remote is the table above and nothing more", `docs/remote-client.md:194`, so this amends the spec) or learns scope by attempting a send.

F10. **The QR's address and the listener disagree when LAN is off.** With `listen.lan` false the listener is `127.0.0.1` (`src/app.rs`, `bind_remote`), but the pairing window still shows `local_address()` (`src/ui/remote.rs:157-159`), which is the LAN address. A phone scanning it reaches nothing. The URL form would carry the same address. The existing behaviour is not this issue's, but the URL form makes it more visible.

F11. **One address in the QR.** `addresses` holds one entry (`src/ui/remote.rs:164`). The URL form carries one host, so a phone on a different interface has no fallback. The spec's `addresses` array (§5, line 309) is already a single item in practice.

F12. **The certificate has no SAN.** `CertificateParams::new(Vec::new())` (`src/adapters/remote.rs:95-110`) and rcgen's default 4096 validity mean every browser sees a name mismatch as well as a self-signed chain, on the URL's IP host. The click-through is therefore the only trust step, as the issue says, and the exception has to hold for `wss://` on the same origin (a [source] question).

F13. **The QR payload is logic under `ui/` with no test.** Building `{server_id, spki_sha256, code, addresses}` happens inside `pairing()` (`src/ui/remote.rs:160-166`). CLAUDE.md says logic in `ui/` moves to the core. The URL form would be a second such builder unless it is a pure function (in `adapters/remote.rs` or `core/remote.rs`) that `tests/remote.rs` can call. The text in the visionOS paste and the CLI both depend on it.

F14. **The Swift invite is JSON-only.** `PairingInvite.parse` (`client/Sources/SwitchboardProtocol/Info.swift:114-116`) and its three callers (`BrowserView.swift:106` and `:883`, `main.swift:111`, the vision app's dev aid at `SwitchboardVisionApp.swift:56`) all assume JSON. A URL form needs a second parser in Swift (`URLComponents`), with tests in `client/Tests`. The issue says the native text path "has to keep working", which is a Swift change plus a test, not only a Rust one.

F15. **The `wrong_key` path counts against the window.** A `/pair` whose `spki_sha256` is not the host's own is a `PairingFailed` (`src/app.rs:3636-3639`), the same as a wrong code. A browser that compares the URL key with `/info` before posting (as the issue requires) avoids sending a wrong key. A stale page open from a previous window would still count.

F16. **Device name.** The page can supply `device_name` from `navigator.userAgent` (JavaScript), or the host can read `User-Agent` (F2). Either way the value is client-supplied and capped at 64 characters (`src/core/remote.rs:169`). "Safari on iPhone" is what the issue asks for; whether the UA string can tell an iPad from a Mac in desktop-site mode is a [source] question.

F17. **`devices.json` has no version.** An added `kind` or `user_agent` field on `PairedDevice` is additive with a serde default (`src/ports/remote.rs:96-107`). The `SCHEMA_VERSION` rule in CLAUDE.md is stated for `model.rs` types; whether `ports/remote.rs` types are covered is a [tree] question (§8).

F18. **The `/info` rate window is shared.** `/info` is limited to 60 per minute per listener (`src/adapters/remote.rs:584`, `:623-640`). Every page load calls `/info` for the key comparison, and so does every other client. A reload loop from one phone could fill it for the others, and then the page's own check fails with 429.

F19. **Identity change in a browser has nothing to compare against.** The browser has no pin. The page can detect a changed key only by comparing `/info`'s `spki_sha256` with a value it stored at pairing time (the URL's `key`). Where the stored value lives (cookie, `localStorage`, or nowhere) is open. A rotated key also changes the certificate, so the browser shows its own warning again (a [source] question).

F20. **Existing tests that the plan will touch.** `tests/remote.rs:426-475` asserts `/other` is 404 and must keep passing. The `http` and `call` helpers (`:245-325`) cannot read a non-JSON body or headers, so the page tests need a raw helper. The fixture set (`write_fixtures`, `:1587-1700`) covers JSON shapes; a view change would need a fixture change and a Swift decode check (`:1829`).

## 4. The six decisions, with what the tree says about each

Not settled here. Each entry lists the facts the plan needs and where the tree is silent.

1. **Certificate trust.** The tree has no certificate download and no UI for one. The paired-devices dialog's text is at `src/ui/remote.rs:71-72` and says nothing about trust. The host's certificate has no SAN (F12). Any option ("click-through as the trust step" or "download and install a profile") needs a sentence in the dialog, which is a UI change under `ui/`. Keeping §5's rule (only `/info`, `/pair`, and the page before pairing) means `serve_remote` needs a path allowlist for the page, which does not exist yet (F1).

2. **Token transport.** Three facts constrain the choice. (a) The `/pair` body carries the token (F4), so the cookie option needs a different reply. (b) `read_request` does not read `Cookie` or `Origin` (F2, F5). (c) `respond_upgrade` does not echo a subprotocol (F3). The bearer header must stay for native clients, and `authenticate` (`src/app.rs:3127`) already reads only the bearer, so a second source is an addition in one function. Where the token lives on the device follows the choice: a cookie is out of script's reach; a subprotocol value or `localStorage` is in it, and the issue's "the page's script never sees" is then false for that option.

3. **One QR or two.** The tree has three readers of the JSON form: the CLI (`main.swift:107-111`), the headset's paste (`BrowserView.swift:883`), and the vision app's dev aid (`SwitchboardVisionApp.swift:56`), plus `ConnectionManager` (`:474-487`), which builds an invite from a Bonjour record and does not parse text. The CLI's three-argument form already exists and does not need the JSON. The URL form needs a Swift parser (F14). The tree has no other consumer of the JSON.

4. **Static paths.** The paths, content types, CSP, caching and pool choices are all open in the tree (F1, F6, F7). What the page shows with no code and no token: `GET /info` gives `pairing_open`, so the page can say "open the Remote devices dialog on the Mac" or "pairing is open, scan the QR". `pairing_open` (`src/app.rs:3600`) is already there.

5. **Where the page lives and how it is tested.** `web/` does not exist. Tests in `tests/remote.rs` would need a raw helper (F20). The pairing flow can be tested end to end with the host's Rust client (the `http` helper plus `pinned_client`, `:168`), as the issue says. The UI test harness (`tests/ui.rs`) covers the dialog only; a browser is not in the tree.

6. **Device record.** `PairedDevice` (`src/ports/remote.rs:96-107`) has no kind field, and the dialog lists only `name`, `scope`, `created`, `last_seen` (`src/ui/remote.rs:230-275`). A "browser" marker or the UA string needs a field and a line in the dialog. Revocation already works from the dialog (F8 is the only gap).

## 5. Things the issue says that the tree confirms

- "TLS with a per-installation self-signed certificate, pairing by a code with a pinned key, bearer tokens, the `/api/v1/*` read and control endpoints, the `/api/v1/events` WebSocket with the revision counter and `changed` events, redaction before every cut." Confirmed in the files above.
- "The pairing window... a QR carrying `{server_id, spki_sha256, code, addresses}`... open for five minutes or three wrong codes." Confirmed: `src/ui/remote.rs:160-166`, `src/core/remote.rs:46-64`, `src/core/action.rs` constants (`PAIRING_WINDOW`, `PAIRING_ATTEMPTS` — not opened, values not checked).
- "Any path outside `/api/v1/` is refused." Confirmed at `src/app.rs:3023-3026`.
- "The headset pairs from the Bonjour record and a typed code; the command-line `switchboard-remote pair` takes the QR's text as one argument." Confirmed: `client/Sources/SwitchboardClientCore/ConnectionManager.swift:474-487` and `main.swift:107-111`.
- "Three speakers of the protocol already: the visionOS app, `switchboard-remote`, and the fixtures shared by `tests/remote.rs` and the Swift tests." Confirmed in the file list; the Swift test targets were not opened.

## 6. Where the issue's "What exists" is not quite right

- The QR's text is not a second format that "the native text path... takes the URL too, or the JSON as before" in any single place. The JSON is parsed in three Swift places and once in Rust tests (not checked). The issue treats them as one.
- "The host's card excerpt" is not a field the tree names. `SessionView` has `card` (`control/src/lib.rs:646`) and no `last_message` (checked by grep over `control/src/lib.rs`). The spec names `last_message` (§4.3). Which one the page shows is for the plan.

## 7. What I did not check

- I did not run `cargo test`, `cargo clippy`, the Swift tests, or any server. I did not run `switchboard-remote`, a browser, or a phone.
- I did not read `docs/remote-client.md` §6, §7, §8, §9, §10 (except by grep), §11 (except by grep) or §12–§13 in full. The truncated read stopped at line 396 of 553; the remainder was checked only for the lines cited.
- I did not read `spikes/22-remote-port/README.md`, the Swift test files, `ConnectionManager.swift` beyond its pairing function, `PAIRING_WINDOW`/`PAIRING_ATTEMPTS` (`src/core/action.rs`), `remote_content_route` (`src/app.rs:1947-2024`), `remote_subscribe`, `hide_workers`, or the redaction functions beyond their call sites.
- I did not read `README.md`, `CHANGELOG.md`, or the `ui/` dialog test in `tests/ui.rs`.
- I did not check the claims about browsers, iOS, certificate exceptions, cookies, CSP, WebSocket subprotocols or history behaviour against any source; those are all questions below.
- I did not check whether the `qrcode` crate accepts a URL-sized payload at the window's error correction level; the payload is short (roughly 130 characters) but this is not measured.

## Open questions

- [source] Does Safari on iPhone and iPad keep a certificate exception for an IP-address host with no SAN and a 4096-year validity, across launches, after a click-through on the page?
- [source] Does a click-through on `https://<ip>:<port>/` also cover `wss://<ip>:<port>/api/v1/events` in Safari and in Chrome on Android, or does the WebSocket raise its own warning?
- [run] On a phone, does the WebSocket handshake to the same host succeed after the page's click-through, without a second warning?
- [source] Does a browser refuse a `101` upgrade that offers `Sec-WebSocket-Protocol` and gets none back (the adapter writes no such header, `src/adapters/remote.rs:519-527`)?
- [source] Does a cookie with `HttpOnly; Secure; SameSite=Strict` set over `https://<ip>:<port>` get stored and sent on `fetch` and on the WebSocket handshake in Safari and Chrome?
- [source] What does `SameSite=Strict` do with an IP-address origin: is a page on another host name on the same network cross-site for the cookie?
- [source] Does `history.replaceState` remove the code from the address bar and from Safari's history entry for the first navigation, and does Safari's history or the camera app keep the original URL?
- [source] Does a `Set-Cookie` with `Max-Age` (not a session cookie) survive an iOS Safari relaunch, and how long does iOS keep it?
- [run] How many concurrent unauthenticated connections does Safari open for the page (HTML, script, stylesheet, favicon, preconnect), and what happens to the fifth at `src/adapters/remote.rs:704` (dropped at accept)?
- [source] Does a browser request `/favicon.ico` by itself, and what does it do with a 404 or a 204?
- [source] Does `connect-src 'self'` in a Content-Security-Policy allow `wss:` to the same host and port in Safari and Chrome?
- [source] Does a browser follow a 302 or a `Location` header to a different path on the same origin in a way that breaks the `?code=` removal? (Needed only if the plan redirects after `replaceState`.)
- [source] What does `navigator.userAgent` report for an iPad in desktop-site mode, and can the page tell it from a Mac?
- [source] Does a browser reject a `Host` header that does not match the certificate's name, and does rebinding from another origin reach the host at all with the certificate exception?
- [tree] Does any code outside `src/adapters/remote.rs` and `src/app.rs` rely on `read_request`'s `RemoteRequest` shape, so that adding fields (cookie, origin, user agent) is not a breaking change? (Grep for `RemoteRequest {` across `src/` and `tests/`.)
- [tree] Is the `SCHEMA_VERSION` rule in CLAUDE.md meant to reach `ports/remote.rs` types such as `PairedDevice`, which are serialised to `devices.json` without a version field (`src/adapters/remote.rs:80-85`)?
- [tree] Where does a browser-visible card excerpt come from today: `SessionView.card` (`control/src/lib.rs:646`), or a `last_message` field that some other view carries? (Grep `last_message` across `control/`, `src/core/` and `src/app.rs`.)
- [tree] Does `remote_content_route` (`src/app.rs:1947-2024`) answer `last-response` and `conversation` with a body the page can render as-is, or does the page need a new model? (Read the function and the wire types.)
- [tree] Does the page's reply box need the caller's scope before it renders, and is there any existing path that returns it (the answer read so far: no, F9)?
- [tree] Should the pairing window refuse to open, or warn, while `listen.lan` is off, given that the QR's address is unreachable then (F10)?
- [tree] Does the spike directory `spikes/22-remote-port` already exercise a browser-shaped request (a second connection, an unauthenticated flood of parallel connections), and would its numbers apply to the pool in F6?
- [tree] Does `tests/ui.rs` have a dialog test that would break if the pairing window's text changes to add a certificate sentence (`src/ui/remote.rs:71-72`)?
- [tree] Which test asserts the pool's behaviour at accept (`stalled_strangers_fill_their_own_pool_and_never_a_paired_clients`, `tests/remote.rs:3226`), and does a static-path change alter its expectations?
- [tree] Is `PAIRING_WINDOW` and `PAIRING_ATTEMPTS` (`src/core/action.rs`) still five minutes and three, as the issue assumes (not opened)?
- [tree] Does the Swift `ServerInfo` decoder (`client/Sources/SwitchboardProtocol/Info.swift:4-39`) tolerate a new `capabilities` value or a new `/info` field, given the issue's plan adds one (a `web` capability, or a device endpoint)?
- [tree] Does the redaction path in `remote_reply` (`src/app.rs`, near line 3700) run over every string the page would render, including the `SessionView.cwd` and `transcript` paths that the view already carries?
- [owner] Choose the certificate trust step: click-through as the trust step, said in the paired-devices dialog, or a downloadable certificate the owner installs on the device first. (Needs a device, and a call on how much friction is acceptable.)
- [owner] Is a cookie-carried token that persists on the phone with no expiry acceptable, given that a cookie reaches every `control` endpoint and the token never expires (`docs/remote-client.md:312`)?
- [owner] Should `POST /pair` answer a browser with a cookie and no token in the body, which changes the reply shape for browsers only (F4)?
- [owner] One URL or two: is the JSON form dropped entirely, or kept for the CLI's three-argument form only (§4.3 of the issue's preference)?
- [owner] Should the revoked and never-paired states be told apart on the wire (a tombstone for revoked token hashes, F8), or does the page say "not paired or revoked" for any 401?
- [owner] Where should the page's stored key for identity-change detection live (cookie, `localStorage` or nowhere), given that a stored key is readable by any script on the origin (F19)?
- [owner] Is a new read endpoint for the caller's own scope and name (F9) an acceptable amendment to §4.3's "nothing more" rule?
- [owner] Should the page serve from the frame (as the spec's "every request is answered on the frame" suggests) or from the connection thread, given that the page's bytes are constant (F7)?
- [owner] Is a browser-driven test (a headless browser in CI, or a manual run on a device) in scope, given that the issue says no UI test drives a browser and that the page may not add a dependency?
- [owner] Which device, account and signed bundle are used for the acceptance run: a phone on the same network, a real Safari, and a bundle signed with the stable identity (CLAUDE.md, "Platform-specific code")?
- [owner] Should the change ship with a CHANGELOG line under Unreleased, a design.md status section, and a README Layout row for `web/` (CLAUDE.md, "Adding a feature" steps 4–5)?
- [owner] Is the page's name shown as "Safari on iPhone" from the UA, or a name the owner types at pairing (the dialog has no field for it today)?
