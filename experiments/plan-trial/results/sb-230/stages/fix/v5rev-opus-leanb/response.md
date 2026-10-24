# Response to round 1

All fixes are in one commit, 538a865 (`squash!` onto 4b3a20c, since it adds `core::remote::invite_address` to the contract list). `cargo fmt --all`, `cargo clippy --locked --workspace --all-targets -- -D warnings` and `cargo test --locked --workspace` all pass. `node --test web/app.test.js` was not run because running node was not permitted in this session. The pure functions it covers did not change. No `cfg(target_os)` arm was touched, so the Linux dispatch cross-check was not needed.

## Correctness review

1. **A refused cookie shows as "removed": fixed.** `pair()` now calls `/me` before it writes to `localStorage`.
   - If that `/me` returns 200, the page stores the host and moves on to the shared `enter()`. `start()` now does its own `/me` and then calls `enter()` too.
   - If it returns 401, the page shows the new `cookieRefused` notice: "Paired, but this browser did not keep the sign-in", with a line telling the user to remove the device on the Mac and try another browser.
   - The spike 23 README sentence that said "Not paired" now describes this behaviour.

2. **The page sticks on an error notice or shows a blank screen: fixed.**
   - A new `showCurrent()` shows the open session if there is one, otherwise the list. `drawList` now calls it whether or not a session is open.
   - `poll()` also calls it when `/waiting` answers and nothing has changed. That case matters because the mark stays the same after a network drop, so `refreshList` never runs and could not clear the notice.
   - `failed()` now shows "The host answered with an error" for an `HttpError` that is not a 401, but only before the list has first been drawn (`!state.listMark`). This covers a 429 or 503 from `/me` at start, and a failed first list load. After the list has been drawn, a passing error still leaves the current view up and the next poll tries again.

3. **The QR names loopback with no explanation: fixed.** The dialog now picks one of three lines:
   - "Address …" when there is an address.
   - The existing Mac-only line when the port is not on the network.
   - "No network address found, so the code names this Mac only; join a network to pair a phone" when the port is on the network but no address was found.

## Style review

1. **The LAN rule is in `ui/`: fixed.** I added a pure core function, `core::remote::invite_address(identity, local, port) -> Option<SocketAddr>`.
   - `invite_url` falls back to loopback when it returns `None`. The dialog uses the same value for its address line and its choice of notice (the point 3 fix above).
   - A new core test, `the_invite_address_is_the_network_address_only_while_the_port_listens_on_it`, covers it.
   - `invite_url`'s address parameter is renamed `local`; the squash body says so.

2. **`authenticate` is too clever: fixed.** It now matches on `(&request.bearer, cookie)` to get `(token, by_cookie)`, refuses with 401 when there is neither, then runs the origin check as before. The `.map(str::to_owned)` clone is gone. Behaviour is unchanged and the remote tests (52) pass.

3. **Undocumented public fields: fixed.** `web::Page`'s `path`, `content_type` and `body` each have a one-line doc comment. `RemoteReply.headers` reads "Headers beyond the ones `respond` always writes, such as a `Set-Cookie` for a browser that paired."

4. **The `too_many_lines` allow on `serve_remote`: reworded.** The comment above the allow now gives the reason not to split: a route table reads best whole, and splitting it would scatter the arms that decide what a device may reach.

5. **Placement of `rate_limited()`: done.** It now sits right after `error_body`, the function it wraps, with a doc comment. `Limits::over` was left as is, as the review allowed.
