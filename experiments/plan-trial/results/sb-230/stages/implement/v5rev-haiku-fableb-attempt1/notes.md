# Notes: implement #230 on stage/sb-230-implement-v5rev-haiku-fableb (v5rev-haiku-fableb)

## Status: NOT COMMITTED, NOT VERIFIED. Stopped on a full disk.

The root volume (APFS, shared container) filled up during the work. `/tmp/claude-501`
is on it, so Bash cannot capture output and the Edit/Write tools fail with ENOSPC on
their temp files. Nothing was deleted to make room: the volume is the user's machine,
and freeing it is their call. The worktree is dirty (partial implementation), so the
stage fails until it is finished and committed.

Measured while the disk was low (about 200-270 MiB free on the container):
- worktree `target/`: 4.8 GB, created by this session's baseline build (it was not in
  the worktree when the session started). Regenerable.
- `~/.cargo/registry`: 1.2 GB (the user's).
- `~/Library/Caches`: 16 GB (the user's).

Baseline before any change: `cargo test --locked --workspace` passed every suite
(log at `target/sb230/baseline.log`).

The last `cargo check --locked --workspace --all-targets` ran the disk to zero and
failed in dependency builds (ENOSPC on rmeta, dep-info and the linker). It did not get
to the workspace crate, so nothing about the new code is compiled yet.

## What is built (uncommitted, in the worktree)

Done and written, not compiled:
- `src/core/remote.rs`: `TOKEN_COOKIE_PREFIX`, `token_cookie_name`, `pairing_invite`.
- `src/core/mod.rs`: `mod remote` is now `pub mod remote` (the plan's path is
  `core::remote::...`, and the app needs it).
- `src/core/tests.rs`: `the_invite_is_a_url_...`, `the_cookie_name_is_per_host_...`.
- `src/adapters/remote.rs`: `RemoteRequest::cookies`, `RemoteReply::headers`,
  `respond(w, &RemoteReply)` writing headers after Content-Length with a JSON default
  Content-Type; `error_reply`; `PAGE_PER_MINUTE = 120` window; cookie-header test.
- `src/web.rs` (new): `INDEX_HTML`, `APP_CSS`, `APP_JS`, `lookup(path)`.
- `src/lib.rs`: `pub mod web;`.
- `src/app.rs`: GET page routing ahead of the `/api/v1/` prefix with `page()` (CSP,
  no-store, nosniff); `authenticate` takes the bearer, else this host's cookie, and
  sends the clearing Set-Cookie on a 401 for an unknown cookie token; `remote_pair`
  with `transport: "cookie"` omits `token` and sets `Set-Cookie`; `token_cookie_clear`.
  The 11 `RemoteReply` literals have `headers: Vec::new()`.
- `src/ui/remote.rs`: invite is the URL (QR and Copy invite); no-address branch shows
  the CLI form and no QR; sentence per plan.
- `web/index.html`, `web/app.css`, `web/app.js` (all three written; app.js was not
  yet syntax-checked).
- `client/Sources/SwitchboardProtocol/Info.swift`: `PairingInvite.parse` takes the URL
  form (authority as text, fragment split on `&`), JSON as before.
- `client/Tests/SwitchboardProtocolTests/FixturesTests.swift`: `testInviteParsesTheURL`
  (reads `client/Fixtures/invite.txt`), IPv6 bracket test, refusal test.
- `tests/remote.rs`: `Response`, `http_full`/`exchange`/`pinned`/`Kept`, `fetch`, `send_on`,
  `pair_browser`, `subscribe_with`; new tests `the_page_is_served_unpaired_with_its_headers`,
  `the_page_references_only_what_the_host_serves`,
  `a_browser_pairs_from_the_invite_and_reads_through_its_cookie`,
  `a_page_load_fits_the_stranger_pool`, `the_page_window_is_rate_limited`; fixture
  writer writes `invite.txt`; decode test checks it equals `pairing_invite` over the
  fixture identity.
- `tests/ui.rs`: the clipboard assertion parses the URL (fragment has code and key).

NOT done:
- `client/Fixtures/invite.txt` is not written. Its writer is in `tests/remote.rs`
  `write_fixtures`, which runs only with `SWITCHBOARD_WRITE_FIXTURES=1`. Running it
  rewrites every client fixture with new random ids (the repo's normal churn). It
  needs disk and a build.
- Docs: `docs/remote-client.md` (none of the edits applied: the disk filled on the
  first one), `docs/design.md` (web page bullet, known-gaps), `README.md` (Layout rows
  for `src/web.rs` and `web/`, the dev-aid paragraph), `CHANGELOG.md` (Unreleased line).
- Verification: fmt, clippy (`-D warnings`), the workspace tests, the Swift tests
  (`client/`), and the dispatch-crate Linux clippy (not needed: no cfg(target_os)
  change in this set, but check `src/adapters/remote.rs` and `src/web.rs` on Linux
  anyway).
- Commit with the plan's message and contract list.

## How to finish (after space is freed)

1. Free space. The fastest reclaim that is mine: `cargo clean` in the worktree
   (4.8 GB). The user's caches are theirs to decide.
2. `cargo fmt --all`; `cargo clippy --locked --workspace --all-targets -- -D warnings`;
   `cargo test --locked --workspace`.
3. `SWITCHBOARD_WRITE_FIXTURES=1 cargo test --locked --test remote the_client_fixtures_decode_on_both_sides`,
   then the normal run; check the diff of `client/Fixtures` is only the churn plus `invite.txt`.
4. `cd client && swift test` (the parser's tests).
5. Docs (remote-client §4.1, §4.3, §5 lines 306/309/312, §13 Authentication row; design
   status; README; CHANGELOG), then commit.

## Plan points that differ from the tree

- The plan says 28 `RemoteReply` literals in `app.rs`; the tree has 11 literals. The
  rest are `-> RemoteReply {` signatures. Each literal got `headers: Vec::new()`.
- The adapter's `respond` callers build a `RemoteReply` via a new `error_reply` helper
  (3 error sites), not `(status, body)`.
- `core::remote` had to become `pub mod` (the plan's path implies it).
- `pairing_invite` takes `host:port` already formatted by the caller
  (`SocketAddr::new(ip, port).to_string()`, which brackets IPv6). "IPv6 bracketed" is
  satisfied by the caller.
- The no-address sentence fills in the code and the full key (the plan's sentence has
  `<code> <key>` placeholders). The full key is shown nowhere else in the dialog.
- The UI test's clipboard check needs a default route (no route, no address, no
  "Copy invite"). That was already true of the QR's address; now the button depends on it.
- The page's send does not resend after a silence (the plan asks for a status query;
  not built: the page asks the owner to check the session).

## Unresolved facts (the plan's fallbacks apply; nothing settled here)

- Secure cookie on a bypassed-certificate origin: the plan wants the phone test first.
  Not done: no device here. The page shows "did not keep its pairing" when the first
  authenticated call after `/pair` is refused.
- iOS Safari and `wss://` under the bypass: the page polls by default, and the socket
  is used when it connects.
- Retries of a dropped page-load connection: the plan's connection count is built as stated.
- Persistence of the bypass across tabs and relaunches: the dialog says the warning may
  return; nothing else depends on it.
- QR camera behaviour with a URL, and VPN default routes: for the owner.
