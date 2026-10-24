# Correctness review: web-control-surface (round 1, head 9b56b17)

I read the host diff in full (`src/adapters/remote.rs`, `web.rs`, `app.rs`, `core/remote.rs`, `ui/remote.rs`) and all of `web/app.js`. I did not read `tests/remote.rs`, `tests/ui.rs` or the Swift change closely, and I ran no tests.

## Points

1. **`web/app.js` `poll()` / `visibilitychange`: each visibility change adds a second polling chain.**
   - `poll()` sets `state.timer = null` as its first line without clearing the timeout that is pending.
   - The `visibilitychange` handler calls `poll()` directly while the normal timer is still pending.
   - The pending timer stays scheduled but is no longer tracked. `schedule()` sees `state.timer == null`, so it arms a second timer and clears nothing. Both timers then fire `poll`, and each re-arms itself.
   - Input: open the page, switch the tab to the background and back three times. Four independent chains now poll `/waiting` every 3 s, and the count never falls.
   - `stopUpdates()` clears only the tracked one, so after a 401 the orphaned chains keep running. They check `state.me`, which is null after a 401, so they stop on their next fire. That bounds the leak, but it persists for the whole signed-in session.
   - Fix: in the handler, `clearTimeout(state.timer)` before calling `poll()`. Or have `poll()` not null `state.timer` before the old one is cleared.

2. **`src/ui/remote.rs` pairing(): the cached QR is keyed on the code only, but the payload also depends on `listen.lan`, the local address and the port.**
   - `invite_url` now varies with `identity.listen.lan`, `local_address()` and `port`. The QR texture is rebuilt only when `window.code` changes.
   - State: the pairing window is open and showing the QR, then the owner turns "allow devices on the local network" on (or the Mac changes network).
   - The "Address" line updates immediately, but the QR and "Copy invite" payload are not tied to the same cache. The QR still encodes `https://127.0.0.1:<port>/…`. A phone scanning it gets an unreachable page while the label beside it shows the LAN address.
   - The old payload had the same keying, but it carried the address list. The new payload makes the loopback fallback visible, and this branch introduced it.
   - Fix: key the cache on the payload string, or on `(code, payload)`.

3. **`src/adapters/remote.rs` `connection`: the page branch sits before the `paired` and pool bookkeeping and ends the connection with `return`.**
   - This is correct for an unauthenticated stranger: the slot is freed.
   - It is wrong for a connection that is already in the paired pool. A paired native client that issues `GET /` on its kept connection gets its connection closed.
   - I do not think a real client does this, so it is low severity. Match the plan (page paths close) but be aware the pool counter is released by the caller's guard, not here. Confirm that the drop path decrements the same pool the connection was counted in; the tests I did not read should cover this.

4. **`authenticate`: an origin-failed cookie request is refused with `authenticated: false`.**
   - It is a 403 for a paired device, and the 403 is sent from `refuse(.., false)`.
   - A paired connection in the paired pool therefore receives a reply with `authenticated == false`. The pool move only happens on `authenticated && !paired`, so nothing breaks.
   - Withdrawn as a defect; noted only so it is not mistaken for one.

5. **`start()` with a stale cookie but no stored host.**
   - If the cookie is valid but `localStorage` was cleared, `start()` succeeds via `/me`.
   - A later 401 then reports "Not paired" (`unpaired`) rather than "removed". The plan accepts this because the inference needs the stored identity. State: cleared site data but the cookie survives, then Revoke. The user sees "Not paired".
   - Low severity. `start()` could store `{server_id, server_name}` from `/me` when nothing is stored, but not `spki_sha256`, which `/me` does not return. So this cannot be fixed without a wire change. Noted, no action.

## Checked, no defect

- `authenticate`: a bearer takes precedence over a cookie. A stale bearer with a valid cookie is a 401, as intended.
- `same_origin` against `Host`: an uppercase `Host` would fail the comparison, but browsers send it lowercase. Exact port match is right.
- Cookie value is `URL_SAFE_NO_PAD`, so it is cookie-safe, and `cookie()` handles `=` inside values via `split_once` on the first `=`.
- The `Set-Cookie` name uses the same `server_id` as the reply body.
- `/pair` without `transport` is unchanged.
- No record or schema change, so no `SCHEMA_VERSION` step is needed.
- `invite_url` / `invite_address` fall back to loopback only when the port is not on the LAN or no address was found.
