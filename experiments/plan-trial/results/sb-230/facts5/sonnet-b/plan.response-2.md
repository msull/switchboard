# Response to plan.feedback-2

Earlier points 1-4: no change, closed.

Cuts, all made in plan.md (rewritten shorter, now 291 lines):

- **Where this departs.** The excerpt bullet and the "Compare the key" bullet are gone as bullets. The excerpt fact is one clause in "The UI"; the key-comparison limit is one parenthesis in "The signals". The "code in the query" bullet is kept as a one-liner and the duplicate reason in decision 3 is trimmed to the fragment fact; the "`?` would have worked" history is cut.
- **Decision 1.** Vendor-text recap cut; the stop condition stays.
- **Decision 2.** "Not scoped by port", the dialog line, the subprotocol sentence and the upgrade/socket clause are cut.
- **Decision 3.** Percent-encoding sentence and `read_request` history cut.
- **Decisions 5 and 6.** Removed as separate items; the file list sits in decision 4 and the device is covered in "The record".
- **The record.** Schema paragraph is now "No record changes, no schema bump." The Identity bullet is merged into the Certificate bullet. Pools: the unresolved-connections prose and the 10 s idle are removed; the pool of 6 and 120/min rate stay, idle stays 30 s.
- **The signals, socket.** Cut from this commit as asked, with its `watch`/`unwatch`, debounce, backoff, slow-down, the upgrade branch of Authenticate, the cookie-on-upgrade Origin test, and the `wss` clause in decision 4. It is a follow-up (and spike 23 still records whether a `wss` opens). "Authenticate" keeps only the Origin check on cookie non-`GET`s, without the hijack sentence.
- **Request, Reply bullets.** The 8 KiB sentence, the 30-sites grep and the reason-table line are cut.
- **`/pair` bullet.** The 64-character cut is dropped and not moved to the tests.
- **The UI.** The `<meta name="referrer">` is dropped (the header in decision 4 stays); the per-file list is cut.
- **The tests.** Cut: the "page references only served paths" test, the `Copy invite` test, "existing tests pass untouched", and the Swift list shortened to the two cases. The acceptance paragraph is kept, shortened: the task requires the acceptance steps to be stated, and the follow-ups do not carry them.
- **Docs and order.** The "Order" sentence is cut.
- **Contract additions.** Kept, as a short list at the end, not cut: the task I was given requires the plan to end with a one-line commit message and the contract-additions list for the implementer to copy. It is now five lines, with the socket-related 403 wording removed.
