# Response to review round 1 (head dc1c58a)

Fix commit: `7a22de4` (`fixup! Remote port serves a browser page paired from the invite URL`), one commit with `--fixup=dc1c58a`. No contract name in dc1c58a's message was renamed or removed, so no squash.

Checks run: `cargo fmt --all`, `cargo clippy --locked --workspace --all-targets -- -D warnings` (clean), `cargo test --locked --workspace` (all pass, remote suite included). Not run: the Linux `dispatch` clippy check, since no `cfg(target_os)` arm changed. `node --check web/app.js` was refused by the approval gate, so the page script is checked only by reading and has not been run in a browser.

## feedback-1 (correctness)

**1. Socket retried on every poll (fixed).** `refreshList` no longer calls `connectSocket()`. `boot` opens the socket after its first list, and `tick` retries it every sixth tick (30 s), as the plan's step 5 says. A failed socket is no longer reopened on each 5 s poll.

**2. `hello` triggers a second full list fetch (partly fixed).** The two fetches no longer run concurrently. The boot fetch is awaited before the socket opens, and `refreshList` now takes a sequence number: only the newest started call applies its reply, so an older reply cannot overwrite a newer list. I kept the `hello` refresh rather than deleting it. The boot snapshot is taken before the subscription exists, so the `hello` refresh is what closes the gap between them. Dropping it would leave a change made in that gap unseen until the next change event. So each connect still makes one extra fetch, but serially. The reconnect-storm concern is covered by the 30 s retry.

**3. "did not keep its pairing" for a later 401 (fixed).** `failed()` now reads and clears `justPaired` on any answered reply (`status !== 0`). A network failure leaves the flag set, so the first answered request decides the message. The success path still clears it too.

**4. Stale QR with no address (fixed).** In `src/ui/remote.rs` the pairing window sets `draft.qr = None` when there is no invite. It rebuilds the QR only when the stored code differs from the current one. No test was added: the UI harness cannot easily control the local address, and the change is a two-arm match.

## feedback-2 (style)

**1. `authenticate` is too clever (fixed).** The tuple-and-bool match is now `let message = if … else …`, and the `Set-Cookie` push is `if let Some(name) = cookie_name.as_ref().filter(|_| cookie.is_some())`. The `by_token` closure is now `fn device_for(&self, token)`, so `self` is not borrowed by a closure across the `refuse` call.

**2. Prefix and page paths written twice (fixed).** `TOKEN_COOKIE_PREFIX` is a `pub const` in `core::remote`. `token_cookie_name` builds from it, and the adapter's header filter reads it. `PAGE_PATHS` is gone: `web::PAGES` is one table of `(path, content type, body)`. `page_reply` finds its entry there, and the rate window's `Limits::limited` checks membership in it.

**3. Needless clone (fixed).** `reply["token"] = token.into();`.

**4. Doc comment placement (fixed).** The prefix constant now sits with the cookie name builder in `core::remote`, with its doc comment. `RemoteReply::headers` has a one-line field doc.

**5. Repeated `headers: Vec::new()` (not changed).** Declined: the plan specified the struct literal, and a `RemoteReply::json` constructor would change the shape the plan names. It is noted for the owner; no change this round.

**6. Boot comment is misleading (fixed, as a code fix).** The comment was right and the code was wrong. After a pairing, `boot` read the stored server id, which a blocked storage write never sets, so the page showed "Not paired" after pairing. Now the stored-id check runs only when there is no invite. The scope comes from the pairing reply (`state.scope = r.body.scope` in `pair()`), not from storage. So a visit that pairs with storage blocked works with its granted scope, and the next visit pairs again as the `writeStored` comment says.

**7. Spec claim about the dialog (fixed).** `docs/remote-client.md` said "the paired-devices dialog says so". The sentence is in the pairing window, and it says only that the warning is the trust step. The spec now says that.

## Not reproduced

Nothing in feedback-1 was left unaddressed. The points marked "fine" in feedback-1 (cookie auth order, `a_page_load_fits_the_stranger_pool`) were not touched.
