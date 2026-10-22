# Response to feedback-3 on plan.md (issue #230)

No edits were made to `review/1/repo/plan.md` this round, because the feedback marks all eleven points resolved. The file is as it stood after the round-2 edits.

1. **Decisions 3 to 6.** No change. They remain written in "The decisions".
2. **§5 justification.** No change. "Decision 4" in "The decisions" already states what a stranger learns and names the §5 amendment at line 306.
3. **Lan guard.** No change. It is in the UI only, under "Actions and effects".
4. **`RemoteReply` count.** No change. Ten literals in `src/app.rs` and one in the adapter are in "Actions and effects".
5. **Direct 429 and 503 writes.** No change. They close the connection, under "Actions and effects".
6. **Design reference.** No change. It is `docs/design.md` 251–252, in "Docs and status".
7. **QR as breaking.** No change. It is recorded in "Decision 3" and "Contract additions".
8. **Gate order.** No change. "Order" runs steps 1 to 3 now and waits on named spike items for the rest.
9. **Origin rule.** No change. Decision 2's "Origin rule" refuses a non-`GET` cookie request without a matching `Origin`.
10. **Shared fixture.** No change. `client/Fixtures/invite.json` is parsed by the Swift test.
11. **Rate test.** No change. The "Tests" section counts requests to the three static paths, so the 61st request is 429.

The earlier plan at `plan/1/repo/plan.md` is untouched. Nothing was built or run.
