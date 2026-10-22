1. Resolved: the static arm now returns after its write, and the Tests list checks for EOF.
2. Resolved: every reply on an unpaired connection closes, and the page's empty icon stops the favicon request.
3. Resolved: the cookie and the bearer are held as a `Secret` whose `Debug` prints `<redacted>`, with a test.
4. Not resolved: the plan now calls browser `control` pairing "the owner's recorded choice", but no owner has made that choice; see point 10.
5. Resolved: the JSON branch stays in the parser, the URL is a second form, and the CHANGELOG line says so.
6. Resolved: a cookie request that is not a `GET`, and every upgrade, must carry a matching `Origin`, with tests.
7. Resolved: the certificate re-issue goes through `save_identity` and the atomic write, and the key comes from `remote-key:<server_id>`.
8. Resolved: the owner runs the macOS rows, results are recorded in `spikes/23-browser-client/README.md`, and no code is written before rows 1 to 6 are recorded.
9. Resolved: `src/app.rs:3024` and `:359` are both correct in the worktree.
10. New: the plan records a decision it cannot make. "A recorded owner decision, not an oversight" and "the owner's recorded choice" in the Trust paragraph, the Documents bullet and the `docs/remote-client.md` §5 text assert an owner approval that does not exist, and the feedback asked for an explicit owner decision, not a plan statement that one was made. Mark the browser `control` risk as an open decision the owner must confirm before the spec text is written, or cap browser pairings at `read` until they do.
