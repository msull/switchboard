1. Trust step: resolved. The plan now states the weakening of §5 as a deliberate exception that needs the owner's sign-off, gated at step 0. No further objection.
2. JSON invite: resolved. "Copy invite" keeps the JSON and the QR carries the URL. One leftover from this change is raised as point 10.
3. Reachability: resolved. The QR is suppressed on loopback, and there are tests for both cases.
4. Origin rule: resolved. It now covers every cookie request, with `Host`, `Sec-Fetch-Site` and `Origin` checks and tests.
5. Shared cookie: resolved in principle. Point 11 is a new detail the fix introduced.
6. `/me` fixture: resolved.
7. Rate limit: resolved.
8. Spike gate: resolved. The cookie rows now gate step 3, and the commit is conditional on them.
9. Docs and process: resolved. The `include_str!` note, the recounted literals and the `design.md` section are all handled.

New points, each created by one of the answers above:

10. The README bullet in "Docs" still says `"Copy invite" puts a URL on the clipboard`. That contradicts Decision 3 and the UI test, where "Copy invite" stays JSON and "Copy link" is the URL. Change the bullet to say "Copy link" copies the URL and "Copy invite" still copies the JSON. The CHANGELOG line is fine.
11. The cookie name says `<server_id, 32 hex>`, but `server_id` is a `Uuid` (`src/adapters/remote.rs:96`). Its `Display` is the 36-character hyphenated form, not 32 hex. The name is wire contract, shared by `Set-Cookie`, `read_request`'s `__Host-sb-` prefix filter and `authenticate`, so pin one form (`simple()` or hyphenated). Also add the fixed length to the 64-byte cap and to the two-hosts test.
