# Response to feedback-2 on plan.md (issue #230)

The plan changed at `review/1/repo/plan.md`. I confirmed both edits with a search of the file after editing: the lines are at 92 and 169.

1. **Decisions 3 to 6.** No change; the feedback marks this resolved.
2. **§5 justification. Changed.** "The Decision 4 justification" in "Decisions" (section "Decision 4: static paths…", the paragraph beginning "Why the static paths may sit outside `/api/v1/`") now says what an unpaired stranger learns: the page's source, which shows the protocol the page speaks. It cites §4.3 as the place the protocol is already documented, so no secret is exposed. It says plainly that this is a judgement, and that the owner accepts or rejects it on the §5 amendment at line 306.
3. **Lan guard.** No change; the feedback marks this resolved.
4. **`RemoteReply` count.** No change; the feedback marks this resolved.
5. **Direct writes.** No change; the feedback marks this resolved.
6. **Design reference.** No change; the feedback marks this resolved.
7. **QR as breaking.** No change; the feedback marks this resolved.
8. **Gate order.** No change; the feedback marks this resolved.
9. **Origin rule.** No change; the feedback marks this resolved.
10. **Shared fixture.** No change; the feedback marks this resolved.
11. **Rate test wording. Changed.** The rate test in "Tests" ("`tests/remote.rs`", the bullet "A rate test") now reads: the 61st request to `/`, `/app.css` or `/app.js` in a minute is 429 and the connection closes. It adds that the window counts requests, not page loads, and that one page load is three requests. The "Rate" paragraph in decision 4 already said the window counts the three 200 paths, so it needed no change.
