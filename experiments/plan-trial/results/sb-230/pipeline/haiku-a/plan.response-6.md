# Response to feedback-6 on plan.md (issue #230)

No edits were made to `review/1/repo/plan.md` this round, because the feedback marks all eleven points resolved. The file is as it stood after the round-2 edits, and I did not change it.

1. **Decisions 3 to 6.** No change. They remain under "The decisions".
2. **§5 justification and the line 306 amendment.** No change. They are in "Decision 4: static paths…", unchanged since round 2.
3. **Lan guard.** No change. It is UI-only, under "Actions and effects".
4. **`RemoteReply` constructions.** No change. They are listed under "Actions and effects".
5. **429 and 503 writes.** No change. They close the connection, under "Actions and effects".
6. **Design reference.** No change. It is `docs/design.md` 251–252, under "Docs and status".
7. **QR change.** No change. It is breaking and recorded in "Decision 3" and "Contract additions".
8. **Gate order.** No change. "Order" runs steps 1 to 3 now and gates the rest on named spike items.
9. **Origin rule.** No change. It is under "Decision 2", and refuses a cookie non-`GET` request without a matching `Origin`.
10. **Shared fixture.** No change. The Swift test parses `client/Fixtures/invite.json`.
11. **Rate test.** No change. The test at line 169 counts requests to the three static paths.

The earlier plan at `plan/1/repo/plan.md` is untouched. Nothing was built or run.
