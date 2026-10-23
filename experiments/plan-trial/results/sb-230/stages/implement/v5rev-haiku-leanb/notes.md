# sb-230 implement, v5rev-haiku-leanb: notes

Branch `stage/sb-230-implement-v5rev-haiku-leanb`, worktree
`/Users/sully/code_repos/plan-trial/sb-230-stage-implement-v5rev-haiku-leanb`.
Base `7fe8949` (Release 0.4.1). Not pushed; no pull request.

## Commit

- `f693952` Serve a web control surface from the remote port, paired from a
  URL whose fragment carries the code. One commit, as the plan's order of
  work says. The message is the plan's title plus its contract-additions
  list. No attribution lines. `git status` is empty after the commit.

## What was built

Host (`src/adapters/remote.rs`, `src/adapters/web.rs`, `src/app.rs`,
`src/core/remote.rs`, `src/core/mod.rs`):

- `RemoteReply` gains `headers` and `RemoteReply::new`; the eleven struct
  literals convert to `new`. `RemoteRequest` gains `cookie`, `origin`, `host`
  (parsed in `read_request`, the only literal).
- `respond` writes the reply's headers and adds `Cache-Control: no-store` and
  `X-Content-Type-Options: nosniff` to every JSON reply.
- Pages: `GET /`, `/app.js`, `/app.css` answered on the connection thread from
  the `web.rs` table (`include_str!` of `web/`), with the CSP, `no-store`,
  `nosniff`, `Referrer-Policy: no-referrer` and `Connection: close`; the
  connection ends after the reply, so the pool slot frees at once. A new
  `Limits.page` window gives 120 page loads a minute per listener.
- `authenticate`: the bearer header, else the cookie named
  `__Host-sb-<server_id>` matched by exact name. A cookie-only request that is
  not a `GET` or is an upgrade must carry `Origin: https://<Host>`, else 403
  `origin`.
- `/pair` takes `transport: "cookie"`: the reply carries `Set-Cookie` (Max-Age
  400 days, `Path=/`, `Secure`, `HttpOnly`, `SameSite=Strict`, no `Domain`) and
  its body names no token. Without it the reply is unchanged.
- `GET /api/v1/me` (read scope): `device_id`, `name`, `scope`, `server_id`,
  `server_name`.
- `core::remote::invite_url(identity, window, host: IpAddr, port)`: the
  fragment URL `https://<host>:<port>/#code=…&key=…&id=…`; IPv6 is bracketed.
  `core::remote` is now `pub mod` so the dialog can call it.

Dialog (`src/ui/remote.rs`): the QR and "Copy invite" carry the URL. The URL
names the LAN address only when `listen.lan` is on and a local address exists;
otherwise loopback, with a "This Mac only" line in place of the address. A
second sentence under the redaction note says a browser cannot pin the key,
proceed past the warning only when the address is this Mac's, pair a browser
only on a network you trust, and the code works once.

Page (`web/index.html`, `web/app.css`, `web/app.js`, `web/app.test.js`): plain
DOM with `textContent` only; no inline script, style or handler. Boot reads the
fragment, strips it with `replaceState`, checks `/info` against the key and id
before any code is sent, pairs with `transport: "cookie"`, and keeps only
`{server_id, spki_sha256, server_name}` in `localStorage`. List and session
views; a reply older than the one held is dropped by revision; the socket is
an enhancement over polling, with a close treated as a reason to read again
and retry after 5 s doubling to 60 s; a 401 runs the `/info` inference.
Operation ids are kept across a retry of an unanswered send.

Swift (`client/Sources/SwitchboardProtocol/Info.swift`,
`client/Tests/.../FixturesTests.swift`): `PairingInvite.parse` reads the URL
form and still reads the JSON. A URL without `code` throws.

Tests:

- `tests/remote.rs`: 11 new tests, plus the stalled-strangers test extended
  with a page load. They cover the page headers and 404s, the no-inline and
  every-route-answers guards, the cookie pairing flow, two hosts on one
  address, the origin rule for POST and for the upgrade, `/me` after a scope
  change, revocation with `/info`, the page pool slot, the page window and the
  node runner.
- `src/core/tests.rs`: `invite_url` (IPv4, loopback, IPv6, no query).
- `src/adapters/remote.rs`: cookie matching, the `Set-Cookie` form, the request
  parse of cookie/origin/host, the reply headers.
- `tests/ui.rs`: the invite assertions and the "This Mac only" label.
- Swift: the URL parse and the missing-code error.

Docs: `docs/remote-client.md` (§0 dated line, §4.1 pages and the window, §4.3
rows for the page files and `/me`, the `pair` row, the excerpt sentence at
the `session.last_response` bullet, §5 the pre-pairing surface, the invite,
the browser's trust and the cookie); `docs/design.md` a dated paragraph before
`## Versioning and updates`; `README.md` (dev-aid sentence, Layout rows for
`web/` and `web.rs`); `CHANGELOG.md` Unreleased line; `spikes/README.md` row;
`spikes/23-web-page/README.md`.

## Verification

- `cargo fmt --all -- --check`: clean.
- `cargo clippy --locked --workspace --all-targets -- -D warnings`: clean.
- `cargo test --locked --workspace`: every test binary green after the final
  change (the lib, the 481-test binary, the lib-test binary with 535, the
  remote binary with 55, and the rest). The baseline before the change was
  green too.
- `tests/remote.rs`: 55 pass. The node runner ran as part of this (node's
  own summary printed); see the gap below.
- `swift build --build-tests` and `swift test --filter SwitchboardProtocolTests`
  in `client/`: 6 tests, 0 failures, including the two new invite tests.
- Linux clippy for the `dispatch` crate was not run: no `cfg(target_os)` arm
  or platform-only code is touched by this change.

## Not verified, and why

- **Browsers.** No browser was driven. Unresolved 1 (click-through), 2 (wss
  to a self-signed certificate on iOS), 3 (the cookie on an IP host) and 5
  (camera and Local Network) are device checks; `spikes/23-web-page/README.md`
  lists the steps. The page's DOM code (boot, list, session, send) has no
  automated test: only its pure functions are tested, and the host routes it
  calls are tested by the "every route answers" test.
- **`node --test web/app.test.js` run directly** was blocked by a permission
  prompt in this session. The same file ran through `cargo test` (the
  `the_page_logic_passes_its_node_tests` test spawns `node --test`), and node
  printed its pass summary, so this is a gap in how I ran it, not in the file.
- **Five connections (Unresolved 6).** Measured against the harness only:
  four unauthenticated slots, the fifth closed at accept, and a page frees its
  slot on close. What a browser opens is unmeasured.

## Where the plan was wrong or needed a decision

1. "About twenty literals in `src/app.rs` plus the one in `connection`": there
   are ten literals in `app.rs` and one in `connection`, eleven in all. The
   grep for `RemoteReply {` shows thirty lines because it also matches the
   `-> RemoteReply {` signatures.
2. `PairingInvite.parse` did not accept the URL. The plan said it did, and it
   is JSON-only in the tree. The URL parse is new code in this commit.
3. `subscribe_at` (`tests/remote.rs`) does not take extra headers, as the
   plan's test list assumes. The upgrade-origin test uses a new head-only
   helper (`head_with`) and a raw upgrade request instead.
4. Clippy: `serve_remote` passed 100 lines with the new `/me` arm. I kept the
   arm and added `#[allow(clippy::too_many_lines)] // one arm per route`, the
   same allow the codebase uses on other route and command tables. `connection`
   also went over, and I cut it with `respond_error` rather than an allow.
5. `core::remote` was a private module. The UI must call `invite_url`, so it
   is `pub mod` now. The plan named the path but did not say it had to be
   public.
6. The spike: the plan asked for a throwaway HTTPS server. I did not write one,
   because the build's own host serves the page over `make_identity`'s
   certificate, which is what the spike was to test. The spike README says so
   and gives the device steps. No browser result is recorded.
7. The plan's `invite_url(identity, window, address, port)` left `address`'s
   type open. It is `IpAddr`, which makes the IPv6 bracket a single rule.
8. The plan's "Mac-only" branch covered `listen.lan` off only. With `listen.lan`
   on and no local address (`local_address()` is `None`), the URL falls back to
   loopback, and the dialog says the Mac has no local network address. The plan
   did not cover that case.
9. The authenticate 403 `origin` refusal is `authenticated: false`. The plan was
   silent. A refused write never moves a connection into the paired pool.
10. A Mac's device name for a user agent with no recognised device is the bare
    word "Browser", the plan's literal reading. The browser name is dropped
    then.
11. The plan's Swift "parse" note assumed a `Wire.decode` path. The URL path
    uses string splitting and throws `DecodingError.dataCorrupted`, so no new
    public type appears in the contract list.
12. `docs/remote-client.md` line 207 is a sentence in the `session.last_response`
    bullet, not a line number I could rely on after edits. The plan's "Remote
    client (2026-10-12)" heading in `design.md` exists at `:3107`, and the new
    paragraph sits before `## Versioning and updates`, as the plan said.

## Left for the owner (from the plan, unchanged)

- The cookie fallback (the page holding the token with the
  `Sec-WebSocket-Protocol` echo) if a device refuses the cookie. Not built.
- A trusted-certificate path (IP subject alternative names, a download, or a
  local CA) if the click-through is not offered, persists, or Safari will not
  open `wss://` to the certificate. Not built.
- The card excerpt on the wire (`src/ui/cards.rs` into the core). Not built.
- The device checks in `spikes/23-web-page/README.md`.

## Recorded facts (for the next stage)

- Commit `f693952`, branch `stage/sb-230-implement-v5rev-haiku-leanb`, not
  pushed.
- No schema version changed: `SCHEMA_VERSION` 16, `VIEWS_SCHEMA_VERSION` 7,
  `IDENTITY_SCHEMA_VERSION` 1, all read in the worktree before the change. No
  `RECORD_VERSION` exists in this tree.
- No client or customer name appears in the code, tests, docs or commit.
