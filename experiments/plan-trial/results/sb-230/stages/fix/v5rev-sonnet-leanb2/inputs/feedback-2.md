# Style review, round 1 (head 4b65af5)

Not wording-only: items 1–2 are structure, the rest are small. Nothing below is a Style-rule violation in the core (no I/O or clock added there), and the comments I read explain why rather than narrate.

## Should change

1. **Address choice is logic under `ui/`** — `src/ui/remote.rs:~159–161`. `remote::local_address().filter(|_| identity.listen.lan)` falling back to `127.0.0.1` decides what the invite names, and the "Mac-only" branch below keys off the same `Option`. CLAUDE.md: "If logic appears under `ui/`, move it to the core." The plan put the URL building in `invite_url` so a test could call it, but the rule it encodes (loopback unless the port listens on the network) is what most needs a core test, and the core test only covers the formatting. Move the choice into the core (for example `invite_url` taking `Option<IpAddr>` for the LAN address plus `identity.listen.lan`, or a small `invite_address` beside it) and have the UI only draw.

2. **README `spikes/` line is mangled** — the new entry `23-web-page (…; not yet run)` is inserted before `13-dispatch-service`, with a double space, so the list is out of order and reads as if it were a different sentence. Put it at the end (before the final "and" it would become "…21-evidence-writes … and 23-web-page …"), single-spaced. The spike README itself is fine and honest about "not run".

## Simplify

3. **`write_response`'s header type forces an allocation per page reply** — `src/adapters/remote.rs` `respond_page` builds a `Vec<(&str, String)>` from `web::HEADERS` (`&str, &str`) only to match `write_response(&[(&str, String)])`, and `respond` does the opposite clone. Pick one header type (`&[(&str, &str)]` would do for both; `RemoteReply.headers` is the only owner of a `String` value, and `respond` can pass `reply.headers.iter().map(|(n, v)| (*n, v.as_str()))` or take the vec directly). Right now two conversion shims exist to bridge one function.

4. **Nested ternary in `deviceName`** — `web/app.js:43–51` is four levels deep while the device half three lines below is a plain `if/else if` chain. Write the browser half the same way; it is the clever-where-simple-would-do case.

5. **`page.marker` duplicates `page.applied`** — `web/app.js:108–109, 289, 300, 439`. `marker` is the string `epoch:revision` of `applied`, kept in step by hand and compared in `tick`. Drop `marker` and compare `epoch`/`revision` against `page.applied` (the plan's wording: "compares its `epoch` and `revision`"). One value, nothing to drift.

## Minor

6. **`web.rs` public fields undocumented** — `Page { path, content_type, body }` are `pub` with no doc lines while the struct and consts have them. One line each, or make them private with the module's two accessors if nothing outside needs them (`respond_page` reads `content_type` and `body`, so the fields do need to be pub; document them).

7. **`remote_me` hides a missing identity** — `src/app.rs` `remote_me` does `.unwrap_or_default()` and answers 200 with an empty `server_id`/`server_name`. `remote_info` right beside it answers 503 `no_identity` for the same condition; the authenticated path cannot reach it in practice (a device implies an identity), so either follow `remote_info` or say why the default is safe. Not worth a round on its own.

8. **`#[allow(clippy::too_many_lines)]` on `serve_remote`** — added with the two-line `/me` arm. The function is now over the limit because of accreted arms, not this change alone, but the allow is a new debris-shaped line; if the lint fires only by a hair, moving the `/me` and `/events` early-returns into a helper is the cleaner fix. Leave it if you prefer; flagging only.

## Checked, no point

- Docs: README Layout rows (`web.rs`, `web/`), dev-aid sentence, design.md status paragraph, remote-client.md §0/§4.1/§4.3/§5/excerpt line, CHANGELOG line are all present and match the code. The excerpt claim corrects the earlier doc rather than restating history.
- No doubled doc comments, no unread fields, no leftover blocks from moved code found in `remote.rs`, `app.rs`, `web.rs`, `core/remote.rs`. `core/mod.rs` re-exports `cookie_name`/`cookie_value`/`invite_url` additively as the contract section says.
- No client or customer names in the diff.
