# Implement notes: #230 web control surface

Branch `stage/sb-230-implement-v5rev-opus-leanb2`, one commit as the plan says:

- `6909b3d` Serve a web control surface from the remote port, paired from a URL whose fragment carries the code

Not pushed, no PR. `git status` empty after the commit. Before it ran:
`cargo fmt --all`, `cargo clippy --locked --workspace --all-targets -- -D warnings`
(clean), `cargo test --locked --workspace` (22 binaries, all ok; tests/remote.rs
51 tests). `swift test --filter SwitchboardProtocolTests.FixturesTests` in
`client/` passed (5 tests). No `cfg(target_os)` code was touched, so the Linux
cross-clippy of `dispatch` was not needed. No schema version moved (SCHEMA_VERSION
16, VIEWS 7, IDENTITY 1 in this worktree, as the plan said).

## What was built

- `src/adapters/remote.rs`: `RemoteRequest.{cookie, origin, host}`;
  `RemoteReply.headers` + `RemoteReply::new`; `respond` takes extra headers and
  adds `Cache-Control: no-store` and `X-Content-Type-Options: nosniff` to every
  JSON reply; page requests (`GET /`, `/app.js`, `/app.css`, exact) answered in
  `connection` via `serve_page` with `Connection: close` and a TLS close_notify;
  `Limits.page` (`PAGE_PER_MINUTE` = 120); `token_cookie_name(server_id)` and
  `cookie_value(header, name)` (exact name match), unit tests for both and for the
  new request fields.
- `src/adapters/web.rs` (new): the `PAGES` table (`include_bytes!` of `web/`),
  `CSP`, `PAGE_HEADERS`, `page(path)`.
- `src/app.rs`: every `RemoteReply` literal converted to `RemoteReply::new`;
  `authenticate` takes the bearer, else the `__Host-sb-<server_id>` cookie; a
  cookie-only non-GET or upgrade must carry `Origin: https://<Host>` (403
  `origin`); `/pair` with `"transport": "cookie"` drops `token` from the body and
  sets `__Host-sb-<id>=<token>; Max-Age=34560000; Path=/; Secure; HttpOnly;
  SameSite=Strict`; `GET /api/v1/me` → `{device_id, name, scope, server_id,
  server_name}`, not in `capabilities`.
- `src/core/remote.rs`: `invite_url(identity, window, address: Option<IpAddr>,
  port)`, re-exported as `core::invite_url`. Loopback unless `listen.lan`; IPv6
  bracketed via `SocketAddr`.
- `src/ui/remote.rs`: QR and "Copy invite" carry the URL; the Mac-only line
  replaces the address when `listen.lan` is off; the browser trust sentence beside
  the redaction one; the QR cache keys on the invite text (it moves with the
  listen setting, not only the code).
- `web/index.html`, `web/app.css`, `web/app.js`, `web/app.test.js`: the page as
  the plan's UI section describes (fragment → `replaceState` → `/info` compare →
  `/pair` cookie; `/me` boot; `/info` inference on every 401; list with waiting
  first; last-response for `kind: claude`, screen otherwise; reply box only under
  `control` with `prompt_refusal` null; op kept for a retry; 3 s poll of
  `/waiting` by epoch/revision, socket as enhancement with 20 s poll while up and
  5 s→60 s retry). `textContent` only. The session view is built once and updated
  in place so a refresh does not wipe the reply box.
- Swift: `PairingInvite.parse` accepts the link (authority kept as written, so
  IPv6 keeps brackets) as well as the JSON; `InviteError.incomplete`; a fixture
  test with the link, the JSON, IPv6 and a missing code.
- Tests: core `the_invite_is_a_url_with_the_code_in_its_fragment`; UI test
  rewritten for the URL and the Mac-only notice; tests/remote.rs: page files and
  headers, the guard test (page names only served paths, every `/api/v1/` string
  in app.js is a route for a paired device, no inline script/style/handlers, no
  innerHTML/eval/document.write), cookie pairing + two hosts + fixed-name cookie,
  `Origin` rule including upgrades (`subscribe_at` now goes through
  `subscribe_with`, which takes headers and returns a Result), `/me` + revoke +
  `/info` unchanged, rotated identity's `/info` key (added to the existing
  `:1423` test), pool (four held page connections see EOF, a fifth loads; the
  stalled-strangers test now also checks a page load is closed at accept) and the
  121st page request 429 with `/info` and `/pair` unaffected, and a wrapper that
  runs `node --test web/app.test.js` and skips when `node` is absent (it ran here:
  no skip message, passed).
- Docs: remote-client.md §0 dated line, §4.1 page bound, §4.3 rows for `/me`,
  the page paths and `/pair`'s `transport`, the excerpt line (now says it is not
  on the port), §5 pre-pairing surface, fragment invite, cookie/Origin rule, and a
  bullet on the browser's weaker trust; design.md dated paragraph and known gaps;
  README layout rows (`web/`, `web.rs`, spike, dialog, core remote, tests) and the
  dev-aid sentence; CHANGELOG Unreleased line as the plan worded it;
  `spikes/README.md` row.

## Fallbacks the plan named, built as said (facts not settled here)

- **Spike 23 not run** (plan Order of work step 1 names this fallback). No browser
  could be driven: keystroke injection is forbidden, and `/Applications`,
  `openssl` and `python3` were not available to this session. I wrote
  `spikes/23-web-page/` (a Python HTTPS server, a page that sets and reads
  `__Host-sb-<id>` and probes `wss:` under the CSP, `.gitignore` for the pems) and
  a README that says "not run" with the commands. The kit itself has not been
  executed either. Unresolved 1, 3, 6 and the CSP `wss:` question stay open;
  acceptance steps 1 to 4 rest on the owner's phone.
- **Cookie transport (Unresolved 3)**: built the cookie path; no fallback
  transport built, per the plan.
- **Certificate (Unresolved 1/2)**: unchanged; click-through is the trust step,
  the dialog sentence promises nothing about which browsers allow it.
- **Unresolved 4** (`replaceState` history): page calls it first; nothing more.
- **Unresolved 6** (connections per load): `Connection: close` decided without
  waiting on it; the "five parallel connections against the harness" measurement
  was not done as a separate run. The tests cover four held + a fifth and the
  full-pool case, which is what the harness can say without a browser.

## Where the plan was off or I deviated

- "About twenty literals in `src/app.rs`": there were nine in app.rs plus one in
  `connection` (facts' count was also high). Converted all.
- `RemoteRequest` literal: the plan says `remote.rs:506`; it was 506 in
  `read_request`, as facts said (`:360` is the struct definition). Fine.
- Line numbers in docs/remote-client.md §5 (306/312/319): 319 is the
  "loopback-only without auth" line, which needed no change; I amended the
  pairing-surface bullet (306), the QR bullet (309) and the token bullet (312),
  and added a browser-trust bullet.
- `serve_remote` grew past clippy's 100-line limit with the `/me` arm (it was at
  exactly 100). I added `#[allow(clippy::too_many_lines)] // one arm per route`,
  the idiom already used for the core's command tables, rather than reshaping
  the route table. `connection` also crossed it; I pulled the page branch into
  `serve_page` and the 429 into `respond_limited` instead.
- Device name: the plan lists `Edg`, `Chrome`, `Firefox`, else `Safari`. iOS
  Chrome and Firefox say `CriOS` / `FxiOS` (and Edge `EdgiOS`/`EdgA`), which the
  plan's list would call "Safari"; I matched those too. Small, noted here.
- The plan's guard test reads "every `/api/v1/...` string in app.js answers
  something other than `not_found`". A route can legitimately answer `not_found`
  for a missing record ("no such …"), so the test asserts the reply is not the
  router's own 404 (messages "no such endpoint" / "not available on this port").
- `/info` inference: the page also stores `{server_id, spki_sha256,
  server_name}` when `/me` succeeds with nothing (or another host) stored, so a
  cleared `localStorage` still gets the right 401 wording later. Not in the plan;
  harmless.
- `TOKEN_COOKIE_MAX_AGE` (34_560_000 s = 400 days) is a const in app.rs; the
  plan gave only the number.
- Contract additions in the commit include the public adapter items the plan's
  list did not name: `adapters::web::{PAGES, CSP, PAGE_HEADERS, page}` and
  `adapters::remote::{token_cookie_name, cookie_value, PAGE_PER_MINUTE}`.

## Left for the owner (unchanged from the plan)

Cookie fallback if a browser refuses the cookie; a trusted-certificate path; the
one-line excerpt on the wire; the phone checks (scan, proceed past the warning,
`changed` on iOS Safari, reachability, "removed on the host" after Revoke).
