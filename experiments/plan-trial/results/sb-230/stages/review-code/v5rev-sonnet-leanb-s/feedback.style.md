# Style review, round 1 (head 47f139e)

Not wording-only: points 1 to 3 are stale or missing docs, 4 is logic under `ui/`.

1. **`docs/remote-client.md` §5 contradicts itself.** The bullet "Pairing verifies the key before the code is sent" still says the QR carries `{server_id, spki_sha256, code, addresses}`. Two bullets later the new "The invite is a URL" bullet says it carries a URL. Edit the old sentence to say the QR is the URL; the JSON form survives only as what `PairingInvite.parse` still accepts.

2. **README Layout rows not updated for what changed.** The `src/adapters/remote.rs` row (README:384) lists "`/pair` and `/info` rate limits" and never mentions the page window, the cookie fields on `RemoteRequest`, or `RemoteReply.headers`. The `src/core/remote.rs` row (README:370) doesn't mention `invite_url`, `cookie_name` or `cookie_value`. CLAUDE.md asks to keep the Layout table current. The `web.rs` and `web/` rows are fine.

3. **design.md.** The new paragraph is good. It doesn't say the two behaviours a reader will wonder about: the browser pairing is weaker than the native pin (it's in remote-client.md only), and the Swift `PairingInvite.parse` now takes the URL. One sentence each, or point at remote-client.md §5.

4. **Logic under `ui/`, `src/ui/remote.rs` `pairing`.** The choice "loopback when the port is Mac-only, else `local_address()`" is made in the draw function, and `address` and `host` are then derived twice (`host.map(...)` and `host.map_or_else(...)`). The plan said `invite_url` takes the address "so the UI holds no logic and a test can call it". Move the choice into the core, e.g. `invite_url` taking `Option<IpAddr>` plus `listen.lan`, or a small `invite_address(identity, local) -> String`. Then core tests cover the loopback case. As it stands the Mac-only branch is only reachable through a UI test.

5. **`respond` / `respond_with` / `respond_page` in `src/adapters/remote.rs`.** `respond` is now a one-line wrapper over `respond_with(w, status, &[], body)`. If it has few callers, make them call `respond_with` or give `respond_with` the name `respond`. Otherwise it's fine. This is minor. The `Page` struct's fields in `adapters/web.rs` are pub with no doc lines. The sibling structs document their fields, so add one-liners.

6. **`#[allow(clippy::too_many_lines)] // one arm per route` on `serve_remote`.** This is a lint silenced rather than a function split. It's defensible since the arms are flat, but say why in terms of this function (the match is the route table) rather than the generic "one arm per route". Check the allow is still needed after the `events` arm moved into the match; if clippy is clean without it, drop it.

7. **`authenticate`.** `by_cookie = request.bearer.is_none()` is true even when no cookie exists. It's harmless because that path already returned 401, but a reader has to prove that. Setting it from whether the token came from the cookie (`request.bearer.is_none() && token.is_some()`) would read as what it means. The `request.bearer.clone()` is fine.

8. **`spikes/23-web-page/`.** It is a README that says "Not run", and `spikes/README.md` and the README row lead with it. CLAUDE.md says a new risky mechanism gets a spike *before* it's built on. That's out of my brief (correctness, not style), so I'm noting it for the other reviewer. The docs at least say it honestly.

9. **`web/app.js`.** The pure functions are separated and exported, and the comments explain why. `start()` is a ~350-line closure holding every screen. It matches the no-build constraint, but splitting the drawing functions out of the closure would help whoever reads it next. Not a blocker. `catch (e)` with an unused `e` appears three times, so use bare `catch {`.

Nothing else: no narrating comments, no doubled doc comments, no I/O or clocks in the core (`cookie_value`, `cookie_name` and `invite_url` are pure), and the CHANGELOG line is present.
