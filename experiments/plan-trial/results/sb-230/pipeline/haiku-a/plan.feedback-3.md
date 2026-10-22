# Feedback 3 on plan.md (issue #230), against worktree HEAD 7fe8949

1. Resolved: decisions 3 to 6 remain written as before.
2. Resolved: the §5 justification now says what a stranger learns (the page source, which shows the protocol §4.3 already documents) and names the §5 amendment at line 306 for the owner to accept or reject.
3. Resolved: the lan guard is in the UI only, and the 35-call count is correct.
4. Resolved: the `RemoteReply` count (ten in `src/app.rs`, one in the adapter) stands.
5. Resolved: the 429 and 503 writes close the connection.
6. Resolved: the design reference is the screen-reading sentence at `docs/design.md` 251–252.
7. Resolved: the QR change is recorded as breaking, with the CLI rebuild in the contract list and the CHANGELOG.
8. Resolved: steps 1 to 3 run now and steps 4 to 6 wait on named spike items.
9. Resolved: a cookie non-GET request without a matching `Origin` is refused.
10. Resolved: the Swift test parses the shared `client/Fixtures/invite.json`.
11. Resolved: the rate test counts requests to the three static paths (61st request is 429), matching the window.
