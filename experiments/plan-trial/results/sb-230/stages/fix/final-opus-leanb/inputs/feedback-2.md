# Style review: web-control-surface (7fe8949..9b38d45... HEAD 9b56b17)

Every point I have left is wording or small tidiness; none blocks the round.

The layering holds. The core additions (`invite_address`, `invite_url`) are pure. `ui/remote.rs` only chooses which sentence to show. The README Layout table, the dev-aid text, design.md's status section and the CHANGELOG line are all updated. I found no leftover value, moved block or doubled doc comment.

1. `src/app.rs` `remote_pair`: `token.clone()` in the non-cookie branch is needed only because the cookie branch uses `token` later. Build the body without the token, then add it in the `else` branch of an `if cookie`. That removes the clone and the `reply["token"] = Value::String(..)` poke, and the two transports read as two arms.

2. `src/app.rs` `serve_remote`: the new `#[allow(clippy::too_many_lines)]` comment says splitting would "scatter the arms that decide what a device may reach". That is a real reason, but it only covers the route table. The one-line `("GET", ["me"])` arm is what pushed the function over the limit. If the function was already close, say so, or take the allow off if the lint passes without it.

3. `src/adapters/remote.rs` `respond`: the doc comment says "No client reads `Cache-Control` or `nosniff`; they are there so a browser...". That explains why, which is good. "Write one JSON response" followed by that sentence reads as two ideas, though. Move the reason into a `//` comment above the `lines` initialiser, where the headers are written.

4. `src/adapters/remote.rs` `Limits::over`: the doc comment on the path-matching method lost the "poisoned lock counts as over" sentence. It now lives on the new helper, which is right. The method doc ("Whether a request for `path` is over its window.") could say that a path with no window is never over, since the `_ => false` arm is the surprising part.

5. `src/core/remote.rs` `invite_url`: the doc comment says "without one, this Mac's loopback". Say outright that the caller has to show the loopback fallback (the dialog does, with "The port answers this Mac only"), so nobody reads the QR as always phone-reachable.

6. `web/app.js`: `state.sessions` is assigned in `refreshList` but missing from the initial `state` literal. The other fields are all listed there, so add `sessions: {}` for consistency.

7. `src/ui/remote.rs`: the two `None` messages are long string literals inside a `match` arm and rustfmt will not wrap them. Hoist them into two `const`s above `pairing`, or into a small `fn reach_text(identity, address) -> String`, so the draw function stays readable. The text is presentation, so it stays under `ui/`.

8. `src/adapters/web.rs` test `the_pages_logic_passes_its_node_tests`: the doc comment says it is "skipped where node is not installed". The test returns early and reports as passed, so a missing node is invisible in CI. This matches the tmux tests' habit, but a `log::warn!`/`eprintln!` before the `return` would show a skip in the output.
