# Issue #230: a web control surface on the remote port

Plan against worktree `/Users/sbgragent/.dispatch/worktrees/1ea22e2f`, HEAD `7fe8949`. Every reference below was read in that worktree. Browser behaviour marked **verify** is not known from the code and is settled by the spike in "Spike first".

## Where this departs from the issue

Each claim the code does not bear out, and what the plan does instead.

1. **"Each with its kind, state and the one-line excerpt the card shows."** The card's excerpt is computed in the UI from the transcript (`src/ui/cards.rs` 176–185 picks the caption; `agent_excerpt` 558–575). The session view has `card` (the word), `waiting` and `waiting_reason` (`control/src/lib.rs` 637–660), and no excerpt. The plan: the list shows kind, `card` and `waiting_reason`; the excerpt appears in the session view, from `last-response`. No excerpt field is added to the wire this cut.
2. **"The code never stays in the address bar or the history"** with the code in the query. `history.replaceState` clears the address bar only after the page's script runs. The browser sends the full URL in the GET request line to whichever host answers it, before any script runs, so a stale QR whose address now belongs to another host gets the code. The plan puts the pairing values in the **fragment** (`/#code=…&key=…&id=…`). A fragment is never sent in a request. This is a deviation from the issue's text.
3. **"Open the pairing window, scan the QR."** With the local network off the port binds loopback (`src/app.rs` 2962), yet the window opens and its QR carries `local_address()` regardless (`src/ui/remote.rs` 157), and `OpenPairing` has no `lan` check (`src/core/remote.rs` 46–52). The phone cannot reach that address. The plan: the **dialog** refuses to open the window with lan off ("Turn on Allow devices on the local network first") and disables the button. The core keeps accepting the action (see "Actions and effects" for why the guard is not in core).
4. **"The page keeps the token."** `/pair` puts `token` in the JSON body (`src/app.rs` 3656–3661). A script that reads the reply holds the token, so a browser cannot be given the body. The plan: a browser asks with `browser: true` and receives a cookie and no token (decision 2).
5. **"Identity changed, pair again" versus "revoked."** A revoked token and a rotated key both get the same 401, "the token is not known; pair again" (`src/app.rs` 3142–3147); rotation clears `devices.json`. The page cannot tell them apart from the reply. The plan: the page keeps the server's SPKI hash in `localStorage` (not secret) and compares it with `/info` on a 401. A different key is "identity changed"; the same key is "revoked".
6. **"The device's scope and host name in the header."** The host name is in `/info` (`server_name`, `src/app.rs` 3568–3596), which is pre-auth. Nothing lets a paired device read its own scope; `/pair` returns it once. The plan adds `GET /api/v1/device`.
7. **"A session that starts waiting moves to the top without a reload."** This does hold. `waiting` is a diff resource (`src/core/revision.rs` 56) and `changed {waiting}` plus `changed {session}` reach a subscriber. No host change is needed for it.

Nothing else in the issue's "What exists" section needs a departure: the QR fields, the five-minute and three-failure window (`src/ui/remote.rs` 194; `src/core/remote.rs` 58–63), the three protocol speakers, and the `/api/v1/` allowlist (`src/app.rs` 3023–3025) are as described.

## The record

- **`devices.json` is unchanged.** A browser is a `PairedDevice` (`src/ports/remote.rs` 96–110). Its `name`, `scope` and token hash are set as decision 6 says. Nothing about the browser's cookie is stored on the host beyond that hash.
- **`identity.json` is unchanged** unless the spike rejects the certificate (decision 1). The certificate has no SANs and rcgen's default validity (`src/adapters/remote.rs` 91–102). If Safari refuses such a leaf outright rather than warning, the fix is to re-issue the certificate over the same Keychain key with a finite validity, replacing the `certificate` field only. The SPKI hash, and so every native pin, does not change.
- **Browser-side state**, in `localStorage` and wrapped in try/catch: the server id, the SPKI hash last seen, and the device name. Never the token; the cookie is HttpOnly (decision 2).
- **Schema versions, as they are today:** `SCHEMA_VERSION` 16 (`src/core/model.rs` 12), `VIEWS_SCHEMA_VERSION` 7 (`src/core/model.rs` 612), `IDENTITY_SCHEMA_VERSION` 1 (`src/ports/remote.rs` 17). `devices.json` has no version field. **No bump is planned.** If the spike forces an identity change, that change still needs no bump, since the `certificate` field already exists. If the implementer adds any field to `model.rs`, the bump is that file's `SCHEMA_VERSION` value at implementation time plus one.

## The decisions

### Decision 1: browser trust in the self-signed certificate

The click-through is the trust step. The Remote devices dialog says so: "A browser trusts this Mac only through the certificate warning you accept. Accept it only on your local network, and only for the address shown here."

What the click-through buys: the browser keeps the connection's key for that address, for as long as the exception lasts. What it does not buy: identity. A page the click let through can say whatever it likes. The page's `key` check stops a genuine page from sending the code to a host with a different key; it cannot stop a page served by the party the click let through.

The host does not offer its certificate for download. The reason is the iOS install path: trusting a root takes a profile install and a separate trust-settings toggle (**verify** whether a self-signed leaf with no CA bit is accepted at all). The reason is not `docs/remote-client.md` §5. The certificate carries no secret, so a download route would not break the rule; the page exception is justified under decision 4.

### Decision 2: the token reaches the page through a cookie

The `/pair` reply to a body with `browser: true` sets

`Set-Cookie: switchboard-<server_id>=<token>; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=34560000`

and leaves `token` out of the body. Native clients send no `browser` field and keep the bearer header and the body token. The `Max-Age` is 400 days; a browser may cap it lower. A lost cookie means the owner pairs again and revokes the old record on the Mac; the device record is not lost, only the browser's copy of the token.

`SameSite=Lax`, not the issue's `Strict`, because a link opened from Messages or Mail is a cross-site navigation: `Strict` drops the cookie and the paired phone shows "not paired". `Lax` still withholds the cookie from cross-site `fetch`, `POST` and WebSocket, which are the state-changing requests. The cookie name is per installation. Cookies are keyed by host, not port, so two instances on one address with different ports are kept apart only because each name differs.

**Origin rule.** `authenticate` (`src/app.rs` 3127) accepts the bearer first and the cookie second. For a cookie-authenticated request, `read_request` keeps `Origin`, `Host` and `Cookie` (`src/adapters/remote.rs` 444–516), and the host applies this:

- a request that is not `GET` (a send, a resume, a trust, a draft write, a new session) must carry an `Origin` equal to `https://<Host>`. Without one, or with another, it is refused 403 `origin`;
- a `GET` (a read, or the events upgrade) that carries an `Origin` must match it the same way. A `GET` without one passes.

A cross-site page cannot make a write, because it cannot forge the `Origin` of a write it sends; it cannot read a reply, because the browser withholds it. A non-browser client holding the cookie reads what the cookie allows and writes only with the bearer, which it may send instead. The `/pair` reply stays HTTP 200 with the cookie; the 401 for a revoked token is unchanged.

**Honest limit.** A script on this origin still makes authenticated calls, so XSS in the page is still a way in. The protections are: the page has no inline script and no external resource (the CSP in decision 4), the cookie cannot be read by script, and revocation works on the next request.

**Spec amendment.** `docs/remote-client.md` §5 (line 312) says the token is "sent only as a bearer header … never in a URL", and line 313 says `/pair` answers "with the token". Both gain the browser's cookie path. The token still never travels in a URL.

### Decision 3: one QR or two

One URL, in the fragment form of decision 2's pairing, for the QR, the text beside it, and "Copy invite". The JSON form stops being produced by the host.

The parsers keep reading JSON: `PairingInvite.parse` (`client/Sources/SwitchboardProtocol/Info.swift` 113–116) and `switchboard-remote pair` (`client/Sources/switchboard-remote/main.swift` 107–112) accept both forms. So text copied from the previous build still pairs with a rebuilt client.

This is a change to the invite format, not to `/api/v1`, and it is breaking in one direction: a `switchboard-remote` binary built before this commit cannot parse the URL and must be rebuilt. An installed visionOS or iPadOS client pairs by Bonjour and a typed code (`client/Sources/SwitchboardClientCore/ConnectionManager.swift` 471–487) and does not read the QR, so it is unaffected. The URL carries one `address:port`; the JSON that still parses keeps its `addresses` array.

The three-argument CLI form (`host:port code spki`) is unchanged.

### Decision 4: static paths, headers, pools, and the page's states

**Routing.** Today `GET /` is refused on the frame (`src/app.rs` 3023–3025, 404 `not_found`), and the frame drains only 8 requests per pass (`src/app.rs` 1584–1596). Serving the page through the frame would make every page load wait for a frame pass. The static set is answered in the adapter, in `connection()` before `requests.try_send` (`src/adapters/remote.rs` 770–812). The frame never sees these requests.

**Exact set.**

| Request | `Content-Type` | Body |
| --- | --- | --- |
| `GET /` | `text/html; charset=utf-8` | `web/index.html` |
| `GET /app.css` | `text/css; charset=utf-8` | `web/app.css` |
| `GET /app.js` | `text/javascript; charset=utf-8` | `web/app.js` |
| `GET /favicon.ico` | none | 404, empty body |

Other methods on these paths, and every other path outside `/api/v1/`, fall through to the frame and get the existing 404 `not_found`. The bytes come from `include_str!` in `src/adapters/remote_page.rs`.

**Headers on each 200.**

- `Content-Security-Policy: default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'`
- `Cache-Control: no-store`
- `X-Content-Type-Options: nosniff`
- `Referrer-Policy: no-referrer`
- `Connection: close` (the reply is unauthenticated; see "Actions and effects")

The page's markup and script obey the policy: no inline script, no inline style, no `style=` attribute, no `eval`, no external resource. Whether `connect-src 'self'` allows `wss:` to the same host is spike item 3.

**Why the static paths may sit outside `/api/v1/`.** `docs/remote-client.md` §5 says "require pairing before anything but `/info` and `/pair`". The amendment adds the four static paths to that list. The justification: the bytes hold no session data and are the same for every host of this version. What an unpaired stranger learns is the page's source, which shows the protocol the page speaks. The spec already documents that protocol in §4.3, so the page exposes no secret, and the page cannot act until it pairs. This is a judgement, not a proof, and the owner accepts or rejects it on the §5 amendment at line 306, which this section names. The §4.1 frame rule (line 123) is amended with it.

**Pools.** Each static response closes its connection, so a page load's parallel connections each hold an unauthenticated slot for one request. The bound stays at 4 (`MAX_UNAUTHENTICATED`, `src/adapters/remote.rs` 567). It moves to 8 only if spike item 4 shows a reset, and then §4.1 is amended by the owner's decision.

**Rate.** A `page` window of 60 per minute per listener covers the three 200 paths, added to `Limits` (`src/adapters/remote.rs` 623–640). Past it, the connection answers the existing 429 JSON error. The window is shared by all devices on the listener; a few reloads a minute fit within it.

**The page's states.** The host serves the same bytes whatever the state; the page's script decides, as the UI section lists. The no-code case needs no host branch: with no fragment and no cookie, the page calls `GET /api/v1/info` and shows "Pairing is closed" or "Scan the code on the Mac" from `pairing_open`.

### Decision 5: where the page lives and how it is tested

`web/` at the repo root holds `index.html`, `app.css` and `app.js`, with no framework and no build step. `src/adapters/remote_page.rs` embeds them. The tests are listed under "Tests". The page is tested by the host's own HTTP client (the CLI's path), since no test drives a browser: the spike covers the browser's own behaviour.

### Decision 6: the device record and scope

- **Name.** The page parses `navigator.userAgent` into a device and a browser: `iPhone`, `iPad`, `Mac`, `Android` or `Device`; `Safari`, `Chrome`, `Firefox`, `Edge` or `browser`. It sends `device_name` as `Safari on iPhone`. The host trims it to 64 characters (`src/core/remote.rs` 166–175) and keeps "Device" when it is empty (`src/app.rs` 3651–3654). The name is cosmetic.
- **Scope.** The scope radio in the pairing window (`src/ui/remote.rs` 215–217) sets a browser's scope, exactly as it does for a native device. The dialog's combo (`src/ui/remote.rs` 243–253) changes it later. A browser cannot change its own scope, and no request does.
- **Reading scope.** The page learns its scope from `GET /api/v1/device` (new, read scope, `{device_id, name, scope}`) on each load and after any 403 `scope`. A read device sees no reply box. A scope change reaches the page at its next request, as the existing 403 `scope` already does (`src/app.rs` 3166–3173).
- **Revocation.** Revoke in the dialog removes the record. The cookie fails with 401 on the next request, and the page says so (UI, "401").

## Signals

No new event. The page uses what exists: `hello` (epoch and revision, `src/app.rs` 2738), `changed` and `removed` from the diff pass for `waiting`, `session`, `projects`, `sessions` and `conversation`, and `resync_required` (`docs/remote-client.md` §4.4). The page sends `watch {kind: "conversation", id}` for a Claude Code session and `watch {kind: "screen", id}` for any other, as §4.4 describes. A watch set is capped at 128 keys globally, so the page unwatches when a session view closes.

## Actions and effects

No new `AppAction` or `Effect` variant, and no port change.

- **Lan guard in the UI, not core.** Core keeps accepting `OpenPairing` with lan off. The reason: the default identity has `lan: false` (`src/ports/remote.rs` 66, and `src/app.rs` 2836 for the first-run identity), so every pairing fixture runs that way. `tests/remote.rs` opens windows at 407 (the `pair()` helper, called 35 times by grep), 584 and 1627, and `src/core/tests.rs` opens them at 11878, 11899, 11922 and 11936 under `identity()` at 11768–11776, which uses `Listen::default()`. A core refusal would break all of them. The guard lives in `src/ui/remote.rs`: the button is disabled while `identity.listen.lan` is false, and the window shows the words. The dev aid `pairing <code>` (`src/script.rs` 730) still opens a window with lan off. It is a dev aid, and the README's dev-aid line says so.
- **`invite_url`**: a pure function in `src/core/remote.rs`, `invite_url(identity, address, code) -> String`, that builds `https://{address}:{port}/#code={code}&key={spki_sha256}&id={server_id}`. The address is the IPv4 string from `local_address()` (`src/adapters/remote.rs` 962).
- **`RemoteReply` gains `headers`** (`src/adapters/remote.rs` 389–395), and `respond` (`src/adapters/remote.rs` 530–553, which hard-codes `application/json` today) takes a content type and the extra headers. The literals move to a `RemoteReply::json(status, body, authenticated)` constructor, which sets `headers` to empty. There are ten literal `RemoteReply { … }` constructions in `src/app.rs` (1664, 1679, 2365, 2371, 2376, 2754, 3570, 3723, 3736, 3767) and one in the adapter (829). The other 18 matches of the text are return types. The change is mechanical.
- **Unauthenticated replies close the connection.** A connection that has not had an authenticated reply (the `paired` flag in `connection()`) closes after every reply it gets. That covers the frame's replies, the static 200s, and the adapter's own writes. Those writes are the 429 at `src/adapters/remote.rs` 789–796 and the 503 at 819–824, which today write and `continue` the loop, so a rate-limited stranger holds a slot for up to `IDLE_TIMEOUT` (30 s). Both now return after the write. A paired connection keeps keep-alive as it does now. This is a §4.1 change.
- **`/pair` for a browser**: `remote_pair` (`src/app.rs` 3606–3670) reads `browser`. The reply carries `Set-Cookie` and no `token`.
- **`GET /api/v1/device`**: built as `remote_info` builds its body (`src/app.rs` 3568), under `authenticate`.
- **Cookie in `authenticate`** and the Origin rule (decision 2).
- **Static set** and the page `Limits` window (decision 4).

## UI

**Pairing window** (`src/ui/remote.rs` 141–200). The address line and the key line stay. The QR encodes `invite_url`, as does the text beside it, and "Copy invite" (line 201) copies the same string. The line at 194 becomes "Scan with the phone's camera, or copy the link into the device. Grants {scope} scope. Open for five minutes or three wrong codes." With lan off, the button is disabled and reads "Allow devices on the local network above first". With lan on and no `local_address()`, the window shows "No local address; check the network" and no QR.

**Devices dialog** (`src/ui/remote.rs` 229–276). Adds the trust sentence from decision 1. Rows are unchanged: name, scope (the combo at 243–253), last seen, Revoke.

**The page** (`web/`).

- *Load.* `app.js` runs first. It reads `location.hash` into `URLSearchParams`, then calls `history.replaceState(null, '', '/')`, then does anything else. The code is uppercased, as the core and `/pair` do (`src/core/remote.rs` 50; `src/app.rs` 3646).
- *Pairing.* With the fragment present, the page fetches `GET /api/v1/info` and compares `key` with `spki_sha256` (case-insensitive). On a mismatch it shows "This is not the Mac whose code was scanned" and sends nothing, so the window's failure counter does not move (`src/app.rs` 3636–3645). On a match it posts `POST /api/v1/pair` with `{code, spki_sha256, browser: true, device_name}`. On 200 the cookie is set and the page forgets the fragment. On 403 `wrong_code` it shows "The code does not match. The window closes after three wrong codes." On 403 `pairing_closed` it shows "Pairing is closed on the Mac".
- *No fragment, no cookie.* `/info` decides. If `pairing_open` is true: "Scan the code on the Mac, or open the link it shows." If false: "Pairing is closed. On the Mac, open Remote devices and choose Allow pairing for 5 minutes."
- *With a cookie.* The page opens `wss://…/api/v1/events` first, waits for `hello`, then snapshots `GET /api/v1/projects`, `GET /api/v1/projects/{p}/sessions` for each project, `GET /api/v1/waiting` and `GET /api/v1/device`. Sessions are grouped by project, with the waiting ones first and marked.
- *Session view.* `GET /api/v1/sessions/{id}`. A Claude Code session shows `GET …/last-response`; anything else shows `GET …/screen`, refreshed on `changed`. A reply box posts `POST …/sessions/{id}/send` with the body §4.5 gives. The box is shown when `scope` is `control` and hidden otherwise. A 403 `scope` on send says "This device reads only. Raise it on the Mac."
- *401.* Compare the stored SPKI hash with `/info`: a different key shows "The Mac's key changed; pair again"; the same key shows "This browser is no longer paired". Nothing is sent after either.
- *Socket loss.* Reconnect with backoff. `resync_required` and a changed `epoch` in `hello` both refetch everything.
- *Status line.* The header shows the host name from `/info` and the device's `name` and scope from `/device`.

Not in this cut: forget-this-browser (the owner revokes on the Mac), dispatch tickets, drafts, dictation, runs, pinned files.

## Spike first

CLAUDE.md requires a spike directory before building on a risky mechanism. The browser questions are not answerable from the code. Add `spikes/23-browser-page/` (the next free number after `spikes/22-remote-port`) with a README recording commands, numbers, the iOS and Safari versions, the date, and a recommendation.

Who runs it: the owner, who holds the phone, runs the iPhone items. The implementer runs the Mac-side items on a second Mac and writes the README, and records the owner's observations with the owner's confirmation. The questions:

1. The certificate warning: is the exception per address and sticky across reload and relaunch; does it cover `wss://` on the same host; does Safari **refuse** a self-signed leaf with no SANs and a validity to year 4096, or only warn (decision 1)?
2. Does `Secure; HttpOnly; SameSite=Lax` on an IP-literal origin store the cookie and send it on `fetch` and on the WebSocket upgrade; does a link opened from Messages carry it?
3. Does `connect-src 'self'` allow `wss:` on the same host under the CSP in decision 4?
4. A cold page load of `/`, `/app.css`, `/app.js` and `/info` in parallel, with the pool at 4 and `Connection: close` on unauthenticated replies: does any request reset?

Which steps wait on which outcome is in "Order". If the spike cannot run before the work is finished, steps 1 to 3 can be finished and committed on the branch; step 4 onward is not merged. There is no fallback that ships the page unverified on Safari. The issue's acceptance needs the phone.

## Tests

- **`tests/remote.rs`**, with a `http_with` helper beside `http` (245–297) that takes request headers (`Cookie`, `Origin`, `Host`) and returns response headers. `http` discards them today.
  - `the_page_and_its_assets_answer_with_their_headers` (new): `GET /`, `/app.css`, `/app.js` answer 200 with the right `Content-Type`, the CSP string exactly as decision 4 gives it, `Cache-Control: no-store`, `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer` and `Connection: close`. `GET /favicon.ico` is 404 with no body. `GET /other` is still 404 `not_found` (the allowlist test at 682 extended).
  - `the_page_references_only_served_paths` (new): scan `web/app.js` and `web/index.html` for `/api/v1/…` literals and `href` or `src`. Request each with a paired cookie and assert none answers 404 `not_found`. This catches a route added to the page and not the host.
  - `a_browser_pairs_by_cookie_and_the_cookie_reads_and_revokes` (new): pinned client, `GET /info`, `POST /pair` with `browser: true`. Assert `Set-Cookie` carries `HttpOnly; Secure; SameSite=Lax`, the body has no `token`, `GET /api/v1/device` and `GET /api/v1/projects` answer 200 with the cookie, and after `RevokeDevice` the next request is 401.
  - `a_cookie_is_checked_against_its_origin` (new): a cookie `POST` with no `Origin` → 403 `origin`; with a foreign `Origin` → 403 `origin`; with `https://<Host>` → 200. A cookie `GET` with a foreign `Origin` → 403; with no `Origin` → 200. A bearer `POST` with no `Origin` → 200.
  - `a_pairing_from_the_shared_invite_fixture_pairs_through_the_host_client` (new): reads `server_id`, `spki_sha256`, `code` and `address` from `client/Fixtures/invite.json` and pairs with the pinned client, as the CLI's three-argument form does. Rust does not parse the URL; the Swift test does (below).
  - `stalled_strangers_fill_their_own_pool_and_never_a_paired_clients` (3226) extended: four page loads' connections hold the unauthenticated pool; a paired client still reads; and the no-code page answers while the pool is full.
  - A rate test: the 61st request to `/`, `/app.css` or `/app.js` in a minute is 429 and the connection closes. The window counts requests, not page loads; one page load is three requests.
  - `a_wrong_pin_fails_at_the_handshake_before_any_request` (1395) and `pair_is_rate_limited_per_listener` (1406) run unchanged.
  - `info_is_served_to_anyone_and_every_other_read_is_refused_unpaired` (426): `/` now answers, so its expectations change; the rest stay.
  - `the_client_fixtures_decode_on_both_sides` (1829) and `write_fixtures` (1587): add `device.json` and `invite.json`. The decode check asserts that `invite.json`'s `url` equals `invite_url` for the fixture identity and code. `pair.json` changes only if the browser reply is a new shape, which it is not on the native path.
- **Core, `src/core/tests.rs`**: `invite_url` for a sample identity gives the exact string. (The lan refusal test is dropped; the guard is in the UI.)
- **Adapter units, `src/adapters/remote.rs`**: `read_request` keeps `Cookie`, `Host` and `Origin`; `respond` writes headers and the content type; a connection with no authenticated reply closes after a 429 and after a 503.
- **UI, `tests/ui.rs`**:
  - The dialog test at 7424 seeds `Listen::default()` (lan off). It clicks the pairing button, which is now disabled, so it seeds lan on. The Revoke part is unchanged.
  - New: with lan off, the button is disabled, no QR is shown, and the "first" message is shown.
  - New: with lan on, the QR's text, the text beside it and the Copy invite string are the same `invite_url`.
- **Swift, `client/Tests/SwitchboardProtocolTests/FixturesTests.swift`**: `testInviteParsesTheQRText` (157) keeps its JSON case. A new case reads `client/Fixtures/invite.json`, parses its `url` with `PairingInvite.parse`, and asserts the server id, key, code and address. This is the only test of the real parser against the URL form; the Rust side checks only that the fixture is what `invite_url` writes. Run on macOS; CI on Linux does not compile this target.
- **Manual.** The phone run in the issue's acceptance list is recorded in the spike README, not automated: scan, accept the warning, see the sessions, a waiting session moving, a reply arriving, revocation.

## Docs and status

- `docs/remote-client.md`:
  - §4.1: the static set is answered by the adapter and not by the frame (line 123, "The frame does core state only"); the unauthenticated-reply close and the page's rate window join the bounds (lines 127–129).
  - §4.3: the endpoint table gains `GET /`, `GET /app.css`, `GET /app.js`, `GET /favicon.ico` and `GET /api/v1/device`; `POST /api/v1/pair` takes `browser`; the cookie joins the auth column; 403 `origin` is added.
  - §5: line 306 gains the four static paths; line 309 changes the QR to the URL form (decision 3); line 312 and line 313 gain the cookie (decision 2).
- `docs/design.md`:
  - Trust boundary, lines 251–252: "a paired device reads a pane's screen" becomes "a paired device, a native client or a browser, reads a pane's screen". The sentence's rule is unchanged.
  - Remote client status (line 3107): a dated bullet when the commit lands.
  - Open questions (line 3672): the Safari answers from the spike.
- `README.md`: Layout rows for `web/` (new), `src/adapters/remote_page.rs` (new), the `src/adapters/remote.rs` row (383), the `src/ui/remote.rs` row (423), the `tests/remote.rs` row (437) and the `client/` row (446). The pasteboard path (277–278) says the Allow pairing button needs the local network on. Dev aids: none new.
- `spikes/README.md`: a row for `23-browser-page`.
- `CHANGELOG.md`: a line under `[Unreleased]` (line 9). The changelog line says the switchboard-remote binary must be rebuilt for the URL invite.
- No `src/script.rs` line: `pairing` (730) and `remote-devices` (734) already reach the states the page needs.

## Order

Steps 1 to 3 do not depend on the spike. Each step leaves the app running. The branch is squashed to one commit before merge.

1. **Now.** The UI lan guard, `invite_url` in core, the QR, the text and Copy invite; `client/Fixtures/invite.json` written by `write_fixtures`; the Swift and CLI parsers accept the URL. Tests: the core `invite_url` test, the UI tests, the Rust fixture check, the Swift case. Changes the invite format (decision 3).
2. **Now.** Adapter: `read_request` keeps `Cookie`, `Host` and `Origin`; `RemoteReply.headers` and `RemoteReply::json` (eleven sites); `respond` with the content type and headers; the close after an unauthenticated reply, including the 429 and 503 writes. Tests: the adapter units; the existing tests run unchanged.
3. **Now, pool at 4.** Static table, `src/adapters/remote_page.rs`, placeholder `web/` pages that show "Pairing is closed" in every state, the header tests, the page rate window and the `the_page_references_only_served_paths` test. The pool bound waits on spike item 4 only if it fails.
4. **Waits on spike items 2 and 4 (merged only after the spike).** `POST /pair` with `browser: true`; the cookie in `authenticate`; the Origin rule; `GET /api/v1/device`. The Rust tests land with the step. Safari's cookie behaviour is confirmed in the spike before merge.
5. **Waits on spike items 1 and 3.** The real `web/` page. If item 3 blocks `wss:`, the page polls `waiting` and the open session every five seconds, and the acceptance line "moves to the top without a reload" is amended to say so. If item 1 forces the certificate re-issue, that lands here, as decision 1 and "The record" say.
6. **Waits on everything.** Docs, README, CHANGELOG, `spikes/README.md`, the design status bullet; the phone run is recorded in the spike README.

**Commit message:** `Serve a web control surface from the remote port and pair browsers by link`

**Contract additions** (none to `src/core/model.rs`, `src/core/action.rs` or `src/ports/`):

- Wire, additive on `/api/v1`: `GET /api/v1/device` → `{device_id, name, scope}`; `POST /api/v1/pair` takes `browser: true` and then answers with `Set-Cookie` and no `token`; 403 `origin` for a cookie request that fails the Origin rule; cookie `switchboard-<server_id>` accepted by `authenticate`.
- Outside `/api/v1`, served by the adapter: `GET /`, `/app.css`, `/app.js` and `/favicon.ico` (decision 4).
- Invite format, changed and breaking for old `switchboard-remote` binaries: the QR, the text and Copy invite carry the URL form. The JSON still parses, on both sides.
- Core, additive pure function: `invite_url` in `src/core/remote.rs`.
- Adapter types (not the shared contract, named so the commit says them): `RemoteRequest` gains `cookie`, `host`, `origin`; `RemoteReply` gains `headers`; `RemoteReply::json`; `remote_page` module; `Limits` gains a `page` window.
- Swift and CLI, additive: `PairingInvite.parse` and `switchboard-remote pair` accept the URL form.
