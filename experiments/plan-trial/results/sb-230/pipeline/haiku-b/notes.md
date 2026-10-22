# Issue #230 — a web control surface on the remote port: investigation notes

Worktree: `/Users/sbgragent/.dispatch/worktrees/434ea5d3` (branch `dispatch/230-a-web-control-surface-the-remote-port-se`).
Paths below are repo-relative to that worktree. Line numbers are as they are in this worktree.
No code was changed and no tests were run. This is reading only.

Read: `CLAUDE.md` (given), `docs/remote-client.md` §4.1, §4.2, §4.3, §4.4, §5, `docs/design.md` (remote status §"Remote client (2026-10-12)", line 3107, and trust boundary line 219), `docs/lessons.md` (general; nothing remote-specific), and the code listed in each section.

---

## 0. Bottom line

The page can be built on the existing port with small additive changes. Three things in the issue's plan do not survive the code as it stands, and should be fixed in the plan before any build:

1. **`?code=` in the URL puts the code in the first HTTP request.** The browser sends the request line (`GET /?code=…&key=…`) to whatever answers at that address before any page JavaScript runs. The page's key check comes after the code has already left the phone. Use a URL **fragment** (`#code=…`), which browsers never send to the server. See §3.
2. **`/pair` returns the bearer token in the JSON body** (`src/app.rs:3661`, `"token": token`). A `HttpOnly` cookie only keeps the token from the page's script if the browser's pairing reply omits the body token. The response writer cannot set cookies today: `RemoteReply` has no headers (`src/adapters/remote.rs:389`) and `respond` hardcodes `Content-Type: application/json` with no other headers (`:530`, `:545`). See §2.
3. **The certificate has no SubjectAltName and rcgen's default validity.** `make_identity` builds `CertificateParams::new(Vec::<String>::new())` (`src/adapters/remote.rs:102`), so it names no host or IP. The native pin ignores names and dates (`pinned_client`, `:168`), so this never mattered. A browser checks both before offering the click-through. Verify on iOS Safari and Chrome before the trust decision is written down. See §1.

Also needed, not in the plan: a WebSocket subprotocol would need the server to echo it (`respond_upgrade`, `:519`, writes no such header); nothing checks `Origin` today, and a cookie brings cross-site exposure that bearer headers do not have (§2).

---

## 1. Browser trust in the self-signed certificate (decision 1)

**What the code does now.**
- The certificate is made once with rcgen, no SAN, rcgen's defaults for validity (`src/adapters/remote.rs:95-127`; the doc comment at `:89-92` says rcgen's default `not_after` is year 4096).
- The native client pins the SPKI hash (`pinned_client`, `:168`; verifier `Pin`, `:183-245`). It ignores names, dates and chains.
- `/info` returns `spki_sha256` to anyone (`src/app.rs:3568`, the body at `:3572-3595`).
- `serve_remote` answers `GET /info` and `POST /pair` before authentication (`src/app.rs:3028-3029`).

**What a browser does.** It checks the name against the SAN, checks the validity period, and checks the chain. A self-signed leaf with no SAN fails the name check, and the warning shows regardless. Whether the click-through is offered for an over-long validity is not verified here. This is the main thing to measure in a spike.

**Options.**
- (a) **The click-through is the trust step.** The owner clicks through once on each device, and the dialog says so in a sentence. This is what I recommend. Reasons:
  - The owner is physically present at the scan. The QR is on the Mac screen and the phone is in the owner's hand.
  - A host-served certificate download (option b) arrives over the same first connection, so an impostor on the LAN can serve its own certificate at that moment. The download is no stronger than the click-through.
  - The download adds a profile-install step on iOS (a trust toggle is needed; verify), and it still needs a SAN and a valid lifetime.
- (b) **Download the certificate first.** Only helps if the owner compares a fingerprint on both screens. Otherwise it is the same trust as (a) with more steps.

**What the URL key buys, and what it does not.**
- It buys this: a QR from host A never sends its code to a host at the same address that reports a different key. A stale QR, a QR from another machine, or a machine that answers at the address after a DHCP change all fail the check before a code is posted. The check is the page's own compare of the URL key with `/info`'s `spki_sha256`.
- It does not buy: protection from an impostor who controls the first connection. The impostor serves the page, so it runs the check, and it can run the check honestly against its own `/info`. Its own key does not match the QR's, so the honest impostor refuses. A dishonest impostor just skips the check and posts the code to itself. The key check is not a trust step.
- The plan's sentence "a page served by an impostor is not a genuine page" is correct, and it should say the same thing about the check.

**The rule in §5 and the plan's wording.** §5 says "Require pairing **before** anything but `/info` and `/pair`". The amendment is: before pairing, a LAN bind serves `/info`, `/pair`, and the page and its assets. The assets carry no secret and no session data.

**Verify before writing this down (not measured here).**
- Whether Chrome and Safari keep the click-through per host:port, and for how long. My understanding is that some browsers keep it only for the session; I did not check this.
- Whether `Secure` cookies are set and sent on an https origin reached through a click-through.
- Whether the validity-length check blocks the click-through on iOS.
- Whether `connect-src 'self'` allows `wss://` to the same host:port in Safari.

**Certificate change needed regardless of the choice above.** Add SANs. The stable one is the Bonjour name (`<LocalHostName>.local`). An IP SAN goes stale on DHCP change. A SAN change re-issues the certificate over the **same key**, so the SPKI hash, and so every existing pairing, stays valid (the spec §5 already says a reissued certificate verifies). This needs the key from the Keychain at re-issue time, which `make_identity` does not take today (`:95-100`). A shorter validity is also a change to the §5 phrase "never expires in practice".

---

## 2. How the token reaches the WebSocket (decision 2)

**Recommended: a cookie.** `Set-Cookie: sb_session=<token>; Secure; HttpOnly; SameSite=Strict; Path=/; Max-Age=<long>`. `fetch` and the WebSocket send it automatically. The page's script never sees it.

What the code needs:
- **Server-side cookie parse.** `read_request` (`src/adapters/remote.rs:444`) keeps only `Authorization` and the upgrade headers (`:487-494`). Add a `cookie` field to `RemoteRequest` (`:360`).
- **Authenticate by cookie.** `authenticate` (`src/app.rs:3127`) reads only `request.bearer`. A cookie fallback is a small change. The native clients keep the bearer path unchanged.
- **Response headers.** `RemoteReply` (`:389`) has no headers, and `respond` (`:530`) hardcodes the content type. Add an optional header list, additively.
- **The pairing reply must not carry the token for a browser.** `remote_pair` builds the body with `"token": token` (`src/app.rs:3661`). The browser variant needs a request-side flag (a body field such as `client: "web"`, or a separate `POST /api/v1/pair/web`). It sets the cookie and returns no token in the body. The native path stays as it is.
- **Expire-on-revoke.** A 401 from `authenticate` should also clear the cookie (`Max-Age=0`).

**Cookie risks the plan must own.**
- **CSRF and cross-site WebSocket hijack.** A bearer header is never attached by a browser to a cross-site request, so the current design has no CSRF surface. A cookie has one. `SameSite=Strict` helps, but it does not isolate ports: a cookie is not port-scoped, and "site" ignores ports, so another local app on the same host can receive it. Add an `Origin` check on every `POST /api/v1/*` and on the WebSocket upgrade. Nothing checks `Origin` today (`read_request` does not read it). Verify the exact site rules in Safari.
- **HttpOnly limits theft, not use.** An injected script cannot read the token, but it can still call `/api/v1/sessions/{id}/send` with the cookie. The real defences are: never render session text as HTML (agent output can echo web content, so treat all transcript and screen text as untrusted), and a CSP with `connect-src 'self'` so injected code cannot send the data elsewhere.
- **Persistence.** The token does not expire (§5), so the cookie needs a long `Max-Age`, or the phone is unpaired every time Safari closes. The `Max-Age` is a browser setting and needs checking.

**The `Sec-WebSocket-Protocol` alternative is worse here.**
- The page's JavaScript must set the value, so the token lives in script memory. That is exactly what the cookie avoids.
- `respond_upgrade` (`src/adapters/remote.rs:519-528`) writes `Upgrade`, `Connection` and `Sec-WebSocket-Accept` only. A browser that offers a subprotocol fails the connection unless the server echoes one. So this path also needs the echo.

**`localStorage`.** Do not put the token there. Any script on the origin reads it. Storing the `server_id` and the SPKI hash there (not secret) is fine, and it is how the page can say "identity changed, pair again" (§6).

---

## 3. One QR or two, and the URL (decision 3)

**Fragment, not query.** Use `https://<address>:<port>/#code=…&key=…&id=…`.
- A query string is part of the HTTP request line, so the code reaches the server (and any box between it and the server, and anything that logs request lines) before the page script can clean the address bar. `read_request` (`src/adapters/remote.rs:444`) parses the query on every request.
- A fragment is never sent to the server. `history.replaceState` then removes it as the plan says.
- The `Referer` leak: even with a query, a same-origin subresource fetched while the URL still holds `?code=` carries the full URL in `Referer` under the default referrer policy. The fragment avoids that. `Referrer-Policy: no-referrer` on the static responses is the belt to go with it.
- The `refuse` log prints only method and path (`src/app.rs:3754-3766`), so the server's own log does not leak the code today. Keep it that way.

**Where the QR text is built.** Inside `draw` (`src/ui/remote.rs:160-165`, the `serde_json::json!` payload). That cannot be unit-tested headless. Move it to a pure function (in `src/adapters/remote.rs` or `src/ports/remote.rs`), so a test can check the URL and the parsing side by side.

**The JSON form.** Who still uses it:
- `switchboard-remote pair '<invite json>'` (`client/Sources/switchboard-remote/main.swift:12` the usage text, `:111` `PairingInvite.parse(rest[0])`).
- The "Copy invite" button (`src/ui/remote.rs:201-207`) copies the JSON.
- `testInviteParsesTheQRText` (`client/Tests/SwitchboardProtocolTests/FixturesTests.swift:157`) decodes an inline JSON string.

The headset does not use the QR at all. It pairs from Bonjour and a typed code (spec §5). So the QR can become the URL. I recommend `PairingInvite.parse` (`client/Sources/SwitchboardProtocol/Info.swift:114`) accept the URL and keep the JSON branch, since it is cheap. The URL form needs `id` and `addresses` derived from `host:port`. The Swift side reads only the fragment, so the test should cover a fragment.

**`addresses` is a list today, one entry in practice.** `local_address()` (`src/adapters/remote.rs:962`) returns one IPv4 address, so a single host in the URL loses nothing. Note that an IPv6 host would need brackets.

---

## 4. Static paths and the port's allowlist (decision 4)

**The frame refuses them today.** `serve_remote` (`src/app.rs:3017`) answers every path without the `/api/v1/` prefix with a JSON 404 (`:3023-3025`). So `GET /` currently reaches the frame and comes back as JSON. The static arm goes before that check, or the static bytes are answered on the connection thread (recommended; see below).

**Recommended exact set and headers** (a proposal to check, not measured):
- Paths: `/` (HTML), `/app.js`, `/app.css`. Optionally `/favicon.ico` as a 204, so Safari's automatic icon request does not show as a refusal in the log.
- `Content-Type`: `text/html; charset=utf-8`, `text/css; charset=utf-8`, `text/javascript; charset=utf-8`.
- `Content-Security-Policy`: `default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'`. No inline script. Also no `style=""` attributes in markup; those fall under `style-src`.
- `Referrer-Policy: no-referrer`, `X-Content-Type-Options: nosniff`, `Cache-Control: no-store` (the files are small; `no-store` avoids a stale page across versions).
- `Content-Length` must be set (the test helper depends on it, see §5).
- Anything else keeps the JSON 404 path. Traversal (`/../x`, `//app.js`) matches nothing in the exact table, so it is refused by construction. Say so in the spec.

**Where to serve it.** On the connection thread, the way the 429 path already does (`src/adapters/remote.rs:788-797`). A static reply needs no state, so it should never queue to the frame, and the frame's allowlist (`serve_remote`) stays the one place that decides what the API serves. That keeps §4.1's "handlers are thin" and the frame's share small.

**Pool and rate window: the real constraint.**
- The unauthenticated slot is taken at accept (`src/adapters/remote.rs:701-706`, the `unauthenticated` counter) before anything is read, so the path is not known when the slot is taken. A separate page pool therefore needs the accept code to know the path, which it cannot. The four slots (`MAX_UNAUTHENTICATED`, `:567`) are shared with every stranger, and with any speculative connection a browser opens.
- A page load of `/` plus two or three assets, plus an icon request, is at or near four concurrent connections. I have not measured browser connection counts on iPhone Safari or Chrome. If the page needs more than four at once, the fifth is closed at accept and the page loads without its script or CSS. Measure before choosing: raise the cap, or reduce the page to fewer requests (for example CSS in a hashed `<style>`, which the CSP can allow without `unsafe-inline`).
- `/pair` (6 per minute, `:583`) and `/info` (60 per minute, `:584`) are the only windows. The static paths need none; the slot already bounds concurrency, and the files are cheap. Keep it that way unless the measurements say otherwise.
- After pairing, the page's API connections move to the paired pool (`MAX_PAIRED` 12, `:568`, moved at `:835-845`). A browser keeps several keep-alive connections, plus the WebSocket. That is fine for one or two devices, and tight with more. Note it.

**What the page shows without a code and without a token.** The page calls `/api/v1/info` (unauthenticated, rate-limited to 60 per minute), and shows the host name and whether pairing is open (`pairing_open`, `src/app.rs:3596`). Closed: "Pairing is closed. Open Remote devices on the Mac and scan the code." It calls no other API path. On a 401 from a data path it shows the revoked/not-paired state (§6).

---

## 5. Tests, and the harness they need (decision 5)

**The test client cannot see headers.** `http()` (`tests/remote.rs:245-297`) reads the status line, then only `Content-Length` (`:284-290`), then the body. `call()` (`:299-326`) parses the body as JSON (`:323`). So a `Content-Type`, `Content-Security-Policy` or `Set-Cookie` assertion is impossible until the helper returns headers. A raw-body variant is also needed, since the page is HTML and `call()` would panic on it.

**Tests to add, in `tests/remote.rs`.**
- Static: `GET /` and the assets answer 200, the right `Content-Type`, the CSP, `Referrer-Policy`, `no-store`, and a correct `Content-Length`.
- Refusals: traversal and unknown assets answer 404 `not_found`; `POST /` answers 404 or 405. Extend `the_allowlist_refuses_everything_else_with_no_side_effect` (`:682`) rather than duplicate it.
- The embedded page references only paths the host serves: scan the embedded text for `/api/v1/` strings and `src=`/`href=` values, and check each against the route table. This is a string test, which is what the issue asks for.
- The pairing flow end to end with the host's own client, as the issue asks: `GET /` (no code) → `GET /api/v1/info` → `POST /api/v1/pair` with the browser flag → expect `Set-Cookie` and no `token` in the body → `GET /api/v1/waiting` with the cookie only → a WebSocket subscribe with the cookie only (`subscribe_at`, `:1960`, takes a token; it needs a cookie variant).
- Cookie path through `read_request`: a request with a `Cookie` header reaches `authenticate`.
- `Origin`: a POST and a WebSocket upgrade with a foreign `Origin` are refused.
- Revocation through the cookie: 401 and cleared cookie (see §6).

**The page's JavaScript has no test harness.** The repo has no JS toolchain, and CI runs cargo only. Options: (a) keep the page small, and check its protocol logic (revision reconcile, watch refcounts) with a `node --test` file run locally and not in CI (a new tool, not a dependency of the build); (b) manual phone checks only. I'd take (a) for the reconcile logic and (b) for the rest. This needs a decision, because the issue says "no framework, no build step, no dependency" and (a) adds a developer tool.

**Fixtures.**
- `write_fixtures` (`tests/remote.rs:1587`) and `the_client_fixtures_decode_on_both_sides` (`:1829`) cover reply shapes. Extend them only if a view changes. The page does not add a wire shape, so this is probably none.
- A new capability (for example `web`) is pinned on the Swift side: `FixturesTests.swift:13` asserts the exact list `["read", "control", "draft", "scope", "tickets", "evidence", "create"]`. That changes with `remote_info`'s list (`src/app.rs:3580`). Update both in one commit, or do not add a capability.

---

## 6. The device record (decision 6)

**The record is already the right shape.** `PairedDevice` (`src/ports/remote.rs:96-107`): `id`, `name`, `scope`, `token_sha256`, `created`, `last_seen`. A browser is a row like any other. `devices.json` is a bare array with no schema version (`src/adapters/remote.rs:80-86`, `DEVICES_FILE`), so an additive field with `#[serde(default)]` is safe. A `kind` (`browser` or `app`) is optional; the dialog can work without it.

**The name.**
- The page can build "Safari on iPhone" from `navigator.userAgent`. Caveat: iPadOS Safari reports a Macintosh user agent by default, so an iPad shows as "Safari on Mac". Either accept that, or have the host record the UA string. The host cannot read it today: `read_request` keeps only the bearer and the upgrade headers (`src/adapters/remote.rs:487-494`).
- `remote_pair` takes `device_name` with no length cap (`src/app.rs:3650-3653`). A user-agent string is user-influenced text, and the body cap (64 KiB, `:438`) is much larger than a name needs. Cap it, to the 4 KiB free-text cap the spec already names (§4.2).

**Revocation and "the page says so".**
- Revoke removes the record (`src/ui/remote.rs:267`, `AppAction::RevokeDevice`). Then `authenticate` answers 401 `unauthorized` with "the token is not known; pair again" (`src/app.rs:3127-3145`).
- That is the same answer a never-paired token gets. The host deletes the record, so it cannot say "revoked" without keeping a tombstone. Two choices: keep a tombstone (a small change to `devices.json`), or let the page say "this device is not paired any more: it was revoked on the Mac, or the host forgot it. Pair again." I recommend the second. It meets the acceptance wording without a new record.
- The 401 should also clear the cookie (see §2).

**"Identity changed, pair again".** Key rotation deletes every record (spec §5). The page then sees 401s, the same as revocation. To say "identity changed" the page must remember the `server_id` and SPKI hash it paired with (non-secret, `localStorage` is fine) and compare them with `/info` on each load. Decide whether that is worth the state.

---

## 7. What the page would show, and what is on disk (the required order)

**On disk, and how each piece comes back.**
- **Identity.** `identity.json` (certificate, `spki_sha256`, `server_id`, where the port binds) and the private key in the Keychain under `remote-key:<server id>` (spec §5; `src/ports/remote.rs:26`). At start, `load_identity` reads it (`src/app.rs`, the startup block near line 2783). Unchanged by this issue.
- **Pairing window.** Memory only: `PairingWindow` (`src/core/action.rs:978`): `until`, `code`, `scope`, `failures`. Never saved. A restart closes it. The URL is never stored, and the code is spent by the first successful `/pair`.
- **Device.** `devices.json`, `PairedDevice` (`src/ports/remote.rs:96`). The token itself is on the device, and the host keeps only its SHA-256. On every request the host hashes the cookie (or the bearer) and looks the device up (`src/app.rs:3127`). A restart therefore keeps the phone paired with no code. The page's own state is held in memory per tab.
- **Sessions.** From the core's read models, which are store-backed (`SessionView`, `SetView`, `ProjectView`, `waiting`). The card excerpt is `last_message` from the `Stop` hook on the record. This is rehydrated from the store, not from the page.
- **Last response.** The transcript file at the session's resume handle, read through the content cache (spec §4.3/§4.4). Switchboard does not store the response. It is read again on request.
- **Screen.** The tmux pane, live (`session.screen`). If the pane is gone the reply says so.

**What the page shows, in the order the issue asks.**
- Sessions grouped by project, waiting ones first and marked. Source: `GET projects` and `GET projects/{id}/sessions`, and `GET waiting` for the membership. Each card: kind, state, the excerpt.
- On `changed {session}`, refetch that session. On `changed {waiting}` (only when the membership moves, spec §4.4), refetch `waiting`, and re-sort. This is how a session that starts waiting moves to the top with no reload. The reorder is the page's own, from the session views.
- A session view: for a Claude Code session, the last response (`/last-response`), with `truncated` marked. For anything else, the screen (`/screen?lines=n`, at most 200 lines). "Not running" is shown from the reply. The page holds a watch on `conversation` or `screen` while the view is open (`watch`/`unwatch` frames, spec §4.4), and refetches on `changed`.
- A reply box, enabled only when the device has `control` scope (the header shows the scope). It posts `POST /api/v1/sessions/{id}/send` with a fresh `op`. `crypto.randomUUID()` needs a secure context, and an https page with a bypassed certificate may not count as one. Fall back to `crypto.getRandomValues`.
- Revision handling (§4.2, §4.4): subscribe first, then snapshot; hold a revision per resource key; drop a reply older than the held one. This is the bulk of the page's logic, and it is where a bug would show. The spec's algorithm is the contract. The page must also resend `watch` frames after a reconnect, and handle `resync_required`.

---

## 8. Problems in the plan that need a decision before the spike

1. **The code in the URL's request line** (§3). Fragment it.
2. **The token in the `/pair` body** (§2). The browser variant must omit it.
3. **No SAN, and the validity** (§1). Browser name and lifetime checks decide whether the click-through is offered at all. Measure first.
4. **No `Origin` check, and the cookie's cross-site surface** (§2). Add the check with the cookie.
5. **The WebSocket subprotocol echo** (§2). Only matters if the protocol route is chosen; the cookie route does not need it.
6. **Four unauthenticated slots against a page load's connections** (§4). Measure before choosing the fix.
7. **"Revoked" and "never paired" are the same 401** (§6). Word the page to cover both.
8. **Testing the page's logic** (§5). Pick node-run-locally or manual.

---

## 9. Spike, and the documents to change

**Spike first.** CLAUDE.md requires a spike directory for a new risky mechanism before building on it. The next free number is `spikes/23-…` (the last is `spikes/22-remote-port`). The browser mechanisms that are unmeasured here: click-through persistence per browser; the validity and name checks on the self-signed certificate; `Secure` and `SameSite=Strict` cookies on the click-through origin; the WebSocket with a cookie under `connect-src 'self'` on Safari; the connection count of a page load against the four-slot pool; `Referer` behaviour for the fragment and query variants.

**Documents to change (CLAUDE.md, "Adding a feature").**
- `docs/remote-client.md`: §4.1 (the static arm and the pool/rate decision), §4.3 (the new `GET /`, asset and browser-pairing rows), §5 (the QR as a fragment URL; the cookie; the click-through as the trust step; the amended "before pairing" rule; the identity note), and a new spike entry in §10.
- `docs/design.md`: a "Browser client" status section after the remote client section (line 3107 onward), per milestone.
- `README.md`: the Layout table for any new file (for example `web/index.html`, `web/app.js`, `web/app.css`, and the adapter file if one is added), and the dev-aid list if a `SWITCHBOARD_` variable is added.
- `CHANGELOG.md`: one line under Unreleased for the user-visible change.
- `web/` is included at compile time with `include_str!`. The crate has no `include`/`exclude` list that would drop it (`Cargo.toml`; only `vendor/egui_term` is excluded, `:91`). Nothing platform-specific is needed, so CI on Linux compiles it.

**Pre-commit.** `cargo clippy --locked --workspace --all-targets -- -D warnings` must be clean, and the Linux check from CLAUDE.md applies to any `cfg` arm touched.

---

## 10. What I did not verify

- Any browser behaviour listed in §1, §2, §4 and §9. These are knowledge claims, not measurements.
- Connection counts for a real page load.
- Whether the code of the existing port builds on Linux with the change (not attempted; no code changed).
- No test was run.

