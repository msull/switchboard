# Style review, round 1: stage/sb-230-implement-v5rev-opus-leanb

Nothing blocking on Style or Architecture. Docs are current: README Layout rows, the dev-aid sentence, design.md status, remote-client.md, CHANGELOG, and the spike index are all updated. Core stays pure, and no I/O or clock was added to it. The points below are small; the first is the only one I'd ask to change, and the rest are wording or taste.

1. **Logic under `ui/`: the LAN rule is written twice.** `src/ui/remote.rs` (the `pairing` fn) now does `local.filter(|_| identity.listen.lan)` to build the "Address" line. `core::remote::invite_url` does the same filter to build the URL. CLAUDE.md says that if logic appears under `ui/`, it moves to the core. If the rule changes (say, a bind to a specific interface), the two copies drift and the dialog shows an address the QR doesn't use. Suggest one small pure core fn, e.g. `invite_address(identity, local) -> Option<SocketAddr>`. `invite_url` and the dialog would both call it, and the "answers this Mac only" branch would test the same value.

2. **`authenticate` is cleverer than it needs to be** (`src/app.rs`, near line 3128). `by_cookie = request.bearer.is_none()` is true even when no cookie exists, and it only means "cookie" because the next `let ... else` returns early. `request.bearer.as_ref().or(cookie.as_ref())` also needs the `.map(str::to_owned)` clone just to line up the types. Two plain steps would read better: pick `(token, by_cookie)` with a `match` on bearer then cookie, then run the origin check. Behaviour is the same.

3. **Undocumented public fields.** `adapters::web::Page` has `path`, `content_type` and `body` with no doc comments. `RemoteReply.headers` has none either, though the struct doc was extended. CLAUDE.md asks for doc comments on public items. A one-liner each is enough (`headers`: "beyond the ones `respond` always writes").

4. **`#[allow(clippy::too_many_lines)] // one arm per route` on `serve_remote`.** This hides that the fn crossed the lint with one added arm. It's defensible as a route table, but the comment explains the lint and not why the fn shouldn't be split. Wording only; leave it if you prefer.

5. **Wording:** `rate_limited()` sits between `respond` and `error_body` in `adapters/remote.rs`. It is a one-liner over `error_body` and reads better next to it. The `Limits::limited` doc lost nothing, but `Limits::over` now carries the poisoned-lock sentence; fine as is.

Points 3 to 5 are wording or taste; if 1 and 2 are settled, the round can close on those.
