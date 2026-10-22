# Response to the review of the #230 plan

Edits are in `/Users/sbgragent/Library/Application Support/Dispatch/tickets/f53f3e3e/review/1/repo/plan.md`. The earlier plan is untouched. Each edit was confirmed present in the file after writing, and each new line cite was checked against the worktree.

1. **`wire::Body::Device`: accepted.** `GET /api/v1/device` is built with `json!` and no Rust type is added. Changed: "Actions and effects", "Routes", "Contract additions". Your cite was partly off: `Cargo.toml` has no `wire` crate, and `Body` in `src/app.rs` is imported from `ports::dispatch` (`src/app.rs:42`). The plan now cites that, and says the control port's `Body` is not touched.

2. **Connection loop: accepted.** `Connection: close` ends nothing; the `while let` at `src/adapters/remote.rs:783` continues after each reply. Changed: "Pool". The page arm returns `Ok(())` from `connection`, and the slot is released at the accept thread (`src/adapters/remote.rs:719`). The page's rate-limited branch returns too; the `continue` at 788–794 stays for `/pair` and `/info`. Changed: "Tests", `page_connections_free_their_slots`, which now requires each client to read to end of stream.

3. **`Host` in the CSP: accepted.** Changed: "Transport", the `Host` check. A `Host` that is missing, or holds anything but `[A-Za-z0-9.:\[\]-]`, gets 400 before any CSP is built. Changed: "Tests", new `page_host_is_validated`, which covers a missing `Host`, `;`, a space and `'`.

4. **Rotation signal: accepted.** Changed: "Revoked, rotated and identity". The plan now cites the key-missing path (`src/app.rs:2810–2816`: `make_identity` with the same `server_id`, then `devices.clear()`), the no-identity path (`src/app.rs:2836–2839`), and the read-only branch (`src/app.rs:2807`), which keeps both. It also cites `RevokeDevice` at `src/core/remote.rs:86`.

5. **Fallbacks: camera accepted; the rest answered.**
   - *Camera:* changed, "Pairing URL", new "Camera fallback" paragraph. If the iOS camera won't open the URL, the window keeps the JSON QR for native paste and the CLI, and the phone types the URL shown beside it.
   - *`wss://`:* the polling fallback already sits in "Spike first", unchanged.
   - *Storage:* no change. "The page" ends with the wrapped `localStorage` behaviour, which covers storage that throws. `sessionStorage` stays the owner's choice, as the plan says.

6. **Subprotocol and failed open: accepted.** Changed: "Transport" (the token alphabet cite, `src/adapters/remote.rs:334–340`; your cite `core/remote.rs:337–341` was wrong, the generator is in the adapter). Changed: "Revoked, rotated and identity", which says a failed open gives no status, so every failed open runs the `/info` check. Changed: "The page", item 8, to say the same.

7. **Checks against CLAUDE.md and design.md.**
   - Body: see 1.
   - Line refs: `src/core/remote.rs:170` corrected to `169` in "The record". Your "~168" is also off; the trim is at 169.
   - Additive contract, schemas, the `respond_upgrade` signature: no change, as your check says.
   - `web/` and `remote_page.rs`: changed "Order of work". This was not in your feedback but was a real defect. Step 2 used `include_str!` on `web/app.js`, which step 5 created, so the build would have broken at step 2. The three files now land in step 2, and the README rows land with them. The `Host` parsing moved into step 2 for the same reason, since the CSP depends on it. Steps are renumbered, and the spec, README and changelog are step 5.
