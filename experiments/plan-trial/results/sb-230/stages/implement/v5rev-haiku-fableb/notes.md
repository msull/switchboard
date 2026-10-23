# Implement notes: #230 web control surface on the remote port (v5rev-haiku-fableb)

## Result

- Branch `stage/sb-230-implement-v5rev-haiku-fableb`, one commit:
  `dc1c58a Remote port serves a browser page paired from the invite URL`.
- Not pushed, no pull request. `git status` is empty after the commit.
- The commit message is the plan's title and its contract-additions list,
  with no attribution lines.
- No serialized type changed (`Identity`, `PairedDevice`, the store), so
  `SCHEMA_VERSION` stays at 16 (`src/core/model.rs:12`) and no `migrate`
  step was written. I did not find a `RECORD_VERSION` constant under
  `src/`, and nothing that needed one changed.

## What was built

- Core (`src/core/remote.rs`): `token_cookie_name(&Identity)` and
  `pairing_invite(&Identity, &str, &str)`. The invite brackets an IPv6
  address itself. `core::remote` is now `pub mod` (see deviations).
- Adapter (`src/adapters/remote.rs`): `RemoteRequest::cookies` (only
  `switchboard_token_*` pairs), `RemoteReply::headers`, and `respond`
  takes the whole reply, writes headers after `Content-Length`, and
  defaults `Content-Type` to JSON. A `PAGE_PER_MINUTE = 120` window
  covers `/`, `/app.css` and `/app.js`, answered 429 before the frame.
  The connection thread's own errors go through `error_reply`.
- Frame (`src/app.rs`):
  - `page_reply` serves the three files with CSP, `no-store` and
    `nosniff`. It is checked at the call site in `serve_pending`, not in
    `serve_remote`, which would otherwise pass the 100-line lint.
  - `authenticate` tries the bearer, then this host's cookie. A 401 over
    this host's own cookie clears it. Another host's cookie is never
    touched.
  - `remote_pair` with `transport: "cookie"` omits the token from the
    body and sets `Set-Cookie: switchboard_token_<server_id>=…; Path=/api/v1/;
    Secure; HttpOnly; SameSite=Strict`.
  - `RemoteReply` literals gained `headers: Vec::new()`.
- Page (`src/web.rs`, `web/index.html`, `web/app.css`, `web/app.js`):
  plain DOM, no inline script or style, and no `http(s)://` in any file.
  The page reads the invite fragment and replaces the URL before any
  request. It pairs, stores only `server_id`, `spki_sha256` and the
  granted scope in `localStorage`, and never reads a token. It lists
  sessions grouped by project, waiting first. It opens the last response
  for a `claude` session and the screen otherwise. It sends `watch` and
  `unwatch` on the socket, and polls every five seconds while the socket
  is down. It retries the socket every 30 s. Replies go out as
  `POST …/send` with a fresh `op`.
- UI (`src/ui/remote.rs`): the QR and "Copy invite" carry the invite URL.
  Without an address there is no QR and no copy button. The dialog shows
  the code and the full key, with the three-argument `switchboard-remote
  pair` sentence.
- Swift (`client/Sources/SwitchboardProtocol/Info.swift`):
  `PairingInvite.parse` takes the URL form and keeps the JSON path. It
  trims surrounding whitespace first.
- Tests:
  - `src/core/tests.rs`: invite shape, IPv4 and bracketed IPv6, the
    token never in the invite, and per-host cookie names.
  - `tests/remote.rs`: four new tests. `the_page_is_served_unpaired_with_its_headers`,
    `the_page_references_only_what_the_host_serves`,
    `a_browser_pairs_from_the_invite_and_reads_through_its_cookie` (covers
    the bearer-over-cookie order, another host's cookie, revocation
    clearing and the closed socket), `a_page_load_fits_the_stranger_pool`,
    and `the_page_window_is_rate_limited`. The fixture test now also
    checks `invite.txt` against `info.json`.
  - `tests/ui.rs`: the clipboard assertion parses the URL.
  - `client/Tests/.../FixturesTests.swift`: `testInviteParsesTheURL` and
    `testInviteKeepsTheBracketsOfAnIPv6URL`.
- Docs, in the same commit: `docs/remote-client.md` (§4.1 bounds, §4.3
  rows, 207 wording, §5 lines, a "Browser trust" paragraph and the cookie
  clause, §13), `docs/design.md` (the web-page bullet, the known-gaps
  bullet, and the status line that still described the JSON QR),
  `README.md` (dev-aid paragraph, rows for `src/web.rs` and `web/`, and
  the amended rows), `CHANGELOG.md` (Unreleased line).

## Checks run

- `cargo fmt --all -- --check`: clean.
- `cargo clippy --locked --workspace --all-targets -- -D warnings`: clean
  (exit 0). It flagged two things, both fixed: the 103-line `serve_remote`
  and the test-body length. The test body got an `#[allow(clippy::too_many_lines)]`
  with a reason, as `write_fixtures` does.
- `cargo test --locked --workspace`: all pass. The remote suite is 49 of
  49, including the four new tests.
- `swift test --package-path client`: 83 of 83 pass, including the three
  invite tests.
- Not run: the dispatch crate's Linux clippy. The change touches no
  `cfg(target_os)` arm and nothing that compiles on one platform only.

## Not verified (no way to run it here)

- **The page's script has never run.** `node --check` was refused by the
  sandbox, and no test drives the page in a browser. The Rust tests check
  the files' policy and the server's replies, not the page's behaviour.
  A syntax error or a wrong DOM id would show only on a real browser.
- **The Decision 2 check was not done.** The plan says the first thing to
  test is whether a browser stores the `Secure` cookie on a
  bypassed-certificate origin and sends it on `fetch` and the socket. I
  had no phone, and the plan's order was not followed: the page was
  built before that check. If the cookie is dropped, the page shows the
  "did not keep its pairing. Pair again." state, as the plan asks. The
  next step is the `Sec-WebSocket-Protocol` follow-up.
- The acceptance steps on one phone: the iPhone Camera opening the URL,
  the bypass across tabs and relaunches, the socket on iOS, and the QR
  readability at 49 modules.

## Where the plan was wrong or unclear

1. **Literal count.** The plan says 28 `RemoteReply` literals in
   `src/app.rs`. There are 10, plus 2 in the adapter, one of which is
   replaced by `error_reply`.
2. **Module path.** The plan's `core::remote::…` paths do not resolve:
   `core::remote` was a private module. I made it `pub mod`. The plan's
   contract list does not mention that change, so the commit message
   says it.
3. **The identity check blocks re-pairing after a key rotation.** Plan
   step 2 says: if the stored `server_id` matches and the stored
   `spki_sha256` differs, show "identity changed" and send nothing else.
   That also blocks a fresh invite, so a rotated host could never be
   paired from the page. I let a fresh invite pair when its key matches
   `/info`. The stored-key check applies only without an invite. This is
   a deviation; the owner should confirm it.
4. **Scope storage.** The plan says the page keeps only the two public
   ids, yet also refers to "the stored scope" for the control reply box.
   I store the granted scope under `switchboard.scope`, as public as the
   others. The page can know the scope after a reload only this way.
5. **Bearer versus cookie fallback.** The plan says "bearer first, failing
   that, the cookie". I read that as: an unknown bearer falls back to a
   known cookie. The plan does not say which.
6. **The `http` test helper.** The plan says it "gains a cookie argument
   and returns the headers". Its four callers changed, and a
   `call_text`/`call_with` split and a `read_response` helper were added.
7. **Favicon.** Browsers request `/favicon.ico` by default. That is an
   unauthenticated `404` on a fifth connection, which the plan's pool
   count does not include. The page has no favicon link, because the
   plan's href test forbids one. This is not settled. It may be
   harmless: the connection closes quickly.
8. **Page refetch.** The plan asks for per-resource revision
   reconciliation against the subscribe-first buffer. The page keeps one
   list revision and refetches the lists when an event passes it, with
   no per-resource buffer. The page's gap-free guarantee is weaker for
   that reason. The plan's text describes the more complete version.
9. **Dialog without an address.** The plan says "the code and the key".
   The short hash cannot feed the CLI's three-argument form, so the
   dialog shows the full key in that case.
10. **`invite.txt`.** The `SWITCHBOARD_WRITE_FIXTURES=1` run was refused
    by the sandbox. I wrote `client/Fixtures/invite.txt` by hand from
    `info.json`. The fixture's `server_id` and `spki_sha256` come from the
    same fixed identity, and the decode test checks the file against
    `info.json`. The file is what `write_fixtures` would write, with its
    trailing newline; the test's `pairing_invite` path itself was not
    run.
11. **Page-rate scope.** `PAGE_PATHS` rate-limits any method on the three
    paths, so a `POST /` also counts. That is harmless but not stated in
    the plan.

## Not done (outside this stage)

- No push, no PR, as asked.
- Follow-ups the plan lists for the owner stay open: the IP SAN or a
  CA-plus-leaf identity, the `Sec-WebSocket-Protocol` transport (only if
  the cookie check fails), and the dispatch tickets on the page.
- The card excerpt on the wire, the iOS socket and the single-address
  gap are recorded in the design doc's known-gaps bullet.
