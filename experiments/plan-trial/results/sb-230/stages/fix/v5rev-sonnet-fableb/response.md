# Response, round 1

All fixes are in one fixup commit (d104728) onto abe87c6. fmt, clippy and `cargo test --locked --workspace` pass. I did not run the page in a browser, and the JS changes have no automated test.

## Correctness

1. **Rotated key blocks re-pairing.** Fixed. `start()` shows "identity changed" only when there is no invite. With an invite it goes on to the existing key check against `invite.key` and then pairs.
2. **Refresh dropped mid-refresh.** Fixed. `refresh()` sets `state.dirty` when one is already running, and runs once more from its `finally`.
3. **Revoked device not shown after a 401.** Fixed. A 401 from `/projects` now calls `revoked()`. It sets `state.dead`, clears the poll timer, closes the socket and shows the notice. `render()` and `refresh()` return early while `dead` is set, so the notice stays. I did not touch the 401 handling in `start()`, which returns before the timer or socket exist.

## Style

1. **QR cache.** Fixed as suggested: `None` payload sets `draft.qr = None`; otherwise the old `is_none_or` refresh block runs.
2. **Address formatted twice.** The UI keeps the bracketing (`SocketAddr` display). `pairing_invite` now just interpolates `address`, and the doc comment says the caller brackets. The silent fallback is gone.
3. **Cookie prefix in two places.** `TOKEN_COOKIE_PREFIX` moved to `core::remote`. `token_cookie_name` builds on it and the adapter imports it. Both doc comments say they must agree.
4. **`set_cookie` with `max_age`.** Replaced by `token_cookie` and `clearing_cookie`.
5. **Spelled-out path and the bearer check.** Added `use crate::core::remote::token_cookie_name` in `app.rs`. The comment at the clearing site now says why a refused bearer must not clear the cookie: the cookie is not consulted when a bearer came.
6. **`respond` buffers.** `content_type` is now a `&str` (empty or the default header) instead of an owned `String`. `extra` stays, since it is built from the headers list.
7. **README wrap.** Rewrapped.
8. **Header comment / docs.** No change, as the review said.

I left the `POST /` returning 429 after 120 hits alone, as the review called it harmless.
