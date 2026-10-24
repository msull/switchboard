# Response to round 1

All fixes are in one commit, 029b7c0, `fixup!` of 6909b3d. Nothing that commit's message names was renamed or removed, so a fixup is enough. `cargo fmt --all` and `cargo clippy --locked --workspace --all-targets -- -D warnings` are clean. `cargo test --locked --workspace` passes, including `tests/remote.rs` (51), whose wrapper test runs `web/app.test.js` under node. No `cfg(target_os)` arm was touched, so the Linux cross-check was not needed.

## Correctness review (feedback-1)

1. **Wrong message when the browser refuses the cookie.** Fixed. After a successful `/pair`, `pairWith` now calls `/me` itself, with the 401 inference turned off. `call` has a new `infer` argument, which defaults to off only for `pair`. A 401 there shows a new `COOKIE_REFUSED` message: the host paired the browser, but the browser did not keep the pairing; allow cookies, remove the device on the Mac, and scan again. Any other failure shows "Can't reach the host." `saveStored` runs only once `/me` returns 200. That reply is passed into `start(me)`, and from there to `readMe(reply)`, so the page does not fetch `/me` twice.

2. **A socket that never opens costs a full refresh on every retry.** Fixed. Each socket now has an `opened` flag, set in `onopen`. `onclose` calls `refresh()` only if that flag is set. It still reschedules the poll and retries the socket. A socket that never opened cannot have missed a change, so the 3 s poll covers it.

3. **A page `GET` with an `Upgrade` header is served as a page.** Not changed. The plan says page paths are matched on method and path alone, and the code does exactly that. As the review says, `/` is not an events route, so the client's handshake fails either way. A 200 followed by a close is no less correct than a refusal. Checking the upgrade first would add a branch with no effect on behaviour.

## Style/architecture review (feedback-2)

1. **`authenticate`: `by_cookie`, the bearer clone, and the closure name.** Fixed. The cookie credential is now `from_cookie: Option<&str>`, computed by one `match` only when there is no bearer and the request has a cookie header. It borrows from the request, because `cookie_value` already returns a slice of the header. The token is `request.bearer.as_deref().or(from_cookie)`, so nothing is cloned. The `Origin` rule now reads `from_cookie.is_some() && writes && !same_origin(request)`, which means "authenticated by cookie".

2. **`/pair` reply built and then trimmed.** Fixed. The body is built without `token`. `if !cookie { reply["token"] = token.as_str().into(); }` adds it for a native client only, so the rule sits in one visible branch.

3. **`respond_page`'s `fold`, and `write_response` with no doc.** Fixed. `respond_page` now builds its headers with a `for` loop, as `respond` does. `write_response` has a doc comment saying that `extra` is whole, pre-formatted `Name: value\r\n` lines. I kept the loose arguments: there are only two callers, both in this file, and the doc names what each one carries.

4. **`Limits::over` has no doc.** Fixed. `limited` now says it picks the window by path, and that a path with no window is never over. `over` says it spends one request from the given window and returns whether the window was already full. The note about the poisoned lock moved to `over`, which is where the lock is taken. `serve_page`'s bool and its doc are unchanged, as the review suggested.

5. **The `too_many_lines` allow on `serve_remote`.** Kept, with the reason stated. Without the allow, clippy reports 101/100: the `me` arm put the function one line over the limit. Moving `me` beside the `events` check would add lines once rustfmt formats it. Splitting the route table would mean a reader has to look in two places to find where a path goes. The comment above the allow now says both.

6. **`src/ui/remote.rs`.** No action; the review asks for none.

7. **README line left long by the reflow.** Fixed. The dev-aid paragraph is rewrapped to about 70 columns.

8. **`web/app.js` comments and debris.** No action; the review asks for none. The new comments in `pairWith` and `onclose` say why, not what.
