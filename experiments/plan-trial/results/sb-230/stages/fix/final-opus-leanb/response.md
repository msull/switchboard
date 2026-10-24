# Response to review round (head 9b56b17)

All fixes are in one commit, 2269bbd, `fixup!` of 8b32e5c. It renames
or removes nothing that 8b32e5c's message names. `cargo fmt --all`,
`cargo clippy --locked --workspace --all-targets -- -D warnings` and
`cargo test --locked --workspace` pass, and so does the pre-commit
hook. No `cfg(target_os)` arm was touched.

## feedback-1 (correctness)

1. **Visibility changes start extra polling chains.** Fixed.
   `poll()` in `web/app.js` now calls `clearTimeout(state.timer)`
   before it nulls the field. Any direct call, from the
   `visibilitychange` handler or anywhere else, therefore cancels the
   pending timer instead of orphaning it. Two overlapping `poll()`s
   (a visibility change during an in-flight poll) still end in one
   chain, because each finishes with `schedule()`, which clears the
   other's timer.

2. **The QR is cached on the code only.** Fixed. In `src/ui/remote.rs`,
   `RemoteDraft.qr` is now keyed on the payload string, and the
   texture is rebuilt whenever `invite_url` changes. A change to the
   LAN setting, the local address or the port now redraws the QR to
   match the Address line. "Copy invite" already copied the payload
   fresh each frame.

3. **The page branch returns from `connection`, skipping pool
   bookkeeping.** Checked, no change needed. `connection` never
   decrements a pool itself. The spawning closure in `RemotePort::bind`
   decrements after `connection` returns, by any path including the
   page branch's `return`. It reads the same `paired` flag that is set
   in the same place the counters move from unauthenticated to paired
   (`paired.store(true)` beside the `fetch_add`/`fetch_sub`). A paired
   connection that requests `GET /` is closed and released from the
   paired pool. A stranger is released from the unauthenticated pool.
   Closing a paired connection on a page path is what the plan asks
   for.

4. **An origin-failed cookie request is refused with
   `authenticated: false`.** Withdrawn by the reviewer. No change.

5. **A stale cookie with no stored host reports "Not paired" after
   Revoke.** No change, as the reviewer concluded. `/me` does not
   return `spki_sha256`, so the page cannot rebuild the stored
   identity that "removed" depends on without a wire change. The plan
   accepts this.

## feedback-2 (style)

1. **`token.clone()` in `remote_pair`.** Fixed. The reply now ends in
   two arms. The cookie arm adds `Set-Cookie`. The token arm moves the
   token into the body. There is no clone.

2. **The `too_many_lines` allow on `serve_remote`.** I kept the allow
   and reworded its comment. Without it clippy fails at 101/100 lines:
   the route table was at the limit, and the one-line `/me` arm pushed
   it over. The comment now says the table alone is about the lint's
   hundred lines, and that splitting it to get under would scatter the
   arms that decide what a device may reach.

3. **`respond`'s doc comment carries two ideas.** Fixed. The doc
   comment is now "Write one JSON response." The reason for the
   headers is a `//` comment above the `lines` initialiser.

4. **`Limits::limited` doesn't say what a path with no window does.**
   Fixed. Its doc now says only `/pair` and `/info` have a window
   there, and any other path is never over. The poisoned-lock sentence
   stays on `Limits::over`.

5. **`invite_url`'s loopback fallback.** Fixed. The doc now says no
   phone can reach the loopback, so a caller showing the invite must
   also say when `invite_address` found nothing. It cites the dialog's
   "port answers this Mac only" as the example.

6. **`state.sessions` is missing from the state literal.** Fixed. It
   starts as `sessions: {}`. Since the field now always exists, the
   `state.sessions && …` guard in `refreshSession` is down to
   `state.sessions[id]`.

7. **Long sentences in the `match` in `pairing`.** Fixed. They moved
   into `fn reach_text(lan, address) -> String` in `src/ui/remote.rs`,
   which `pairing` calls in one line.

8. **The node test skips silently.** Fixed. It prints "skipping the
   page's node tests: node is not installed" before returning, the
   same as the tmux tests' `eprintln!("skipping …")`.
