# Style review, round 1 (7fe8949..ecffe04)

Nothing here blocks: every point is small cleanup or wording, and none is a layering breach. The core stays pure (`invite_url` takes its inputs), the web page is under `web/` with no logic in `ui/` beyond the one item in point 1, and README, design.md, remote-client.md and CHANGELOG are all updated.

1. **Logic under `ui/` (`src/ui/remote.rs`, `pairing`).** The rule "the invite names a network address only while `listen.lan`" lives in the dialog (`identity.listen.lan.then(remote::local_address).flatten()`). The empty arm `None if identity.listen.lan => {}` means "on the network but no address found", and nothing explains it. The `SocketAddr::new(ip, port)` formatting is also done again beside the one in `core::invite_url`. Have the UI hand `local_address()` to the core only when `lan` is set, or give the core a small fn that returns the display address, so the "Address" line and the URL cannot drift. At the least, comment why the empty arm is right, and collapse the match to `if let` plus an `else if !lan`.

2. **The cookie name is built twice (`src/app.rs`).** `cookie_name()` is the one place that knows `__Host-sb-<server_id>`, but `remote_pair` rebuilds it in a `format!` for `Set-Cookie`. Reuse the helper (or a free fn taking `&Identity`) so the read and write sides cannot disagree. The comment on `cookie_name` is the right place for the "why", and the pair site should point at it.

3. **`authenticate` is clumsier than needed.** `(token.clone(), false)` clones a `String` the request already owns. The nested `match` inside `match` with the `return Err(...)` three levels down reads worse than `let (token, by_cookie) = if let Some(t) = &request.bearer { (t.as_str().to_owned(), false) } else if let Some(t) = self.cookie_token(request) { (t, true) } else { return Err(...) };`. Borrow instead of cloning. The doc comment is long, and half of it restates the `Origin` rule that `docs/remote-client.md` already carries. Keep the why (cross-site is withheld by `SameSite`, this closes same-site other-port) and drop the rest.

4. **Unrelated churn in `src/app.rs` (`remote_dispatch`).** `notes_cap` was split into `let alone = matches!(...)` plus a one-line `if`. Nothing in the change needs it, and it is a reformat of code the diff otherwise does not touch. Revert it.

5. **Wording and placement in `src/adapters/remote.rs`.**
   - The `respond` doc says every reply is "marked uncacheable and its type final, which no client reads and a browser honours." That is hard to parse. Say what the headers are for: `no-store` and `nosniff` so a browser never caches or sniffs session text.
   - `write_response` has no doc comment. It is private, but it now carries a five-argument contract (`extra` is pre-formatted header lines, `close` toggles `Connection`), which deserves one sentence.
   - `respond_error`'s doc, "One such error, written.", refers to the item above it and breaks if the fns are reordered. Name the thing: "Write a JSON error reply with this status and code."
   - `RATE_LIMITED` sits between two fns. Put it with the other limit constants (`PAIR_PER_MINUTE` and friends).
   - `respond_page` sets `Referrer-Policy` and CSP but not the content type or close flag, which `write_response` takes as arguments. This is fine, but the doc should say that the common headers (`Cache-Control`, `nosniff`) come from `write_response`, so a reader does not look for them here.

6. **`web.rs` `Page` fields are public with no field docs.** The struct doc covers them, so this is wording only. `PAGES` is `pub`, but only `page()` and the test use it. Make it private unless `tests/remote.rs` reads it. The plan has the test read the table, so keep it `pub` if that is so.

7. **`origin_is_own` is an associated fn with no `self` (`src/app.rs`).** A free fn next to the other request helpers, or a method on `RemoteRequest` (`request.origin_is_own()`), matches how `RemoteRequest::param` is already done. The `.map(str::trim)` calls on header values that `read_request` has already trimmed are noise; drop them or note why they are needed.

No README or design.md gaps found: the Layout rows for `web/`, `web.rs` and the spike are present, the dev-aid sentence is updated, and the design.md status paragraph lists its known gaps.
