# Style review, round 1 (head dc1c58a)

Points are small; none blocks. Layering is clean: the core additions are pure, `pairing_invite` and `token_cookie_name` do no I/O, the UI only calls them, the page routing sits in `app.rs`. README Layout, design.md status and gaps, remote-client.md and CHANGELOG are all updated. Nothing client-named.

1. **`authenticate` is cleverer than it needs to be** (`src/app.rs`, ~3139-3176). `let (message, cleared) = if bearer.is_none() && cookie.is_none() {..} else {..}` followed by `if let (true, Some(name)) = (cleared, &cookie_name)` is a tuple-and-bool dance. Plain version: compute `message` from "was any credential presented", then `if cookie.is_some() && let Some(name) = &cookie_name { push Set-Cookie }`. `cleared` is just `cookie.is_some()` and `cookie` is already `None` when `cookie_name` is `None`, so the pair match is redundant. The `by_token` closure borrowing `self` and then `self.refuse` after it also reads as fragile; a small `fn device_for(&self, token)` would be plainer.

2. **The cookie prefix is written twice.** `TOKEN_COOKIE_PREFIX = "switchboard_token_"` in `src/adapters/remote.rs` and the literal inside `format!` in `core::remote::token_cookie_name`. Define the prefix once (a `pub const` in `core::remote`, which the adapter can import) so the name builder and the header filter cannot drift. Same for the page paths: `PAGE_PATHS` in the adapter (rate window) and the `match` arms in `page_reply` in `app.rs` list the same three strings. The doc comment admits it ("listed here for the rate window"). Say it once, or have `page_reply` look up in a shared table.

3. **Needless clone** (`src/app.rs`, `/pair`): `reply["token"] = token.clone().into();`. `token` is not used afterwards, so `token.into()` suffices. The `let mut out = ...; out.headers = headers;` tail could be a struct update or a `.with_headers` helper, but this one is just taste.

4. **Doc comment placement.** `TOKEN_COOKIE_PREFIX` sits between `read_request` and `respond_upgrade`, away from the other constants, after its use. Move it with the other consts. `RemoteReply` and `RemoteRequest` fields: the plan's contract additions are documented on the struct, but `headers` has no per-field doc beyond the struct comment. That is acceptable, but a one-line field doc matches `cookies`.

5. **Repeated `headers: Vec::new()` in ~12 literals in `app.rs`.** This is what the plan specified, so I am not asking for a change. If `RemoteReply` had a small constructor (`RemoteReply::json(status, body, authenticated)`) the next field would not touch every site. Optional, recorded for the owner.

6. **`web/app.js`.** Reads fine and has no inline narration. One check: `boot` returns "Not paired" when `localStorage` has no `server_id` and never tries `/waiting`, so a browser whose storage is blocked (the case `writeStored` comments on) cannot use its valid cookie. The comment says "this visit still works", but in this path `readStored(STORE.server)` is read right after `pair` wrote it, so it does not. Either the comment or the check is wrong; this is a correctness point I am leaving to the code reviewer, but the comment as written is misleading.

7. **design.md / spec wording.** `docs/remote-client.md` "Browser trust" says "the paired-devices dialog says so" about an impostor page not being a genuine page. The dialog sentence (`src/ui/remote.rs`) only says the warning is the trust step and may return; it does not say that. Soften the spec or add the clause to the dialog.

Not raised: `pub mod remote` in `core/mod.rs` is needed by the UI and tests; the `#[must_use]` and doc comments on the new public items are present.
