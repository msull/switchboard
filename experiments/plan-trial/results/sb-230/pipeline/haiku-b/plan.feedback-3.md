1. Resolved: the static arm returns after its write, and the Tests list checks for EOF.
2. Resolved: every reply on an unpaired connection closes, and the page's empty icon stops the favicon request.
3. Resolved: the cookie and the bearer are held as a `Secret` whose `Debug` prints `<redacted>`, with a test.
4. Resolved: browser pairings are capped at `read` in `remote_pair`, and browser `control` is listed as an open decision the owner must make before the §5 text is written.
5. Resolved: the JSON branch stays in the parser, the URL is a second form, and the CHANGELOG line says so.
6. Resolved: a cookie request that is not a `GET`, and every upgrade, must carry a matching `Origin`, with tests.
7. Resolved: the certificate re-issue goes through `save_identity` and the atomic write, and the key comes from `remote-key:<server_id>`.
8. Resolved: the owner runs the macOS rows, results are recorded in `spikes/23-browser-client/README.md`, and no code is written before rows 1 to 6 are recorded.
9. Resolved: `src/app.rs:3024` and `src/adapters/remote.rs:359` are both correct in the worktree.
10. Resolved: the same point as 4; the "recorded owner decision" wording is gone from the Trust paragraph, the Documents bullet and the §5 text.
11. New: the Trust paragraph says a relay that gets past the browser's check "reaches read access only", which understates the cap. `read` lets a device read every view, including transcripts and redacted screens (`docs/remote-client.md:304`, "expose private transcripts"). Say that a read-scope browser pairing exposes the same transcripts a read-scope headset does, so the owner's decision on `control` is not the only one that matters here.
