# Style review, round 1: stage/sb-230-implement-v5rev-haiku-leanb (7fe8949..HEAD)

Not all wording. The first three points are structure or missing doc comments. The rest are small.

The layering holds. `invite_url` is a pure function in the core with a core test. The pages are answered on the connection thread and the UI only draws. README (Layout rows, dev-aid sentence), design.md, CHANGELOG, `docs/remote-client.md` and `spikes/README.md` were all touched. I found no doubled doc comments and no narrating comments.

## Points

1. **`authenticate` (src/app.rs, ~3126) nests the bearer path and the cookie path in one `if let … else`.**
   - The bearer arm clones the token only so both arms yield a `String`. The cookie arm builds the cookie name, reads the cookie, checks the origin and returns the token, all inside the `else`. The `same_origin` expression with a `format!` inside `is_some_and` is the "clever" kind CLAUDE.md asks to avoid.
   - Pull the cookie arm out into a helper such as `fn cookie_credential(&self, request) -> Result<String, RemoteReply>`. Do the origin check as a named step with its own doc line.
   - `authenticate` then reads: bearer, else cookie, then look up the device. The `token.clone()` goes away.

2. **`remote_me` copies the `no_identity` 503 block from `remote_info`** (src/app.rs, ~3591 and ~3712).
   - The same `error_json("no_identity", …)` literal now appears twice. Make one small `no_identity()` reply helper and use it in both.

3. **Public items without doc comments.**
   - src/adapters/web.rs: `Page`'s three fields, `PAGES`, and `CSP` (the struct and the const do have docs).
   - `RemoteReply::new` in src/adapters/remote.rs has a `#[must_use]` and no doc line.
   - The `RemoteRequest` additions are fine.

4. **Stale spec sentence.** docs/remote-client.md:208 still lists "the card excerpt" as a masked, served thing ("last response and the card excerpt"). Line 214 now says the excerpt is not on the port. Reword 208 to match.

5. **Host choice sits in the UI** (src/ui/remote.rs, `pairing`).
   - `identity.listen.lan.then(remote::local_address).flatten()` followed by `unwrap_or(127.0.0.1)` and a three-way notice is a decision about which address to offer. CLAUDE.md says "if logic appears under `ui/`, move it to the core."
   - It is small, so I would accept it. If it grows, move the address choice into `core::remote` next to `invite_url`.
   - The new 200-character warning string is long inline. Consider a `const`.

6. **Spike without a spike** (spikes/23-web-page).
   - CLAUDE.md says to add a spike for any risky mechanism before building on it. The README is a device checklist with no code and "Status: not run". The cookie-on-self-signed-certificate question is the one the plan called risky.
   - It is honest, and design.md's gaps line says so. But the `spikes/README.md` row sits in a table of measured results with a "Not run" answer. Say in the row that it is a checklist and not a measurement, or keep it out of the table until a device has run it.

7. **web/app.js, `refresh`** (~605): `sameStamp(...) || !isNewer(...)`.
   - It works only because `isNewer` is true on an equal stamp (`>=`). A reader has to know that. Either name the combined test (`isChange(applied, incoming)`) with a one-line why, or drop `sameStamp` from the exports if only that call uses it.

8. **tests/remote.rs, unit test in adapters/remote.rs** (`a_request_reads_its_cookie_origin_and_host_and_a_cookie_is_matched_by_name`).
   - It is two tests in one (header parsing, and `cookie_value` matching) with a name that says so. Split them.
