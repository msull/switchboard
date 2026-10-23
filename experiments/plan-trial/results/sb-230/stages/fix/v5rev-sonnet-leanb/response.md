# Response, round 1

STATUS: the edits are in the worktree but NOT committed. The disk filled up
(ENOSPC on the volume holding Claude Code's temp directory) before I could
commit, and the Bash tool refused to run. The last full `cargo test
--locked --workspace` showed one failing test binary (52 passed, 1 failed)
that I did not identify. `cargo test --test remote` passed alone (53) right
after, and `cargo fmt` and `cargo clippy -D warnings` were clean. The Linux
dispatch clippy check was not needed (no `cfg(target_os)` change).

## Feedback 1

1. Fixed (`web/app.js` `tick`, `drawSession`). When `/waiting` has not
   moved and a session is open, `tick` now re-reads the session's content
   route. That picks up text changes and renews the host's watch, so the
   socket keeps getting `changed` for it after the first minute.
2. Fixed (`drawSession`). If the screen already shows the same session
   under the same controls (scope and prompt refusal), only the `pre` text
   is updated in place. The reply box keeps focus and a pending send keeps
   its status node. A full rebuild happens only when the session or the
   controls change.
3. Fixed (`src/ui/remote.rs`). With network listening on and no address
   found, the window now says "No network address found: the code names
   this Mac only."

## Feedback 2

1. Fixed. `docs/remote-client.md` §5 now says the QR carries the invite
   URL, and the JSON form survives only as what `PairingInvite.parse`
   still accepts.
2. Fixed. README rows for `adapters/remote.rs` and `core/remote.rs` now
   mention the page window, request and reply headers, `invite_url`,
   `cookie_name` and `cookie_value`.
3. Fixed. `docs/design.md` gets one sentence on the weaker browser
   pairing (pointing at remote-client.md §5) and one on `PairingInvite.parse`
   taking the URL.
4. Fixed. `core::invite_url` now takes `Option<IpAddr>` and chooses
   loopback when `listen.lan` is false or no address was found. The UI
   only draws. The core test covers the LAN, Mac-only and no-address
   cases.
5. Fixed. `respond_with` was renamed `respond`, the one-line wrapper is
   gone, and a small `respond_error` covers the three
   connection-thread errors. Without it, `connection` went one line over
   the clippy limit. `Page` fields have doc lines.
6. Kept, with a better reason. The allow is still needed (clippy reports
   101/100 without it). It now says "the match is the route table; its arms
   are flat".
7. Fixed. `by_cookie` is `request.bearer.is_none() && token.is_some()`.
8. Not changed. This was noted for the other reviewer, and the docs
   already say the spike was not run.
9. Partly. The five `catch (e)` with an unused `e` are now bare `catch`.
   Splitting `start()` is not done: the reviewer called it not a blocker
   and it fits the no-build constraint.
