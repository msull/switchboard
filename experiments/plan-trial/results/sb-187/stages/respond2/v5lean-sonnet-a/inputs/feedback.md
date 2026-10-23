## Earlier points

1. `rerun_copy` assuming the newest failed review held the newest plan: closed. The backwards walk over `t.attempts` and the two-order unit test settle it.
2. `rerun_copy` and the `ticket_paths` rule being two signatures: closed. There is one function now. (See the cut under "The UI": the `before` parameter that fixed this is itself unneeded once the UI goes.)

## Cuts

**Where this departs from the issue**
- Cut the fourth bullet ("Matches the code"). It confirms the issue and changes nothing.
- Cut the fifth bullet. `rerun --note` is out of scope and is repeated under "For the owner".
- Shorten the third bullet to one sentence: the plan fixes any failed review attempt, and the test uses `runs.clear()`. The `remove_workflow` analysis is already in the facts file.

**The record**
- Cut the `RECORD_VERSION` and `SCHEMA_VERSION` line numbers and values. "No change, no bump" is enough, and "Contract additions" says it again.
- The walk is described here and again in "The actions and effects" item 1. Keep it once, in item 1. The "cannot drift" sentence goes with the UI cut.

**The signals**
- Delete the section. "None new" is a section with nothing to say. The existing `ticket.rs` tests stay green and the test suite enforces that.

**The actions and effects**
- Drop the `before` parameter. Once `ticket_paths` stops calling it, `start_workflow` and `rerun_carries` both pass `t.attempts.len()`, so `rerun_copy(t, stage, ctx, subject)` over the whole vector is enough. This also removes the "attempt never finds itself" test.
- Item 1: cut the explanation of why `latest_attempt` is not used. State the walk once.
- Item 2: cut the sentence "Attempt 2 failing in turn copies from `review/2`...". It follows from the code and the walk.
- Item 3: cut the last sentence on how `rerun_carries` finds the attempt. It is how the code already works.
- Delete the "Decisions resting on Unresolved lines" block. Two of its three bullets say "do the same thing as today" (copy as it is, pass `plan/1`'s session). The third (re-sending the objection) is an enhancement the plan says it will not build, and it is listed again under "For the owner". Keep only the one-line residual note (the cloned planner has not seen the objection) for design.md.

**The UI**
- Delete the whole section. The `LaneFile.revised_from` field, the `ticket_paths` change, the `print_lane_files` line and the `serve.rs` test are an enhancement built in front of the default. The `rerun_carries` question already names the copy at the moment the owner decides. Dropping them removes a contract change in `dispatch-control`, a third caller of `rerun_copy`, and two test sites.
- The "rounds not merged into `plan_rounds`" paragraph goes with it. It is a non-feature.
- With this cut, the commit message and the CHANGELOG line should drop "and `show` say which copy" and say "the rerun question says which copy".

**The tests**
- Cut the final bullet ("Existing tests must stay green"). The suite forces it.
- Cut the assertion that the `finalize` note and `plan.feedback-2.md` still exist. The facts file already settles that nothing deletes them, and this fix does not touch them.
- Cut the second `first_slice.rs` bullet's "failed first attempt with no revise reruns from its copy". It is the same code path as the main test without the marked text. Keep the Cancelled case, since it covers the skip rule.
- In the unit test, cut "the newest failure when two have failed" and the missing-file case if you want it smaller. The cross-stage ordering pair is the one that matters.
- Cut the sentence "I read the code only and did not run a restart...". It restates a facts-file caveat.
- Cut the `serve.rs` test bullet (it goes with the UI).

**Docs and release**
- Cut the README bullet. "No change" is a bullet with nothing to say.
- design.md: one paragraph is enough. The separate "line in the status section" can be a single sentence in that paragraph.

**For the owner**
- Delete the section. Its first two questions are copied from the facts file and the plan says none blocks it. Follow-ups for `rerun --note`, round listing and re-sending the objection can be one line in the PR text or dropped.

**Commit**
- Cut "One commit, README and design status updated, the app runs throughout". That is a project rule in CLAUDE.md.
- Cut "Contract additions" except for the `rerun_copy` signature, which is already given in "The actions and effects". With the UI removed, there is no contract change to list.
