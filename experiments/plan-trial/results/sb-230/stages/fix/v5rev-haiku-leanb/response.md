# Response to review round 1 (head f693952)

One fixup commit, `fixup! Serve a web control surface…` (13eabd0), amends f693952. It changes no name the implementation message lists, so it is a fixup rather than a squash.

Checks on the branch: `cargo fmt --all` clean, `cargo clippy --locked --workspace --all-targets -- -D warnings` clean, `cargo test --locked --workspace` all green (including `every_route_the_page_names_is_served` and `the_page_logic_passes_its_node_tests`, which runs `node --test web/`). Nothing in the change touches a `cfg(target_os)` arm, so the Linux dispatch check was not run.

## feedback-1 (correctness)

### 1. `web/app.js` `openSession` crashes or leaks when the user leaves mid-load — fixed

`openSession` now keeps the session object it created (`mine`) and checks `state.session === mine` after each await before it reads or writes page state. The 401 check still runs first, since it applies whatever the reader is looking at. `loadContent` takes the session as a parameter instead of reading `state.session`, so a late reply writes only into the object it was started for. `refreshAll`, `sendReply` and the render calls after their awaits are guarded the same way, because they had the same read-after-await pattern (`state.session.view = …` after an await would throw once Back cleared it). The two sequences in the review (Back during load; Back then B) now leave B's page showing only B's data.

### 2. `loadList` stamp order and concurrent `refreshAll` — fixed (honouring the plan's rule)

The review said no action was needed, but the plan's "a reply older than the one applied is dropped" rule was only applied to session content. `loadList` now returns early when the waiting read's stamp is not newer than the list already applied (`isNewer(state.list.stamp, waiting.stamp)`), so an older slow read cannot overwrite a newer one. An equal stamp still applies, as before.

### 3. Test that every `/api/v1/` route in `ROUTES` is served — added

The review asked me to add or confirm it. It did not exist. `tests/remote.rs` now has `every_route_the_page_names_is_served`. It reads the `ROUTES` table from `web/app.js`, substitutes a seeded session id (or project id for `/projects/{id}/sessions`), pairs a control device, and asserts that no route answers `not_found`. `send` and `pair` are sent as POST, the rest as GET. The test is read from the page's source, so a route the page adds is checked without changing the test.

## feedback-2 (style)

### 1. `authenticate` nests bearer and cookie paths — fixed

`authenticate` now reads `match &request.bearer { Some(token) => token.as_str(), None => self.cookie_token(request)? }`, then looks up the device. No `token.clone()`. The cookie arm is `cookie_token(&self, request) -> Result<&str, RemoteReply>`, which borrows the token from the request (no allocation) and then calls the origin check. The origin check is its own step, `check_origin`, with its own doc line. The `same_origin` closure with an inline `format!` is replaced by a `match` on `(host, origin)`. Order and outcomes are unchanged: a missing cookie is 401 before the origin check, and bearer requests skip the origin check as before.

### 2. `no_identity` 503 block duplicated — fixed

`App::no_identity(&self) -> RemoteReply` is the one helper, used by `remote_info` and `remote_me`. `remote_pair` keeps its own `refuse` with a different message and logs the refusal, so it is not changed. I left it because folding it in would change its message and its logging.

### 3. Public items without doc comments — fixed

- `Page`: its three fields each have a doc line.
- `PAGES`: a doc line.
- `CSP` already had a doc comment, so it is unchanged.
- `RemoteReply::new`: a doc line.
- `RemoteRequest` additions: unchanged, as the review said they were fine.

### 4. Stale spec sentence — fixed

`docs/remote-client.md` now lists only the last response as masked and served. The sentence about the card excerpt, which is not on the port, is unchanged.

### 5. Host choice in the UI — kept in `ui/`, notice text named

I did not move the address choice into `core::remote`. The review said it was small and acceptable as is, and the choice (LAN address or loopback) is a presentation decision for the pairing dialog with no state of its own. The two three-way notice strings that were inline are now the consts `NO_LAN_ADDRESS` and `LAN_OFF` in `src/ui/remote.rs`. If the choice grows, it moves into `core::remote` next to `invite_url`.

### 6. Spike row in the spike table — reworded

The `23-web-page` row in `spikes/README.md` now says "Not a measurement: a device checklist (`spikes/23-web-page/README.md`) that no browser has run yet", so it does not sit as a measured result.

### 7. `refresh` relies on `isNewer` being true for an equal stamp — fixed

`web/app.js` has `isAhead(applied, incoming)`, which is `isNewer(applied, incoming) && !sameStamp(applied, incoming)`, with a doc line saying it is strictly newer. `refresh` now reads `if (!waiting.ok || !isAhead(...)) return;`. `isAhead` is exported and covered by three assertions in `web/app.test.js`. `sameStamp` stays exported because `isAhead` and the existing test use it.

### 8. Unit test with two tests in one — split

`src/adapters/remote.rs` now has `a_request_reads_its_host_origin_and_cookie` (header parsing, plus an assertion that the cookie header is read) and `a_cookie_is_matched_by_its_exact_name` (the name format and `cookie_value`'s matching cases).
