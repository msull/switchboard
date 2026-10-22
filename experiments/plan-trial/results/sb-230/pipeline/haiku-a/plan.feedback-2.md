# Feedback 2 on plan.md (issue #230), against worktree HEAD 7fe8949

1. Resolved: decisions 3 to 6 are now written, with the full CSP string, the static path table and the browser scope rule.
2. Resolved: §5 now names line 306 and §4.1 line 123 in the amendments, and the page exception is argued in decision 4; the argument that the bytes "reveal only what the documented protocol already reveals" is thin, but the owner can accept or reject it on the named amendment.
3. Resolved: the lan guard is in the UI, core is unchanged, and the fixture and core-test breakage is explained. My count of 36 was wrong (it included the `fn pair` definition); 35 calls is correct.
4. Resolved: ten literal constructions in `src/app.rs` and one in the adapter, with the 18 return-type matches explained; I confirmed the ten line numbers.
5. Resolved: the 429 and 503 writes at `src/adapters/remote.rs` 789 and 819 now return after the write, and the adapter test covers both.
6. Resolved: the design reference is now "a paired device reads a pane's screen" at `docs/design.md` 251–252, which matches the text.
7. Resolved as breaking: the QR is now the URL form, the JSON still parses on both sides, and the CLI binary rebuild is recorded in the contract list and the CHANGELOG line.
8. Resolved: steps 1 to 3 run now, steps 4 to 6 wait on named spike items, and the spike owner is named.
9. Resolved: the Origin rule now refuses cookie non-GET requests without a matching `Origin`.
10. Resolved: the Rust re-implementation is gone, and the shared `client/Fixtures/invite.json` is parsed by the Swift test.
11. New, from the rewrite of point 5's rate test: the Tests section says "the 61st page load in a minute is 429", but the `page` window in decision 4 counts requests, and one page load is three requests (`/`, `/app.css`, `/app.js`). The test as written would be wrong; the rewrite changed the earlier "61st request" wording. Make it "the 61st request to those paths in a minute".
