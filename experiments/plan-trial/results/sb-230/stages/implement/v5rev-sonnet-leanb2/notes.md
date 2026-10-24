# Implement notes: #230 web control surface

Branch `stage/sb-230-implement-v5rev-sonnet-leanb2`, one commit (as the plan names):
`4b65af5` "Serve a web control surface from the remote port, paired from a URL whose fragment carries the code". Not pushed; no PR. `git status` is empty.
`cargo fmt --all`, `cargo clippy --locked --workspace --all-targets -- -D warnings` and `cargo test --locked --workspace` are clean. `swift test --filter testInvite` passes (the new URL test and the existing JSON one). No `cfg(target_os)` arm touched, so the Linux dispatch clippy was not run.

## Built
- `RemoteReply::new` / `with_header` / `headers`; all literals converted. `RemoteRequest.{cookie, origin, host}`. `respond` adds `Cache-Control: no-store` and `X-Content-Type-Options: nosniff` to every JSON reply and writes reply headers.
- `src/adapters/web.rs`: the page table (`include_str!`), the CSP and headers. Pages answered in `connection` (GET only, exact path) with `Connection: close`; `Limits.page` 120/min.
- `authenticate`: bearer, else the cookie named `__Host-sb-<server_id>` (exact name). Cookie-only non-GET or upgrade requires `Origin == https://<Host>`, else 403 `origin`. `/pair` takes `transport: "cookie"` (no `token` in the body, `Set-Cookie` as planned). `GET /api/v1/me`.
- `core::invite_url`, `cookie_name`, `cookie_value` (re-exported from `core`). Dialog: QR and "Copy invite" carry the URL; Mac-only notice when `listen.lan` is off (URL then uses 127.0.0.1); browser-trust sentence.
- Swift `PairingInvite.parse` reads the URL as well as the JSON.
- `web/index.html`, `app.js`, `app.css`, `app.test.js` (`node --test`, run from a Rust test that skips without node).
- Tests: core (`invite_url`, cookie parse), `tests/remote.rs` (pages and headers, embedded-page guards and route table, cookie pairing, two hosts, Origin rule incl. upgrade, `/me` and revoke/`/info`, rotated id, pool and 120/min window, node tests), `tests/ui.rs` (invite URL, Mac-only notice), Swift.
- Docs: remote-client.md (§0, §4.1, §4.3 rows, excerpt sentence, §5), design.md paragraph before "Versioning and updates", README Layout/spikes/dev-aid, CHANGELOG Unreleased.

## Plan fallbacks and unresolved facts (not settled by me)
- **Unresolved 1, 2, 3, 4, 6 (certificate click-through, iOS `wss://`, cookie in Safari/Chrome from an accepted certificate, `replaceState` history, parallel connection count)**: built the plan's default (click-through certificate, cookie transport, polling base, socket as an enhancement). Spike `spikes/23-web-page/` has a README that says **not run** and gives the commands; no browser could be driven here. The cookie fallback (token held by the page + `Sec-WebSocket-Protocol`) is not built, as the plan says.
- Five-parallel-connection measurement not made (needs a browser or harness); only the sequential and stalled-stranger pool behaviour is tested.

## Where the plan was wrong or differed
- Plan's line numbers for `docs/remote-client.md` (207, 306, 312, 319) did not match this tree; I edited the same passages by content (excerpt sentence in the `last_response` bullet, §5 "Require pairing before…", QR sentence, token sentence).
- `serve_remote` crossed clippy's 100-line limit with the `/me` arm; added `#[allow(clippy::too_many_lines)]` (the file already uses it once).
- "Spike" is a README only, not a throwaway server, because nothing could drive Chrome/Safari.
- The plan's `invite_url(identity, window, address, port)`: `address` is an `IpAddr` (brackets added for IPv6).
- Content replies are judged by `epoch`/`revision` per session key in the page, not against the list's global revision: a content key's revision can be lower than the global one, so comparing against the list's would drop valid replies. (Plan said "by revision as §4.4 says"; this is the reading that works.)
- The old UI test asserted the JSON invite; rewritten to the URL as the plan said. No schema versions touched (SCHEMA_VERSION 16 unchanged).
