# Implementation notes: web control surface (#230)

Branch `stage/sb-230-implement-v5rev-sonnet-leanb`, one commit: "Serve a web control surface from the remote port, paired from a URL whose fragment carries the code". Not pushed; no PR.

## What was built

- `src/adapters/remote.rs`: `RemoteReply.headers` and `RemoteReply::new` (all ~11 literals converted); `RemoteRequest.{cookie, origin, host}`; `respond_with` (extra headers, plus `Cache-Control: no-store` and `X-Content-Type-Options: nosniff` on every JSON reply); `Limits.page` (120/min) keyed on the three page paths; page requests answered on the connection thread with the CSP, `Referrer-Policy`, `Connection: close`, then the connection ends.
- `src/adapters/web.rs`: the table of three `include_str!` pages and the CSP.
- `src/app.rs`: `authenticate` takes the bearer, else the cookie `__Host-sb-<server_id>` (exact name); a cookie-only non-GET or upgrade needs `Origin: https://<Host>` or 403 `origin`; `/pair` honours `"transport": "cookie"` (token omitted from the body, `Set-Cookie` set); `GET /api/v1/me`. `serve_remote` got `#[allow(clippy::too_many_lines)]` like its neighbours.
- `src/core/remote.rs`: `invite_url`, `cookie_name`, `cookie_value` (re-exported from `core`).
- `src/ui/remote.rs`: QR and "Copy invite" carry the URL; loopback + "Mac-only" line when `listen.lan` is off; the browser-trust sentence.
- `client/.../Info.swift`: `PairingInvite.parse` accepts the URL (JSON still parses); test added; `swift test` passes (82 tests).
- `web/index.html`, `app.js`, `app.css`, `app.test.js` (pure parts under `node --test`; a Rust test runs it and skips if `node` is absent).
- Tests: core (`invite_url`, cookie helpers), `tests/remote.rs` (pages and headers, JSON 404s, page-vs-host route table and CSP guards, cookie pair/`/me`/revoke/fixed-name, native pair unchanged, two hosts, Origin rule incl. upgrade, pool and page window, node tests), `tests/ui.rs` (invite rewritten).
- Docs: `docs/remote-client.md` (§0, §4.1, §4.3, the excerpt line, §5), `docs/design.md` dated paragraph, README rows, `CHANGELOG.md`, `spikes/23-web-page/README.md` and the spikes index.

`cargo fmt`, `cargo clippy --locked --workspace --all-targets -- -D warnings` and `cargo test --locked --workspace` are clean. No `cfg(target_os)` arm touched, so the dispatch Linux check was not needed.

## Contract additions (as in the commit message)

No new `AppAction`/`Effect`/port method. `RemoteReply.headers`, `RemoteReply::new`; `RemoteRequest.{cookie, origin, host}`; `core::remote::invite_url` (plus `cookie_name`, `cookie_value`, which the plan did not list); wire: `GET /api/v1/me`, `/pair` field `transport`, the `__Host-sb-<server_id>` cookie as a credential, 403 `origin`, `GET /`, `/app.js`, `/app.css` before pairing, invite text is a URL. No `model.rs`, `control/` or schema change.

## Fallbacks and unresolved facts, built as the plan says (not settled here)

- **Spike 23 was not run.** No browser was driven. Its README says "not run" with the steps and an empty table. Unresolved 3 (cookie on a user-accepted self-signed IP-literal host), Unresolved 1/2 (click-through, `wss` on Safari), Unresolved 4 (`replaceState`), 6 (connection count) remain open. The cookie transport is built; the `Sec-WebSocket-Protocol` fallback is not built (left for the owner, per the plan).
- Certificate untouched: no SAN, click-through is the trust step. `facts.md` says to add an IP SAN regardless; the plan overrides this and I followed the plan.
- Polling is the base; the socket is an enhancement.

## Where the plan was wrong or loose

- The plan says `RemoteRequest` literal is at `remote.rs:506` only, true; but also says "about twenty" `RemoteReply` literals; there were 11 (the rest are fn signatures).
- The plan's route-table test said every `/api/v1` string "answers something other than `not_found`". With real ids a missing session still gives `not_found`, and `/events` without an upgrade gives 400. The test instead asserts the error message is not the port's route-level "not available on this port"/"no such endpoint".
- A cookie `GET` upgrade also needs `Origin`; I implemented "non-GET or upgrade" as written, so a cookie GET without upgrade (the page's reads) needs none.
- `invite_url` takes the address without a port plus the port separately (plan: `address, port`); when `listen.lan` is on but no LAN address is found, the URL falls back to `127.0.0.1` and no Address line shows.
- The plan's "the dialog says the port is Mac-only" is shown as a line in place of the Address line.
- The `"Origin"` check compares `Origin` to `https://<Host header>` as a string; a browser omits default ports, which is consistent with `Host`, but a proxy that rewrites `Host` would break it.
- The node test of `app.js` was run only through the Rust test (the shell would not run `node` directly in this session), so I could not see its output; the Rust test passes (and silently skips if `node` is missing).
- Browser behaviours (page load, pairing, polling, socket) were never exercised in a real browser; only the Rust host side and the pure JS functions are tested.
