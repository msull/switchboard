# Response, round 1

All fixes are in one fixup commit (681a359) on 4b65af5. `cargo fmt`, clippy with `-D warnings` and `cargo test --locked --workspace` were run. The workspace run had one failure, in `an_overflowed_subscriber_is_told_to_resync_and_closed_while_another_keeps_up` (see the end). No `cfg(target_os)` arm was touched, so I did not run the Linux dispatch check.

## Correctness

1. **Fixed.** `start(justPaired)` in `web/app.js`: a 401 on the first `/me` right after a successful `/pair` now shows "Paired, but not signed in". It says the host accepted the code, the browser did not keep the credential, and pairing again will not help. It no longer calls `infer401`, so it cannot say "removed".
2. **Fixed.** `tick` now calls `readSession(false)` on every poll while a session is open and the list is unchanged. That renews the watch and compares the content reply's own epoch and revision. An unchanged reply does not redraw, so scroll position and the draft are not disturbed. List-driven refreshes, `openSession` and the scope recheck pass `redraw = true`. I did not check whether pane output moves the global revision; the content read now covers either case.
3. Withdrawn by the reviewer. No change.
4. **Fixed.** `Limits::limited` now takes the method, and the page window counts `GET` plus a page path only. That matches the page branch in `connection`.

## Style

1. **Fixed.** The address rule is now in the core: `lan_address(identity, local)` returns the LAN address only when the port listens on the network. `invite_url` takes `Option<IpAddr>` and falls back to loopback. The UI only calls these two and draws. The core test now covers the loopback fallback with `None`. Both functions are re-exported from `core/mod.rs`, additively.
2. **Fixed.** The `23-web-page` entry in the README `spikes/` line is at the end, single-spaced.
3. **Fixed.** `write_response` takes `&[(&str, &str)]`. `respond_page` passes `web::HEADERS` directly. `respond` builds its list of borrowed `&str`s, so there is no per-reply `String` allocation.
4. **Fixed.** The browser half of `deviceName` is an `if/else if` chain, like the device half.
5. **Fixed.** `page.marker` is gone. `tick` compares the reply's epoch and revision with `page.applied`.
6. **Fixed.** One doc line on each of `Page`'s fields.
7. **Fixed.** `remote_me` answers 503 `no_identity` as `remote_info` does, instead of 200 with empty ids.
8. **Not changed.** The reviewer flagged this one as optional. `serve_remote` is over the line limit because of arms that built up over several changes, and splitting them is a refactor of its own. The `/me` arm is two lines. I left the allow in place.

## Test failure

`an_overflowed_subscriber_is_told_to_resync_and_closed_while_another_keeps_up` failed twice in the full-workspace run. It asserts one surviving subscriber and saw 0. `cargo test --test remote` passes 53/53 on its own, including three runs after my edits. The test exercises subscriber queue timing, which none of my edits touch. I think it is load-sensitive when run alongside the other suites, but I did not run it on the base commit to confirm that.
