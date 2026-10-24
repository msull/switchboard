# Correctness review, round 1: stage/sb-230-implement-v5rev-opus-leanb2 @ 6909b3d

I found no blocking defects. Method: I read the diff against the plan and facts, read all of `web/app.js`, and ran `cargo test --locked --test remote` (51 passed). I did not run the whole workspace, clippy, or node.

Checked and found to match the plan:
- Cookie auth: the bearer wins when present, and the cookie is read only by its exact per-server name.
- The `Origin` rule applies to cookie-only requests that are non-`GET` or an upgrade. A bearer request is exempt.
- `/pair` with `transport: "cookie"` drops `token` from the body and sets the cookie. A native pairing is unchanged.
- Page paths are answered on the connection thread, with their own window, and the connection closes afterwards.
- `invite_url` falls back to loopback when `listen.lan` is off.
- No record, schema or contract type changed, so no `SCHEMA_VERSION` step is owed.

## Points (all minor, none blocks)

1. **`web/app.js` `pairWith` → `start`: wrong message if the browser refuses the cookie.** This is the plan's Unresolved 3 failing. `/pair` succeeds, so `saveStored` runs. `/me` then returns 401 because the browser stored or sent no cookie. `unauthorized()` finds the stored id and key equal to `/info` and says "This device was removed on the host. Pair again." That is false: the device is paired, and the cookie is the problem. The host then lists a device the user cannot use. It is only a wrong message, and it appears exactly in the case the plan told the implementer to test first. Suggestion: when `/me` fails with 401 right after a successful pair, say that the browser did not keep the pairing. Do not save `stored` until `/me` works.

2. **`web/app.js` `connectSocket`/`onclose`: a socket that never connects costs a full refresh each retry.** The case is Safari over a click-through certificate, which the facts say is expected. Each `onclose` calls `refresh()`, which makes `/projects` plus one `/sessions` call per project, and then `retrySocket()` runs. The retry delay caps at 60 s, so the page makes a full list fetch every 60 s on top of the 3 s poll, whether or not anything changed. The plan says "on a close or an error, the page refetches". It does not say to refetch after a socket that never opened. Suggestion: refetch in `onclose` only if `onopen` fired for that socket.

3. **`src/adapters/remote.rs` `connection`: a page `GET` that carries an `Upgrade` header is served as a page.** The page match comes before the `request.upgrade` handling. A handshake for `wss://host/` therefore gets a 200 and a close instead of an upgrade refusal. This is harmless, because `/` is not an events route. I mention it only because the plan says page paths are matched on method and path alone.

## Not checked
- Real browser behaviour (the spike README says what was and was not run).
- `cargo clippy`, the Swift tests, and `node --test web/app.test.js`, except that the Rust wrapper test passed inside the 51.
