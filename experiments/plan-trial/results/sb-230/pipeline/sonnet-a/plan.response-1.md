# Response to plan.feedback-1.md

1. **Stale Swift line.** Fixed. Confirmed in the worktree: `struct PairingInvite`
   is at `Info.swift:94` and `parse` at `:114`. Changed in "The actions and
   effects", the Swift bullet.

2. **`pairing_invite` and `local_address` split.** Fixed. "The actions and
   effects", the `src/core/remote.rs` bullet, now gives the signature
   `pairing_invite(&Identity, &str, Option<SocketAddr>) -> String`, says the UI
   keeps calling `remote::local_address()`, takes the port from
   `cx.remote_port.unwrap_or(identity.listen.port)` (`src/ui/remote.rs:157`),
   builds the `SocketAddr` and passes it in, so no adapter function enters
   `src/core/`. The core test bullet in "The tests" and the contract line are
   updated to the same signature, so the IPv6 bracketing test is writable.

3. **Cookie lifetime against a token that does not expire.** You are right; the
   old sentence ("an expired cookie is a re-pair") contradicted the spec. Chose
   the re-issue option: `GET /api/v1/device`, which the page calls once per
   load, answers with `Set-Cookie` and a fresh `Max-Age` when the request came
   by cookie. Changed in Decisions 2 (the `Path/Max-Age` bullet) and in "The
   actions and effects" (`serve_remote` bullet), with a test added in "The
   tests". A device that never opens the page for 400 days still lapses; that
   orphan row is accepted and stated, since the dialog already shows
   `last_seen` and Revoke.

4. **The code in a URL.** Fixed. Decisions 4 now has "The code in a URL": it
   says §5's "never in a URL" stays true of the token only, lists where the
   code now lives (QR, clipboard, request line, first history entry before
   `replaceState`), states the limit (40 bits, single-use, three failures, five
   minutes) and that logs record the path only. The header set is now on every
   answer to a table path, 405 and 429 included, because `connection` builds
   those; `respond` adds `no-store`, `nosniff` and `no-referrer` to every API
   answer too. The §5 docs bullet lists the new statement.

5. **Headers on 429 and 405 untested.** Fixed. "The tests", first bullet: one
   helper asserts the full header set and is applied to the 200, the 405 and
   the 429; the API answers are asserted for the three shared headers.

6. **Unauthenticated serving.** Stated, plan unchanged in behaviour. Decisions
   4, "Always on, deliberately": the table is served with or without a window
   or a token, because gating on the window would break an already-paired
   phone's reload. What a stranger gets is source that is in the public
   repository and no host data; `/info` already identifies the app to anyone.
   §5's pre-pairing sentence is widened to the three files, and a test asserts
   the table is served with no window and no token.

7. **Per-listener static window.** Accepted and said. Decisions 4: the window
   is per listener like `/info`'s and `/pair`'s; a stranger can cost the owner
   a one-minute retry of the page load but cannot touch paired API calls (the
   window covers only table paths). Keying by peer would add a map an attacker
   can grow, so it was not done.

8. **`/device` and the allowlist test.** Checked rather than assumed. The
   allowlist test (`the_allowlist_refuses_everything_else_with_no_side_effect`,
   `tests/remote.rs:682`) lists no `/device` path, so it does not fail and is
   not changed. The test that does need a line is
   `info_is_served_to_anyone_and_every_other_read_is_refused_unpaired`
   (`:426`), whose unpaired-401 list at `:445` gains `/api/v1/device`; its
   `/other` 404 stays true and no test requests `GET /`. Both facts, and the
   §4.3 table row, are in "The actions and effects", the `serve_remote` bullet.
