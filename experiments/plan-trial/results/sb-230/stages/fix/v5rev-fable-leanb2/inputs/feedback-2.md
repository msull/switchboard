Every point below is wording or small tidiness; none blocks. The layering holds: `invite_url` is pure in the core, `web.rs` is a constants table with no I/O, nothing new is under `ui/` beyond calling it, and README and design.md are updated.

1. `src/app.rs` (`authenticate`): `by_cookie = request.bearer.is_none()` is named for the credential used. It is true before a cookie has been found, and it only means "cookie" because the early 401 returns first. Compute it after `token`, or name it `no_bearer`. The `Some(token) => Some(token.clone())` arm clones a string that `request.bearer.clone().or_else(...)` would give directly.

2. `src/app.rs` (`NOT_ON_PORT`, and the `("GET", ["events"])` move into the match): both are churn outside the plan. The const has one use and the old inline literal was fine. The `events` arm moved from a pre-match `if let` into the match, and that was needed only to sit beside `me`. Keep the second, since `me` needs the same early return, but revert the const. The commit message should not need to explain it.

3. `src/adapters/remote.rs` (`respond` and `respond_page`): both hand-build a status line, headers and body with the same `format!`, `write!`, `push_str` and `extend_from_slice` shape. Simple enough, but the two will drift. Consider one `write_head(status, content_type, len, connection, &headers)` helper. If you leave it, say in `respond_page`'s doc why it does not share `respond`.

4. `src/adapters/remote.rs` (rate constants): the doc comment above `PAIR_PER_MINUTE` now says "`/pair`, `/info` and the web page's files per minute... A page load is three requests", but it sits on the 6 limit only. `PAGE_PER_MINUTE` is `pub` with no doc comment. Put the page sentence on `PAGE_PER_MINUTE`, document it, and say why it is `pub` (the test uses it).

5. `src/adapters/web.rs`: `Page`'s fields and `PAGES`, `CSP`, `HEADERS` and `page` are public. They have docs, but `Page`'s three fields do not, and CLAUDE.md asks for docs on public items. Either add one-liners or make the fields `pub(crate)`.

6. `src/ui/remote.rs`: `MAC_ONLY_NOTICE` is `pub` and `BROWSER_NOTICE` is private, with no stated reason. If the test needs the first, say so in a clause. The `on_lan` local duplicates the `listen.lan` filter inside `invite_url`. The UI repeats a decision the core already makes, so a small core helper (`reachable_address`) would keep it in one place. This is minor, but CLAUDE.md says "if logic appears under `ui/`, move it to the core".

7. `src/app.rs`: the `COOKIE_MAX_AGE_SECS` comment says "400 days, the most a browser honours", which is accurate for Chrome, and the plan says the same. Say "Chrome's cap" or drop the claim, because Safari behaves differently.

8. `docs/design.md`: the new paragraph is a single 30-line bullet. The surrounding status entries are shorter. Splitting it into "what is built", "what rests on the owner's phone" and "known gaps" would match the rest. I did not verify that `docs/remote-client.md` line 207 (the excerpt correction) was edited as the plan says; the diffstat shows the file changed, so the author should confirm.

No debris found: no unused value, no doubled doc comment, no leftover block from moved code.
