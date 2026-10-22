# Plan: #230, a browser page on the remote port, paired from a link

Checked against the worktree at `7fe8949` (branch `stage/sb-230-plan-v3-haiku-a`, clean). Facts cited as "facts: Settled …", "facts: Unresolved n", or "facts: For the owner X" are lines of `inputs/facts.md`.

## Where this departs from the issue

Each claim below is one the tree or the facts file does not bear out, and what the plan does instead.

1. **"The one-line excerpt the card shows" on the session list.** Not on the wire. The spec says the excerpt is `last_message` from the `Stop` hook, "a field of the session's metadata" (`docs/remote-client.md:207`). The tree has no such field. `last_message` lives on the hook event (`src/ports/events.rs:23-24`), and the core binds only `pending` from it (`src/core/events.rs:132`), so it is dropped. The card's excerpt is computed in the UI from the parsed conversation (`src/ui/cards.rs:177-181`). **This contradicts facts: Settled, "The tree", the line that puts `last_message` on the session record.** The plan's list shows the card word, the waiting reason and the name. The session page shows the last response. A host-computed excerpt would need a transcript parse per listed session, so it is an owner question, not built here.
2. **"The device's scope and host name in the header."** No endpoint returns the scope after pairing. `/pair` reports it once, and `info` is unauthenticated (`src/app.rs:3568-3599`). The plan adds an authenticated `GET /api/v1/device`.
3. **"'Revoked' said plainly."** `authenticate` gives one code, `unauthorized`, for both "never paired" and "revoked" (`src/app.rs:3127-3151`). The host keeps no record of revoked tokens, so it cannot tell them apart. The page says "no longer paired: the Mac revoked this device, or forgot it", and it says "identity changed" only when `/info` presents a different key, which it can tell apart. A distinct code would need a record change, so it is an owner question.
4. **The URL `key` "lets a genuine page refuse to send the code to a host whose `/info` key differs."** That holds, but only as far as `/info` is believed. The page cannot read the certificate it talks over, and the host's own key check takes the key from the request body (`src/app.rs:3636`). Spec §5 says a relayed pairing "fails even if the code reached it" (`docs/remote-client.md:309`). For a browser, a page served by an impostor can post the right key, so that sentence does not hold. The spec amendment says so.
5. **"The code never stays in the address bar or the history."** `history.replaceState` replaces the current entry (facts: Settled, "history.replaceState"). Whether the URL was recorded before the script ran is unknown (facts: Unresolved 6). The plan claims only what the tree supports: the code is single-use, and the window closes at five minutes or three wrong codes (`src/core/action.rs:972-973`, `src/core/remote.rs:65-71`). The dialog warns that a link opened on a phone may stay in its history.
6. **Click-through alone as the trust step, with the current certificate.** The certificate has no SAN, a CN only, and rcgen's default year-4096 expiry (`src/adapters/remote.rs:101-110`). It meets none of Apple's TLS rules (facts: Settled, "Apple's TLS server certificate rules"). The plan reissues it over the same key. The spec's "the certificate is verified out of band" assumption does not hold for browsers either.
7. **The events socket as the page's live channel.** Not the default. Safari's `wss` after a click-through is unverified (facts: Settled, "Safari and `wss://`"; Unresolved 1). The page polls REST. The socket is an upgrade: when it is open the page refetches on `changed`.
8. **The cookie as the token store.** The shape makes the cookie the token store. The plan's default is the page's own storage and a header. The cookie's `Secure` on an IP host and its `SameSite` on a WebSocket handshake are unresolved (facts: Unresolved 5, For the owner D). The host reads and writes the cookie in this commit. The page does not use it until D is favourable.

Facts corrections found in the tree (these are not issue claims, but the facts file is the yardstick, so they are noted):
- `src/adapters/remote.rs:112` holds the SPKI hash, not `:107` as facts: Settled, "The tree" says.
- `RemoteReply {` appears in 10 places in `src/app.rs` by grep, not "about 15" as facts says. The implementer should count at edit time.

## The record

- **No `SCHEMA_VERSION` bump and no `model.rs` change.** `SCHEMA_VERSION` is 16 (`src/core/model.rs:12`), `VIEWS_SCHEMA_VERSION` is 7 (`src/core/model.rs:612`), `IDENTITY_SCHEMA_VERSION` is 1 (`src/ports/remote.rs:17`). None changes. The identity version is written but never checked on read, so the new identity fields are additive with serde defaults. If the implementer decides to bump `IDENTITY_SCHEMA_VERSION` anyway, the bump is its value at implementation time plus one.
- **Device record: unchanged.** A browser is a `PairedDevice` in `devices.json` (a bare `Vec`, `src/adapters/remote.rs:80-86`). Its name is the page's `device_name`, built from `navigator.userAgent` (for example `Safari on iPhone`). `new_device` cuts it to 64 characters (`src/core/remote.rs:166-175`). No `kind` field (decision 6; facts: For the owner, the `kind` question).
- **Identity: two additive fields** in `Identity` (`src/ports/remote.rs:26-42`):
  - `certificate_names: Vec<String>`, the SANs the certificate carries, `#[serde(default)]`.
  - `certificate_issued: Option<SystemTime>`, `#[serde(default)]`.
  An empty `certificate_names` or a `None` issue time marks the old certificate. The key is not touched, so `spki_sha256` is unchanged (facts: Settled, "The certificate can be reissued over the same key"; `KeyPair::from_pem` at `src/adapters/remote.rs:139,146`).
- **The reissued certificate** (decision 1). `reissue_certificate(key_pem, identity, names)` in `src/adapters/remote.rs` (next to `make_identity`, `:95`):
  - the same key from the Keychain PEM the listener was built with;
  - SANs from `names`, which are the LAN address from `local_address()` (`:962`) and `<LocalHostName>.local` (facts: Unresolved 2 and For the owner B: reissue with both, whatever the answers);
  - `extended_key_usages` = `serverAuth` (facts: Settled, Apple's rules);
  - validity under 825 days (`not_before` an hour back, `not_after` 824 days out), from facts: Unresolved 3;
  - the CN `Switchboard <server_id>` as now.
  The builder that sets these fields is a separate pure function (`certificate_params`), so tests inspect its fields without parsing DER and without a new dependency. Verify in rcgen 0.14.10 at implementation: that `CertificateParams::new` turns an IP-looking name into an IP SAN, and the names of the public fields. The registry is outside the sandbox, so this was not checked here.
- **Decision 1: the click-through is the trust step.** The first visit is already past the warning, so a downloaded certificate would be a second, manual step (on iOS, a profile and a trust toggle). The download would itself need the warning to fetch. It removes no warning and adds a step, so it is not built. What the click-through buys: a first visit whose TLS identity was accepted by the owner, on the owner's network. What it does not buy: a verified identity. An impostor on the network gets the same warning, and the owner has no key to compare. The dialog says so.
- **`identity.json` writes** go through the existing atomic path (`Effect::SaveIdentity`, `src/core/action.rs:734`; the arm in `src/app.rs:2893`).

## The signals

- **No new event and no change to the socket's frames.** The `changed`/`removed` stream is as the spec describes it (`docs/remote-client.md` §4.4).
- **The page polls REST** (decision 7): the list every 5 s, the open session every 3 s, only while the page is visible. Each reply's envelope `revision` is applied only if it is at or past the held revision for its key (spec §4.2 at `docs/remote-client.md:146`). A waiting session moves to the top on the next poll.
- **Origin check on the WebSocket upgrade** (owner question on Origin; default here). If an `Origin` header is present and is not `https://<Host>`, the connection thread answers 403 `forbidden_origin`. Native clients send no `Origin` and are unaffected. The page's origin is the host's own name, so it passes whatever the owner answers about IP or `.local`. A cross-site page cannot set the subprotocol token, so the default has no CSRF surface.
- **Token transports** (decision 2). The host reads three: `Authorization: Bearer` (native and REST), a `bearer.<token>` value in `Sec-WebSocket-Protocol` (the socket), and, behind the enhancement, the cookie. The 101 echoes the `bearer.<token>` value the browser offered. RFC 6455 §4.2.2 requires an echo of one offered token, and `bearer.` plus base64url is a legal token (facts: Settled, "WebSocket subprotocol"). The cookie is set on `/pair` as `Set-Cookie: switchboard-<server_id>=…; HttpOnly; Secure; SameSite=Strict; Path=/` and read on requests. The page ignores it unless the owner's D answer is favourable, in which case the constant `COOKIE_ONLY` in `web/app.js` flips to `true` and the page stops storing the token.
- **The page's storage** is `localStorage`, scoped to the origin (scheme, host and port; facts: Settled, "Cookies are not isolated by port"). The CSP has no inline script, no external source and no `eval`. The page renders server text with `textContent` only. What the trade-off costs: a script on the origin could read the token. The CSP keeps the origin's scripts to the page's own. The owner accepts that, or chooses the cookie.
- **No `/api/v1/` path is refused any differently.** `serve_remote` (`src/app.rs:3017-3025`) keeps its prefix match. The static paths are answered before it, on the connection thread (below).

## The actions and effects

- **Static paths: exactly `GET /` and `GET /app.js`** (decision 4). Both are answered on the connection thread in `connection` (`src/adapters/remote.rs:770`), after `read_request` and before the rate check and the queue. The page holds no core state, and the spec puts content work on the connection thread (`docs/remote-client.md` §4.1). Responses:
  - `/`: `Content-Type: text/html; charset=utf-8`; `/app.js`: `text/javascript; charset=utf-8`.
  - `Cache-Control: no-store` on both (no caching across versions).
  - `Content-Security-Policy: default-src 'none'; script-src 'self'; style-src 'sha256-…'; img-src 'self' data:; connect-src 'self' wss:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'`. The `style-src` hash is of the inline CSS, computed once with the existing `sha2` crate at first use. The CSS is inlined into `/` at serve time, so the page is two requests. `connect-src 'self' wss:` is a choice: `'self'` alone may not match `wss` in every browser, and the device run checks it.
  - `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`, `Connection: close`.
  - The favicon is `<link rel="icon" href="data:,">`, so no `/favicon.ico` request is made.
  - Any other GET is unchanged: refused on the frame as today.
- **Pools and windows** (decision 4). A page load is an unpaired stranger, so it uses the unauthenticated pool (`MAX_UNAUTHENTICATED`, `src/adapters/remote.rs:567`). `Connection: close` frees the slot as soon as the response is written, so two requests per load do not hold it for 30 s. No rate window: the bytes are constant, and a window would fail a reload. A test checks that a page load is answered while strangers hold their slots (`tests/remote.rs`, the tests section).
- **Pairing link.** `pairing_link(spki, code, server_id, host, port)` in `src/adapters/remote.rs`: `https://<host>:<port>/?code=<code>&key=<spki>&id=<server_id>`. IPv6 is bracketed. The host is the LAN address from `local_address()`, which is the route's source address, not necessarily the one a phone reaches. If there is no address, the window shows no QR and says so. The QR at `src/ui/remote.rs:160-166` encodes this text. The plan's host is the IP (decision in For the owner B: IP by default, `.local` only if the owner's answer makes it better).
- **Pairing in the page** (`web/app.js`):
  1. Read `location.search` (`code`, `key`, `id`), then call `history.replaceState(null, "", "/")` before any network call.
  2. Fetch `/api/v1/info`. If its `spki_sha256` differs from `key`, show "This is not the Mac whose code this is" and send nothing (the wrong-key path counts against the window only when the page posts, so it never posts).
  3. Post `/api/v1/pair` with `{code, spki_sha256: key, device_name}`. On success, store `{server_id, spki, token}` in `localStorage`, fetch `/api/v1/device`, and render.
  4. Map refusals: `wrong_code` says the code does not match and codes close after three tries; `pairing_closed` says to open the window again; `wrong_key` as in step 2.
- **Resumed pages**: with a stored token, `/info` is fetched first. A different `spki` gives "identity changed, pair again" and no request carries the token (`docs/remote-client.md:315`). Otherwise `/device` is fetched. A 401 gives "no longer paired", and the stored token is cleared.
- **Reply** (`POST sessions/{id}/send`, body `{op, text}`). The op is built from `crypto.getRandomValues`, not `crypto.randomUUID`, so it needs no secure context (facts: Unresolved 4 is avoided). On a network error, the page queries `/api/v1/ops/{op}` and follows spec §4.5 (`docs/remote-client.md:274-278`). It never resends blind.
- **`GET /api/v1/device`** (new, read scope, any paired device). A new `remote_device(&device)` in `src/app.rs` beside `remote_info` (`:3568`), returning `{device_id, name, scope, server_name}`. It is answered directly, with `authenticated: true`, not through the wire `Body` enum. The `DeviceView` type is in `control/src/lib.rs`, next to `SessionView` (`:637`).
- **Core changes** (`src/core/remote.rs`):
  - `CertificateNamesSeen(Vec<String>)` (the app sends the desired names on listen and on a rebind, as an action the core decides on): if the identity's names differ, its issue time is `None`, or it is more than 365 days old, emit `Effect::ReissueCertificate { names }`.
  - `IdentityReissued(Identity)`: replace the identity, keeping `server_id`, `server_name`, `listen` and `spki_sha256` (assert in the core test), emit `Effect::SaveIdentity`, and rebuild the listener with the new TLS config through the existing rebind path (`a_change_of_address_rebinds_the_port`, `tests/remote.rs:1494`).
- **Effects** (`src/core/action.rs`): `ReissueCertificate { names: Vec<String> }` runs in `run_remote_effect` (`src/app.rs:680`) and dispatches `IdentityReissued`. The fake remote store in `adapters/fakes.rs` needs no change, because `reissue_certificate` is a pure adapter function.
- **Cookie** (enhancement, behind the default, decision 2). `RemoteReply` gains `cookie: Option<String>`. `/pair` sets it. The 12 literal sites (10 in `app.rs` by grep, 2 in the adapter) take `cookie: None`. Mechanical.
- **`RemoteRequest` fields** (adapter-only; facts: Settled, "RemoteRequest is built only inside `src/adapters/remote.rs`"): `host`, `origin`, `protocols: Vec<String>`, `cookie: Option<String>`. `read_request` (`src/adapters/remote.rs:444-514`) fills them. If there is no `Authorization` header, the bearer is taken from a `bearer.` protocol value. No header is logged. `refuse` logs method and path only (`src/app.rs:3754-3772`), and the query is never logged.
- **Text pairing paths** (decision 3). The QR, the "Copy invite" text and the README's `switchboard-remote pair "$(pbpaste)"` (`README.md:265-278`) carry the link. Swift `PairingInvite.parse` (`client/Sources/SwitchboardProtocol/Info.swift:114-116`) tries the URL first (`URLComponents`, `code`, `key`, `id`), with `addresses` set to the one `host:port`, and falls back to JSON. The JSON stays only in the parser, for copies made before this change. Nothing emits it. `switchboard-remote pair` (`client/Sources/switchboard-remote/main.swift:12-13`) takes the link or the JSON.
- **Page states** the page shows in plain words: unpaired ("Open the pairing window on the Mac and scan its code, or open the link it shows"); identity changed; no longer paired; could not reach the Mac (this is also the text when the certificate warning was not accepted); pairing refusals.

## The UI

- **Pairing window** (`src/ui/remote.rs:141-210`): the QR encodes the link. Under it, the same link as text with "Copy invite". The text says: "Opening the link on a phone trusts this Mac's certificate for this address once you accept the browser's warning. The code works once, for five minutes or three wrong codes. The link may stay in the phone's history." The current "Scan, or type the key and the code" line keeps the key and code.
- **Paired devices dialog**: each row keeps scope, last seen, and Revoke. The dialog's sentence about pairing trust is the one above.
- **The page** (`web/index.html`, `web/app.js`, `web/app.css`): a header with the Mac's name, the device name and scope. Sessions grouped by project, waiting first, each waiting one marked "Waiting on you" with its reason; the list shows the card word and the name, with no excerpt (departure 1). A session view: the last response for a Claude Code session, the screen otherwise, polled every 3 s. A reply box that is shown only with `control` scope. The layout is one column, with 44-point touch targets, and it follows the system's light and dark setting.
- **Owner-facing wording** for the dialog and the page is in `docs/remote-client.md` §5 as amended.

## The tests

No browser UI test exists in the tree, and the issue rules out a JS dependency, so the page's logic is checked by hand. The host's behaviour is tested over the real TLS listener.

- **`src/adapters/remote.rs` unit tests**:
  - `certificate_params` carries the names, `serverAuth` and the bounded validity, and the reissued certificate keeps `spki_sha256` (the existing `debug_assert` at `:113`, and `spki_sha256_of_key` at `:138`).
  - `pairing_link` carries code, key and id, and brackets an IPv6 host.
  - `read_request` takes the bearer from the protocol value when there is no header, and fills `origin`, `host` and `protocols` (extend `a_request_reads_with_its_query_bearer_and_body`, `:1045`).
  - `asset` matches exactly `/` and `/app.js`, and nothing else (`/index.html`, `/app.js/`, `/api/v2` are refused).
- **Core** (`src/core/tests.rs`, dispatch with `Clock::at`):
  - a fresh identity whose names differ emits `ReissueCertificate` once, and not again when they match;
  - `IdentityReissued` keeps id, name, listen and SPKI, and emits `SaveIdentity`.
- **`tests/remote.rs`** (raw TLS, the existing `http` helper at `:245` extended to return headers, and `subscribe_at` at `:1960` extended to send headers):
  - `the_page_is_served_unpaired_with_its_headers`: `/` and `/app.js` return 200 with the content types, `Cache-Control: no-store`, `Connection: close` and the CSP named in the actions section, with no `unsafe-inline`;
  - `the_page_asks_only_for_paths_the_host_serves`: scan `web/app.js` and `web/index.html` for `/api/v1/` literals, replace `${…}` and `{…}` with a sample id, and send each with a paired token. None may answer with the message "not available on this port" (a 404 for an unknown id is fine);
  - `a_page_load_is_answered_while_strangers_hold_their_slots`: three idle unauthenticated connections, then `/` still answers;
  - `a_browser_pairs_from_the_link_and_reads_its_scope`: build the link with `pairing_link`, parse its query in the test, post `/pair` with `device_name` `Safari on iPhone`, then `GET /api/v1/device` returns the name and the scope;
  - `a_socket_takes_its_token_from_the_protocol_and_echoes_it`: an upgrade with `Sec-WebSocket-Protocol: bearer.<token>` and no `Authorization` gets 101 with the same value echoed;
  - `a_socket_from_another_origin_is_refused`: `Origin: https://evil.example` gets 403 `forbidden_origin`; `Origin` equal to `https://<Host>` gets 101; no `Origin` gets 101;
  - `a_revoked_device_is_refused_on_the_next_request`: after `RevokeDevice`, `/device` answers 401 `unauthorized` with the message the page maps to "no longer paired" (the existing revocation test at `:658` pins the same behaviour);
  - the existing `info_is_served_to_anyone_and_every_other_read_is_refused_unpaired` (`:426`), `the_allowlist_refuses_everything_else_with_no_side_effect` (`:682`, which uses `/api/v2`) and `pair_is_rate_limited_per_listener` (`:1406`) stay as they are.
  - A reissued certificate keeps the pin: a client with the old SPKI pin still verifies after `IdentityReissued` (`tls` pinned client at `src/adapters/remote.rs:168`).
- **Fixtures.** `DeviceView` is new and the Swift client does not decode it in this change, so no `client/Fixtures` file is added. If it does later, the rule applies.
- **Swift** (`client/Tests/SwitchboardProtocolTests/`): one new test file, `PairingInviteTests.swift`, parses the link, the JSON, and a link with an IPv6 host. This is the client's own test target; the implementer runs it with the client's usual command.
- **Manual, not gating the commit**:
  - Safari on the Mac opens `https://<lan address>:<port>/` (decision 1's click-through), reaches the page, pairs from the link, and sees the sessions. This is the one browser check the tree allows.
  - The phone run is the owner's (facts: For the owner A, B, C, D). It records, with the certificate as built, whether the page loads, `fetch` works, the socket connects, and the cookie works. Its answer chooses the cookie and the `.local` URL, and changes no default.
- **Pre-push**: `cargo test --locked --workspace`, `cargo clippy --locked --workspace --all-targets -- -D warnings`, `cargo fmt --all`, and the Linux clippy check for `dispatch` (CLAUDE.md). `include_str!` paths are relative to `src/adapters/`, so `../../web/…` is portable to CI.

## Docs and status

- **`docs/remote-client.md`**: §4.1 bounds gain the two static paths, the `Connection: close` rule and the Origin check (`:124-131`). §4.3 table gains `GET /`, `GET /app.js` and `GET /api/v1/device` (`:159-192`). Line 207 is amended: the excerpt is not on the session metadata today; the list shows the card word. §5: line 306 becomes "before anything but `/info`, `/pair`, and the page's two paths"; line 309 says the QR carries the link, and that a relayed pairing through a browser is not stopped by the key check; line 312 says the token travels as a bearer header, or in the `bearer.` subprotocol for a socket, and never in a URL; line 316 gains the trust sentence.
- **`docs/design.md`**: a new dated section, "Remote web page status (2026-10-22)", at the end of "Remote client (2026-10-12)", before "Versioning and updates (2026-10-15)" (`:3585`). It records what is built, the decisions, and the open facts (For the owner A–D, Unresolved 1–9) as "to run".
- **`README.md`**: the Layout table (`:369-446`) gains `web/` and `src/adapters/remote_page.rs`; the rows for `src/adapters/remote.rs` (`:383`), `src/ui/remote.rs` (`:423`), `src/core/remote.rs` (`:369`) and `tests/remote.rs` (`:437`) are updated. The Development section's remote text (`:265-278`) is updated to the link.
- **`CHANGELOG.md`**: one line under `[Unreleased]` (`:9`), for the user-visible change.

## Commit message

`Serve a browser page on the remote port and pair from a link`

## Contract additions

- `control::DeviceView` (new wire view, `control/src/lib.rs`): `{device_id, name, scope, server_name}`.
- `GET /api/v1/device` (read scope) and the static paths `GET /`, `GET /app.js` (`docs/remote-client.md` §4.3).
- `AppAction::CertificateNamesSeen(Vec<String>)` and `AppAction::IdentityReissued(Identity)` (`src/core/action.rs`).
- `Effect::ReissueCertificate { names: Vec<String> }` (`src/core/action.rs`).
- `ports::remote::Identity` gains `certificate_names: Vec<String>` and `certificate_issued: Option<SystemTime>`, both `#[serde(default)]` (`src/ports/remote.rs`).

Not contract, but changed: `RemoteRequest` gains `host`, `origin`, `protocols` and `cookie`; `RemoteReply` gains `cookie: Option<String>`. Both live in `adapters/`, so CLAUDE.md's additivity rule does not name them (facts: Settled, "RemoteRequest and RemoteReply live in adapters/"). No schema version changes.
