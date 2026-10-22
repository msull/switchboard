# #230: a browser page on the remote port

Checked against the worktree at `7fe8949` (`stage/sb-230-plan-v4-haiku-a`, clean). Every file and line below was read in that tree. Facts are cited by section and item in `inputs/facts.md`: **[S: name]** for a Settled line, **[U n]** for Unresolved item n, **[O X]** for For the owner item X.

## Where this departs from the issue

- **The card excerpt is not on the record.** The issue, and `docs/remote-client.md:207`, call the excerpt `last_message` on the session record. The tree says otherwise. `last_message` is a field of the `Stopped` event (`src/ports/events.rs:24`, built at `src/adapters/hooks.rs:71`), and the core drops it: `src/core/events.rs:132` matches `EventKind::Stopped { pending, .. }`. No record struct carries it, and `SessionView` (`control/src/lib.rs:637`) has no excerpt field. The facts file says it "exists on the session record"; that line is wrong and this plan contradicts it. The plan adds no field. The page fetches `sessions/{id}/last-response` for each Claude session when its `last_stop_at_ms` (`control/src/lib.rs:650`) moves. The headset does the same (`client/Sources/SwitchboardUI/BrowserView.swift:149-163`), so this adds no read model.
- **The device's scope is on no endpoint.** `remote_info` (`src/app.rs:3568`) is unauthenticated and carries no device scope. The scope arrives only in the `/pair` reply (`src/app.rs:3606`) and then goes stale. The plan adds `GET /api/v1/device`.
- **"Revoked" cannot be told from "never paired".** `authenticate` (`src/app.rs:3127`) refuses a missing token and an unknown one with the same code, `unauthorized`, and the host keeps no list of revoked tokens. The page says "revoked, or its sign-in has expired: pair again" and claims no more. A distinct code is an owner follow-up.
- **The page does not keep the token. The browser does, in a cookie.** The issue says the page "keeps the token". Decision 2 keeps it out of page script. This departs from the issue's wording for a security reason.
- **The certificate, as built, cannot be trusted by a browser.** `make_identity` (`src/adapters/remote.rs:95-127`) puts no subjectAltName in the certificate, a CN only, and rcgen's default validity (`:92`, "never expires in practice"). **[S: Apple's TLS server certificate rules]** says such a certificate meets none of Apple's rules. Decision 1 reissues it.
- **The code may already be in history.** `replaceState` removes the code from the address bar and the current entry before anything else runs. The issue's claim that it "never stays in the history" goes further than that. The URL may be recorded in history, the tab list or iCloud tab sync before the script runs **[U 6]**. The plan says so in the dialog.
- **The key in the URL does not stop a relay.** The issue says a genuine page refuses to send the code to a host whose `/info` key differs. That holds. But `docs/remote-client.md:309` says the host "refuses a pairing whose `spki_sha256` is not its own, so a relayed request through a substituted certificate fails even if the code reached it." For a browser that is false. An impostor page is handed the real key and the code in the URL, so it can post both, and the check at `src/app.rs:3636` passes. The spec sentence is amended for browsers.
- **The events socket is not the default.** The issue has the page open `/api/v1/events` directly. **[S: Safari and wss after a click-through]** says a click-through did not carry `wss` in Safari (WebKit 41419, open), and worked only after the certificate was always-trusted. The default is REST polling. The socket is an enhancement.
- **Facts correction, not an issue claim.** Facts *Settled, the tree* says `RemoteReply` is built "at about 15 sites in `src/app.rs`". The grep finds 10 literal sites (`RemoteReply {` without `->`). The plan uses 10.

## Decisions

Each decision names its default and why, and says how the default still works if the open question goes the other way.

### 1. Trust in the self-signed certificate

**Pick:** the click-through is the trust step. The host also serves its public certificate at `/host.cer` as the way out if the warning does not carry the page.

- **Why the click-through.** It is per browser and per address, and it is the only trust step the facts describe as working in Safari, for top-level navigation **[S: Safari and wss]**. A profile install is standing trust in this certificate on the device, and its mechanics are open **[U 3]**. The owner would not want a standing trust as the default step.
- **What it buys:** a device that accepts the warning talks TLS to whatever answers at that address. **What it does not buy:** anything that answers at that address while the window is open can take the code. The window is single-use and closes after five minutes or three wrong codes (`src/core/remote.rs:46-72`). The dialog says: "open this window only where you are."
- **Why `/host.cer` is built now.** If the click-through does not carry `fetch` on current iOS Safari **[U 1]**, the page cannot work on that device at all. `/host.cer` returns the certificate's DER bytes (`Content-Type: application/x-x509-ca-cert`). It holds no key, and it is the same bytes the host already sends in the handshake. The devices dialog shows its address so the owner can install it as a profile if the warning cannot be accepted. This is the one way out that works whichever way **[U 1]** goes. It is a static path, not a credential.
- **Reissue is required, not optional.** **[S: Apple's TLS server certificate rules]** requires a DNS name in subjectAltName, `serverAuth`, and at most 825 days of validity. The reissue keeps the key, so the pin stays **[S: the tree, reissue over the same key]**.

### 2. Where the token lives

**Pick:** on `POST /api/v1/pair` with `"browser": true` in the body, the reply sets `__Host-sb-<server_id>=<token>; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=31536000`, and the body carries no token. The page never holds the token. Native clients keep the bearer header.

- **Why a cookie.** A script cannot read an `HttpOnly` cookie. `localStorage` is readable by any script on the origin. The cookie is the safer store, so it is the default.
- **Why the name carries the server id.** Cookies are scoped by host, not port **[S: Cookies are not isolated by port]**. Two instances on one Mac would otherwise overwrite each other's token.
- **Why `Max-Age` is one year.** The token does not expire; the owner revokes it. A session cookie would make the phone sign in again each time Safari restarts.
- **The mechanics are open, so the first test is the cookie.** **[U 5]** asks whether `Secure` stores on an IP host and whether `SameSite=Strict` reaches the socket. The implementer tests the cookie on a phone first, with the cookie-authenticated `fetch`, which is the default path. If the test fails, the page does not ship with the token in script. The fallback is a token in a `Sec-WebSocket-Protocol` value **[O D]**, and it is a follow-up, not a second transport built now. The socket half of **[U 5]** does not gate the default, because the socket is an enhancement.
- **Origin check (decision for [O origin], answered by default).** A request that carries the cookie and an `Origin` header must have `Origin` equal to `https://<Host>` (case-insensitive). Otherwise it is refused with `403 bad_origin` and counts toward no window. Native clients send no `Origin` and are unaffected. `SameSite=Strict` is the first defence against cross-site requests. The check is the second, and it covers cross-site WebSocket hijacking. A DNS-rebinding page is a different host name, so the browser does not send the cookie to it.

### 3. One QR or two

**Pick:** one URL, in the QR and in the text beside it. The host emits only the URL. `PairingInvite.parse` accepts the URL first and the JSON it accepts today second.

- **Why keep the JSON parse.** Text already copied still pastes. It costs a few lines. Nothing emits JSON.
- **The URL's host.** `local_address()` (`src/adapters/remote.rs:962`) gives an `IpAddr`. An IPv6 address goes in brackets. **[O B]** and **[U 2]** ask whether an IP in the certificate's SAN works in Safari, and whether `.local` resolves **[U 9]**. The default URL host is the IP. The certificate carries both the IP and the `.local` name (§The record), so if the owner's device test refuses the IP, the URL host switches to the `.local` name. That is a one-line change in `pairing_url`, and the certificate needs no reissue.
- **The URL's address on the native side.** **[S: `PairingInvite.addresses` is a list]** means the URL parse fills `addresses` with one `host:port`.

### 4. Static paths on the allowlisted port

**Pick:** four paths, `GET` only, matched exactly. The query is ignored and never logged.

| Path | `Content-Type` | Body |
|---|---|---|
| `/` | `text/html; charset=utf-8` | `web/index.html` |
| `/app.js` | `text/javascript; charset=utf-8` | `web/app.js` |
| `/app.css` | `text/css; charset=utf-8` | `web/app.css` |
| `/host.cer` | `application/x-x509-ca-cert` | the certificate's DER |

- **Headers on every static response:** `Cache-Control: no-store` (no caching across versions), `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer` (so the code is never sent in a `Referer` on the page's own requests), and `Connection: close`.
- **`Content-Security-Policy` on the HTML:** `default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'`. No inline script, no inline style, no external resource. The page's icon is `<link rel="icon" href="data:,">`, so the browser does not request `/favicon.ico`.
- **Where they are served.** On the connection thread, in `connection` (`src/adapters/remote.rs:770`), before the `limits.limited` check. They read no core state, so the frame's share is unchanged. The `/host.cer` bytes are set on `Shared` (built in `bind`, `:667`) next to `tls`.
- **Which pool and window.** The unauthenticated pool, as today (`MAX_UNAUTHENTICATED = 4`, `:567`). `Connection: close` frees the slot as soon as the reply is written, so a page load's three or four requests do not hold slots for the 30 s idle timeout (`:573`). **No rate window.** Static bytes hold nothing secret and reveal nothing `/info` does not, and the pool is the bound on strangers. **[U 8]** asks whether iOS opens more parallel connections than four. The device test answers it. If the test shows drops at accept, a static pool with its own bound is the follow-up. The page is three requests, so the default does not depend on the answer.
- **The spec's rule.** `docs/remote-client.md:306` says "require pairing before anything but `/info` and `/pair`". It is amended to name the four paths. The reason is the public-bytes argument above, not a loosening of the rule.
- **What an unpaired page with no code and no marker shows.** "Open Remote devices on the Mac and scan its code, or open the address that window shows." **[O page without code]** is answered by default with this instruction alone. A typed-code form is not built, because a typed code has no URL key to check against.

### 5. Where the page lives and how it is tested

`web/index.html`, `web/app.js` and `web/app.css` are embedded by `include_str!` in a new `src/adapters/remote_web.rs`, which holds the table in §4. The table is a `const` of `(path, content type, body)`. There is no build step and no package manager. The test for the page's references is in `tests/remote.rs`, and the page's logic is tested by hand (§Tests). The issue rules out a JS dependency, so the page's reconcile logic is not in CI. That gap is stated, not hidden.

### 6. The device record

**Pick:** a browser is a `PairedDevice` like any other. Its name is the page's `device_name`, computed from `navigator.userAgent` as "<browser> on <device>" (for example "Safari on iPhone"). `new_device` caps it at 64 characters (`src/core/remote.rs:166-175`). Scope is chosen in the pairing window as today, and the device is listed and revocable in the devices dialog as today. **[O kind]** is answered by default with no `kind` field. The name is enough.

## The record

- **`devices.json`: no change.** It stays a bare `Vec<PairedDevice>` with no version (`src/adapters/remote.rs:80-83`).
- **`identity.json` (`Identity`, `src/ports/remote.rs:26-42`): two fields.** `cert_names: Vec<String>` (the subjectAltName names as text) and `cert_issued: SystemTime`, both `#[serde(default)]`. `IDENTITY_SCHEMA_VERSION` is bumped from its value at implementation time, plus one. Today it is 1 (`src/ports/remote.rs:17`), so the bump is to 2 unless another ticket lands first. The loader accepts the previous version, and the defaults make an old file reissue on its first bind, because an empty `cert_names` never matches. The implementer checks the version gate in `load_identity` (`src/adapters/remote.rs:74`) and its test (`:980-990`).
- **Reissue.** `make_identity` (`src/adapters/remote.rs:95`) sets the SANs on the new certificates: `127.0.0.1`, the LAN address from `local_address()`, and the DNS name `<default_server_name()>.local` (`:304`, the name Bonjour publishes at `src/adapters/bonjour.rs:41`). A DNS name rcgen refuses is dropped, and the IP stays. The certificate gets `ServerAuth` as its extended key usage, `digitalSignature` as its key usage, and a validity of 800 days. The new function `remote::reissue_certificate(identity, key_pem, names) -> Result<Identity, String>` sits beside `make_identity`. It loads the key with `KeyPair::from_pem` (the pattern at `:139` and `:146`) and checks the SPKI is still `identity.spki_sha256`. The rcgen 0.14 API for `SanType` and the DNS name type is checked in `~/.cargo/registry` at implementation time, not from memory.
- **When.** In `bind_remote` (`src/app.rs:2936`), before `tls_config` (`:2957`), and only when the wanted names differ from `cert_names` or the certificate is more than 700 days old. Otherwise nothing is written. A reissue reaches the core as `AppAction::IdentityReissued` and then `Effect::SaveIdentity` (`src/core/action.rs:734`, handled at `src/app.rs:2893`). That keeps the core's copy and the file the same, so a later save cannot write back the old certificate. If reissue fails, the old certificate stays, the port still binds, and a log line records the failure.
- **Why not on every bind.** A profile installed from `/host.cer` trusts one certificate. A reissue means a reinstall, so the certificate changes only when the names do or it nears expiry.
- **Not on disk:** the cookie (in the browser), and the page's `localStorage` marker, which holds `{server_id, spki_sha256, device_name}` and no secret.
- **Schema versions as they are today:** `SCHEMA_VERSION` 16 (`src/core/model.rs:12`) and `VIEWS_SCHEMA_VERSION` 7 (`src/core/model.rs:612`) are unchanged. No `model.rs` type changes.

## Signals

None new. The page reads `changed` and `removed` only when the socket is up, and both already exist. The page keeps the `revision` each reply carries, per resource. It ignores a reply older than the revision it holds, and it refetches everything on a new `epoch` (§4.4).

## Actions and effects

- **`AppAction::IdentityReissued(Identity)`** (new, `src/core/action.rs`). The core ignores it unless `server_id` and `spki_sha256` are unchanged. Otherwise it replaces the identity and emits `Effect::SaveIdentity`.
- **`remote_pair`** (`src/app.rs:3606`). With `"browser": true`, the body has no `token`, and the reply's headers carry the `Set-Cookie`. The rest is unchanged: the code is single-use, and `AppAction::Paired(device)` is as today.
- **`authenticate`** (`src/app.rs:3127`). It reads the bearer first. Failing that, it reads the cookie `__Host-sb-<identity.server_id>` through a pure `cookie_value` helper in `src/adapters/remote.rs`. The Origin check runs before either. Refusals keep their codes.
- **`GET /api/v1/device`** (new arm in `serve_remote`, `src/app.rs:3017`, after `authenticate`). It returns `{device_id, name, scope, server_name}`, read from the authenticated device and the identity. It has no effect.
- **The static arm** is in `connection`, not on the frame. Its `GET` and exact-path match write the response with `respond_static` and close. The prefix refusal in `serve_remote` (`src/app.rs:3023`) is unchanged for every other path.
- **`RemoteRequest`** (`src/adapters/remote.rs:360`) gains `host`, `origin` and `cookie`, taken from the `headers` vector `read_request` already builds (`:444`). No test constructs `RemoteRequest` (grep finds none in `tests/`), so only the adapter changes. **[S: RemoteRequest built only in adapters]**.
- **`RemoteReply`** (`src/adapters/remote.rs:389`) gains `headers: Vec<(String, String)>`. The 10 literal sites in `src/app.rs` take `headers: Vec::new()`. `respond` (`:530`) writes them.
- **Refusals.** `refuse` (`src/app.rs:3754`) logs method and path and never the query, and that stays true. The new refusal is `403 bad_origin`.
- **No new `Effect` variant, no new port method.** `SaveIdentity` is the existing effect.

## The page and the native UI

### The page (`web/`)

**Pairing**, in this order:

1. First statement: read `code`, `key` and `id` from `location.search` into locals, then `history.replaceState(null, '', '/')`.
2. `GET /api/v1/info`. Its `spki_sha256` must equal `key` and its `server_id` must equal `id`. Otherwise show "This is not the host whose code was scanned" and post nothing.
3. `POST /api/v1/pair` with `{code, spki_sha256: key, device_name, browser: true}`. The page drops `code` from memory after the call.
4. On success, write the `localStorage` marker, fetch `/api/v1/device`, and show the list.
5. On refusal, show the host's message (`wrong_code`, `wrong_key`, or the window's closed state). No retry loop.

**Paired use:**

- **Polling is the default.** While the page is visible, every 3 s it reads `/api/v1/device` (scope and name, so a scope change shows within a poll), the projects and each project's sessions, and the open session's view with `last-response` or `screen`. Excerpts are refetched when a Claude session's `last_stop_at_ms` moves.
- **The socket is an enhancement.** After `hello`, polling slows to 15 s, the page refreshes on `changed`, and it sends `watch` for the open session's one key only. If the socket fails, the page says "live updates unavailable" and goes back to 3 s polling. Nothing requires the socket.
- **Marker check on load.** If `/api/v1/info`'s `spki_sha256` differs from the marker's, the page shows "identity changed, pair again" and sends nothing else.
- **Revoked or expired.** Any `401` after a successful pairing stops polling and shows "This browser was revoked, or its sign-in expired. Pair again."
- **Reply box.** Shown only when `scope` is `control`, and hidden when the session has `trust_question` set or a `prompt_refusal` (`control/src/lib.rs:662` and `:666`). **[O prompt refusal]** is answered by default with hidden. The op id is `crypto.randomUUID()` when `window.isSecureContext`, and otherwise random hex from `crypto.getRandomValues` **[U 4]**. A timed-out send is checked at `/api/v1/ops/{op}` (§4.5).
- **Order.** Waiting sessions first, each marked, then the rank in §8 by the card word. Grouped by project.
- **Excerpt.** The first non-empty line of the last response's `final_text`, cut at 120 characters.

### The native UI (`src/ui/remote.rs`)

- **QR** (`:160-166`). The payload becomes `pairing_url(host, port, code, spki_sha256, server_id)`, a pure function in `src/core/remote.rs` so the core test can pin it. The `serde_json::json!` goes.
- **Copy invite** (`:199-206`) copies that URL, and the text beside the QR shows it too.
- **Window text.** "Scan with a phone's camera. Its browser will show a certificate warning for this address; accept it. While this window is open, anyone who can answer at this address can take the code, so open it only where you are. The code is single-use, and the window closes after five minutes or three wrong codes. The code may already be in the phone's history; it stops working once the phone pairs."
- **Devices dialog** (`:230`). Browser devices show by name, with Revoke. A line reads: "A browser keeps the certificate exception it was given. Revoking a device signs its browser out on the next request. If the browser will not load the page after the warning, install the certificate at the address shown here." It shows `https://<address>/host.cer`.
- **README** (dev aids, lines 271-276). "Copy invite" now copies the URL. `switchboard-remote pair "$(pbpaste)"` still works.

### Swift

- `PairingInvite.parse` (`client/Sources/SwitchboardProtocol/Info.swift:114`) tries the URL first, with `URLComponents`: the `https` scheme, the host and port, and the `code`, `key` and `id` query items. It sets `addresses` to `["host:port"]`. Otherwise it parses the JSON as before.
- `switchboard-remote`'s usage text (`client/Sources/switchboard-remote/main.swift:12`) names the URL.
- No view changes, so no fixture changes. `/device` has no Swift decoder in this cut.

## Tests

Tests are listed by file. Nothing is gated on a device run except the manual checklist.

**Core** (`src/core/tests.rs`):
- `pairing_url` for an IPv4 host and an IPv6 host (bracketed).
- `IdentityReissued` keeps the id and the SPKI and emits `SaveIdentity`. A reissue with another SPKI is ignored.

**Adapter** (`src/adapters/remote.rs` tests):
- `cookie_value` finds the named cookie among several.
- `reissue_certificate` keeps `spki_sha256`, records the names, and yields a TLS config that accepts a pinned handshake.
- A version-1 identity loads with the defaults.

**Spike** (`spikes/23-browser-page/README.md`, the next free number after `22-remote-port`). CLAUDE.md requires a spike directory before a risky mechanism is built on. The reissue is that mechanism. The spike writes a certificate, checks it with `openssl x509 -noout -text` (SAN with the IP and the DNS name, `serverAuth`, validity of 825 days or less, SPKI unchanged), and records the numbers. The Safari items (**[U 1]**, **[U 5]**, **[U 8]**, **[O A]**, **[O B]**, **[U 6]**) are device checks, and the spike README lists them.

**Integration** (`tests/remote.rs`, a real TLS listener with the pinned rustls client like `http` at `:245`):
- `the_page_and_its_files_are_served_before_pairing`: `/`, `/app.js`, `/app.css` and `/host.cer` return 200 with their content types, the headers in §4, and `Connection: close`. The CSP string is checked exactly.
- `the_page_references_only_served_paths`: parse `web/index.html` and `web/app.js` as text. Every `/api/v1/` literal matches an allowed route, every `src` and `href` is a served path, there is no `<script>` without `src`, no `on*=` attribute, and no `http(s)://` outside the host.
- `the_allowlist_refuses_everything_else_with_no_side_effect` (`:682`) is extended: the four paths answer, `POST /` and `/app.json` are refused.
- `browser_pairing_sets_a_cookie_and_keeps_the_token_out_of_the_body`: the cookie's name, `HttpOnly`, `Secure`, `SameSite=Strict`, `Path=/`, and `Max-Age`. The body has no token.
- `the_cookie_reads_the_device_and_opens_the_socket`: `/api/v1/device` with the cookie returns the scope, and the upgrade with the cookie returns `101` (via `subscribe_at`, `:1960`).
- `a_cookie_request_from_another_origin_is_refused`: a foreign `Origin` gets `403 bad_origin`. A matching `Origin` and a request with no `Origin` both succeed.
- `revoking_a_browser_signs_it_out_on_the_next_request`: the cookie gets `401 unauthorized` after `RevokeDevice`, and the socket closes (as `revoke_closes_the_devices_socket`, `:2892`).
- `the_url_pairs_the_way_the_cli_does`: the pinned client takes the URL's three values, reads `/info`, compares the key, and posts `/pair` without `browser`. It gets the token.
- `the_pin_survives_a_reissue`: after a reissue at bind, the pinned client still connects.
- `static_connections_do_not_starve_a_paired_client`: four static connections close after their replies, and a paired subscriber keeps receiving (the pattern at `:3226`).
- Logging is checked by review, not a test: the static arm has no log call, and `refuse` logs method and path.

**UI** (`tests/ui.rs`): the pairing window's Copy invite dispatches the URL that `pairing_url` builds, and the devices dialog lists a browser device by name with Revoke.

**Swift** (`client/Tests/SwitchboardProtocolTests`): `PairingInvite.parse` on the URL (IPv4, and IPv6 in brackets) and on the old JSON. The URL literal is the same string the Rust test asserts, so drift fails on one side. The Swift tests are not in the pre-commit hook or CI (`docs/design.md`, remote status A2), so they run by hand with `swift test` in `client/`.

**Manual, on a phone** (owner acceptance, recorded in the spike README): the QR opens the page, the warning is accepted, the sessions appear, a session that starts waiting moves to the top within a poll, a reply arrives, revoking the browser produces "revoked" on the next poll, and the items in **[U 1]**, **[U 5]**, **[U 8]**, **[U 6]** and **[O A]**. The page's JS is not in CI.

## Order of work

Each step builds, and the app runs after it. All steps land in the one commit.

1. Spike 23: the reissue and its checks. No app change.
2. Adapter plumbing: the `RemoteRequest` and `RemoteReply` fields, `respond`'s headers, `cookie_value`, and the Origin check. Behaviour is unchanged.
3. The static arm and the page's first screen (the scan instruction). The header and reference tests.
4. The cookie on `/pair` and in `authenticate`, and `/device`. Their tests.
5. The reissue at bind, `IdentityReissued`, the identity fields and the schema bump. Their tests.
6. `pairing_url`, the QR, Copy invite, the window and devices text, the Swift parse, and the README dev-aid text.
7. The page's pairing, list, session, reply and states. Checked by hand on a phone.
8. Documentation: `docs/remote-client.md` §4.1 (static paths and pools), §4.3 (the four paths, `/device`, `browser` on `/pair`, `bad_origin`, and line 207's excerpt), §5 (the URL, the browser trust step, the relay sentence at `:309`, the cookie, and line 306's list). `docs/design.md`: a bullet at the end of the remote client status (`:3107`, before `## Versioning and updates` at `:3585`). `README.md`: Layout rows for `web/` and `src/adapters/remote_web.rs`, and the remote adapter's row at `:383` (the static arm). `CHANGELOG.md`: a line under Unreleased.

## Owner follow-ups (named, not built)

- **[O A]** After the device test: if `fetch` fails after the click-through, the page's instructions change to the profile install, and the code does not. If `wss` works only with the always-trust, the socket is the enhancement it already is, and the profile route becomes a decision, because it changes §5's trust story.
- **[O D]** If the cookie fails the device test: a token in a `Sec-WebSocket-Protocol` value. It is a new decision, because the token then sits in script memory.
- **[U 9] and [O B]**: if the IP is refused and the `.local` name is needed, the URL host switches (a one-line change, §3). If `.local` does not resolve either, the URL carries the IP and the owner reviews the pairing flow.
- **A static pool**, with its own bound, if the device test shows drops at accept.
- **A distinct `device_revoked` code** (additive), if the owner wants the host to name revocation.
- **A `kind` on the device record**, if the owner wants browser and native devices told apart in the dialog.
- **An excerpt on `SessionView`**, which needs the Stop message stored on the record (a record change and a `SCHEMA_VERSION` bump), if the per-session `last-response` polling is too heavy.
- **Profile reinstall on reissue**: a certificate installed as a profile needs reinstalling when its names change. If the address changes often, the owner decides between a stable `.local` URL and reinstalls.
- **A Host allowlist** beyond the Origin check, if the owner wants one.

## Contract additions

- `src/core/action.rs`: `AppAction::IdentityReissued(Identity)`.
- `src/ports/remote.rs`: `Identity` gains `cert_names: Vec<String>` and `cert_issued: SystemTime`, both `#[serde(default)]`. `IDENTITY_SCHEMA_VERSION` is bumped from its value at implementation time plus one (1 today, at `:17`).
- `src/adapters/remote.rs`: `RemoteRequest` gains `host: Option<String>`, `origin: Option<String>`, `cookie: Option<String>`. `RemoteReply` gains `headers: Vec<(String, String)>`.
- `src/core/remote.rs`: `pairing_url(host, port, code, spki_sha256, server_id) -> String`.
- `src/adapters/remote.rs`: `reissue_certificate(identity, key_pem, names) -> Result<Identity, String>`, `cookie_value(...)`, `respond_static(...)`.
- Endpoint `GET /api/v1/device` (authenticated), body `{device_id, name, scope, server_name}`.
- Endpoint `POST /api/v1/pair`: optional body field `browser` (bool, default false). With `true`, the reply sets the cookie and omits the token.
- Static paths `GET /`, `/app.js`, `/app.css`, `/host.cer` (unauthenticated).
- Cookie `__Host-sb-<server_id>`, carrying the bearer token.
- Refusal `403 bad_origin`.
- Pairing URL `https://<host>:<port>/?code=<code>&key=<spki_sha256>&id=<server_id>`, accepted by `PairingInvite.parse`.
- Not changed: `SCHEMA_VERSION` 16 (`src/core/model.rs:12`), `VIEWS_SCHEMA_VERSION` 7 (`:612`), `devices.json` (no version, no new field).

## Commit message

```
Serve a browser page from the remote port and pair it from a URL
```
