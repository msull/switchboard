# Response to round 1 (head ecffe04)

One commit, `squash! ecffe04` (9b42aae), because the implementation commit's contract list names `core::remote::invite_url` and the fix adds `invite_address` and `InviteAddress` beside it; the squash body carries the corrected line. `cargo fmt --all`, `cargo clippy --locked --workspace --all-targets -- -D warnings` and `cargo test --locked --workspace` are clean (the remote suite is 51 tests and runs `node --test web/app.test.js` inside it). No `cfg(target_os)` arm was touched.

## Correctness review

**1. A transient failure leaves the page stuck on the error screen.** Fixed. `show()` now records the current view, `unreachable()` remembers the list or session it is hiding, and `read()` calls `reachable()` on any reply that is not a network failure or a 401, which restores the hidden view and clears the mark. Polling was never stopped, so the first poll after the host comes back puts the page where it was, whether or not the revision moved (the restore is in `read`, not in the revision comparison). The boot-time case (host unreachable before the page was ever paired) still ends on the notice, because no poll is running then; that is a reload, as before.

**2. A refused cookie reports "Removed on the host".** Fixed. `pair()` no longer routes a 401 from the `/me` that follows a 200 `/pair` through `inferUnauthorized`. It shows a notice of its own: "Paired, but the cookie did not come back", saying that the Mac paired and listed this browser, the code is spent, so revoke it on the Mac, allow pairing again and scan the new code, and that a browser may refuse cookies from a certificate it was clicked through. A network failure on that `/me` shows the unreachable notice. design.md's status paragraph names this as the behaviour when the browser withholds the cookie.

**3. A malformed fragment throws before `replaceState`.** Fixed. `boot()` calls `history.replaceState` whenever `location.hash` is non-empty, before parsing. `parseFragment` catches `URIError` from `decodeURIComponent` and returns null, and a null from a non-empty hash (bad escape or a partial invite) shows an "Incomplete link" notice: scan again or paste the whole URL from Copy invite. `web/app.test.js` covers both malformed inputs.

**4. Loopback URL with no warning when the listener is on the LAN but no address was found.** Fixed together with style point 1. `core::remote::invite_address(identity, port, local)` returns an `InviteAddress`: `Network(addr)`, `NoNetwork(loopback)` or `MacOnly(loopback)`, and `invite_url` takes that value, so the dialog's address line and the QR code come from one decision. The `NoNetwork` arm shows "No network address found: join a network for a phone to reach the port." The core test covers all three cases, including that `local` is never called while the port is this Mac's alone.

**5. Weak or missing tests.** Partly a correction, partly fixed.
- `stalled_strangers_fill_their_own_pool…` was extended in ecffe04: after the four stalled strangers it sends a page request with `http_with` and asserts it fails at accept (`tests/remote.rs`, "A page request is a stranger too"). That is the plan's assertion.
- The sequential-load weakness stands and is fixed: `page_loads_free_their_slot_and_have_a_window_of_their_own` now opens four pinned connections, reads a page on each and keeps them open (a new `http_open` helper hands the stream back; `http_with` is now a wrapper over it), then reads `/info` from a fifth and expects 200. I checked what it guards: with `Connection: close` alone removed it still passes, because the slot is freed by the connection thread returning after the page, which the header announces; with the thread kept reading (`return` changed to `continue`) it fails at the fifth accept. The test comment says exactly that.
- A `GET /` carrying `Upgrade: websocket` and a `Sec-WebSocket-Key` is now asserted to be answered as the page with `Connection: close`. "The page answer does not consume a paired slot" is not asserted directly: a page reply never sets `authenticated`, so it never reaches the paired-pool branch, and there is no observable from a test other than the paired count, which the harness does not expose. Left as the reviewer rated it, low value.

## Style review

**1. Logic under `ui/`.** Fixed as above: the `lan` rule and the loopback fallback are in `invite_address`, the dialog passes `remote::local_address` as the closure and matches the enum to pick a line. The second `SocketAddr::new` is gone. `README.md`'s Layout row, `docs/design.md` and `docs/remote-client.md` say the address is decided in the core and that the dialog now has two notices.

**2. The cookie name is built twice.** Fixed. `cookie_name(&Identity) -> String` is a free function beside `COOKIE_MAX_AGE_SECS` with the "why" on it; `cookie_token` and `remote_pair` both call it, and the pair site has a one-line comment pointing at it.

**3. `authenticate` is clumsier than needed.** Fixed. `cookie_token` returns `Option<&'r str>` borrowed from the request, the credential is chosen with `if let … else if let … else return`, and nothing is cloned. The doc comment keeps the why (`SameSite=Strict` withholds cross-site; `Origin` closes same-site, other-port) and drops the rest.

**4. Unrelated churn in `remote_dispatch`.** Reverted, with a correction to the premise: the reformat was not free. `serve_remote` was at clippy's `too_many_lines` limit and the one-line `/me` arm pushed it to 101, which the reformat hid. The original `notes_cap` form is back and the function carries `#[allow(clippy::too_many_lines)] // one arm per route`, which is how the tree's other route tables handle it (`core/control.rs`, `core/action.rs`).

**5. Wording and placement in `src/adapters/remote.rs`.** All fixed: `respond`'s doc drops the sentence; `write_response` has a doc naming `no-store` and `nosniff` and their purpose, `extra` and `close`; `respond_error`'s doc names the thing; `RATE_LIMITED` sits after `PAGE_PER_MINUTE` with a doc; `respond_page`'s doc says the common headers come from `write_response`.

**6. `web.rs` `Page` fields and `PAGES`.** Fixed. Each field has a doc. `PAGES` is private: `tests/remote.rs` reads the table through `web::page(path)` only.

**7. `origin_is_own`.** Fixed. It is `RemoteRequest::origin_is_own()` beside `param`, without the `trim` calls (the doc notes both headers were trimmed on reading), and the adapter's request-parsing test asserts it true for a matching origin and false for another port and for no origin.
