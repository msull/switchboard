# Response to round 2 feedback

**Earlier points 1 and 2** were closed. They stay closed; the single `rerun_copy` is kept, now without `before`.

**Where this departs from the issue**
- Fourth bullet cut. Fifth bullet cut. Third bullet is now one sentence (any failed review attempt; test uses `runs.clear()`); the `remove_workflow` analysis stays only as a citation. Section "Where this departs from the issue".
- I kept the second bullet (the `show` claim) and narrowed it to say the plan fixes the rerun question only. With the UI gone, the plan has to say what it leaves alone.

**The record**
- Line numbers and versions cut: now "No change, no bump." The walk is stated once, in item 1 of "The actions and effects".

**The signals**
- Section deleted.

**The actions and effects**
- `before` dropped; signature is `rerun_copy(t, stage, ctx, subject)` over the whole vector. The "attempt never finds itself" test went with it.
- Item 1: the `latest_attempt` explanation cut; walk stated once. Item 2: the "Attempt 2 failing in turn" sentence cut. Item 3: the last sentence cut.
- The "Decisions resting on Unresolved lines" block is gone. One sentence remains saying the defaults are today's behaviour and re-sending the objection is not built. The residual note moved into the design.md paragraph under "Docs and release".

**The UI**
- Section deleted, including `LaneFile.revised_from`, the `ticket_paths` and `print_lane_files` changes, the `serve.rs` test and the `plan_rounds` paragraph. The rerun question names the copy. Commit message is unchanged because it already said "and says so"; the CHANGELOG line now says "the rerun question says which copy".

**The tests**
- Cut: the "stay green" bullet, the `finalize` note / `plan.feedback-2.md` assertion, the failed-first-attempt-no-revise bullet (Cancelled case kept), the missing-file and two-failures unit cases, the "read the code only" sentence, and the `serve.rs` bullet. The cross-stage ordering pair stays, plus a Cancelled → `None` case, which is one line and covers the skip rule at unit level.

**Docs and release**
- README bullet cut. design.md is one paragraph with the status sentence inside it.

**For the owner**
- Section deleted. The `rerun --note` and round-listing follow-ups are dropped from the plan; they can go in the PR text.

**Commit**
- The "one commit…" line is cut. Contract additions reduced to the `rerun_copy` signature.
