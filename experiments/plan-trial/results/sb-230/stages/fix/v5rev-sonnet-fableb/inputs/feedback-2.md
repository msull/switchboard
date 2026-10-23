# Style review, round 1 (abe87c6)

Not wording-only: items 1 and 2 are code, the rest are small. Docs (README Layout, design.md status and gaps, CHANGELOG, remote-client.md) are updated for what changed. No I/O or clock in the core, no logic added under `ui/` beyond choosing the payload. No client names found.

1. `src/ui/remote.rs` ~159-178: the `draft.qr = match &payload { Some(..) => if cache-miss { qr_texture(..) } else { draft.qr.take() }, None => None }` is clever where simple would do. `take()` then assign back is a move-out-and-reinsert dance just to keep the cache. Write it as: if `payload` is `None`, `draft.qr = None`; else keep the existing `is_none_or(code differs)` refresh block. That is the old shape plus one line, and it is easier for a Rust learner to read.

2. `src/ui/remote.rs` ~157 with `src/core/remote.rs` `pairing_invite`: the address is formatted through `SocketAddr::new(ip, port).to_string()` in the UI (to bracket IPv6), then `pairing_invite` parses it back to a `SocketAddr` and prints it again, falling back to the raw string on parse failure. Two places own the bracketing and one is a round trip. Pick one: either the UI keeps the formatting and `pairing_invite` just interpolates `address` (the doc comment already says "an IPv6 literal is bracketed"), or `pairing_invite` takes `(IpAddr, u16)`. The silent `map_or_else` fallback also hides a bad address instead of failing.

3. `src/core/remote.rs` `token_cookie_name` hardcodes `"switchboard_token_"` while `adapters/remote.rs` has `pub const TOKEN_COOKIE_PREFIX` with the same literal. Two sources for one string that must agree. The adapter filter silently drops every cookie if they drift. Have one own it (the core fn can use the prefix const, or the const can move to `core::remote` and the adapter import it) and say in the doc comment that they must match.

4. `src/app.rs` `set_cookie(name, value, max_age: Option<u32>)`: the only values ever passed are `None` and `Some(0)`. A general `Max-Age` parameter is more than the two callers use. Two small functions (`token_cookie`, `clearing_cookie`) or a `clear: bool` read plainer, and the "`Some(0)` clears it" doc line goes away.

5. `src/app.rs` `authenticate`: `crate::core::remote::token_cookie_name` is spelled out in full twice (here and in `remote_pair`) while `ui/remote.rs` imports the module as `core_remote`. Add one `use` at the top of `app.rs`. Also `request.bearer.is_none()` is checked to decide whether to clear the cookie, but the cookie is only consulted when no bearer is present; a comment that says so (why a bearer 401 must not clear the cookie) would help, as written it reads like debris.

6. `src/adapters/remote.rs` `respond`: `content_type` is built as an empty `String` when a header already supplies it, else a literal. An `Option`/`if` that pushes into one `String` is not needed, but `String::new()` as "nothing" plus a separate `extra` string is two buffers for one head. Low priority; wording-level.

7. `README.md` remote paragraph: the diff left `... still takes it (and the earlier JSON form). Script lines \`remote-listen lan <port>\` and` on one overlong line, breaking the paragraph's wrap. Rewrap.

8. `web/app.js` header comment says the page keeps "the host's id and key ... plus the scope". The plan said only `server_id` and `spki_sha256`; the comment is accurate to the code, so no change needed, but design.md and the README `web/` row say "holds no token" only. Fine, no action.

Settled: nothing carried over (first round).
