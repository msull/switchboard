# Response to feedback on plan.md (issue #187), round 1

The copy at `review/1/repo/plan.md` is edited; the earlier plan at `plan/1/repo/plan.md` is untouched. I re-read the copy after the edits and each change below is in it.

1. **Copied `revisions` make the reviewer's round 1 read as the owner's objection.** Accepted. The plan no longer copies `revisions` onto the rerun. Changed: "The record" (no copy; the source search walks back past a failed attempt with no revisions of its own, so a later failure still finds the revised copy), and "The actions and effects" (`review_subject` takes the copy from the source search; `start_workflow` starts the new attempt with no `revisions`). The `plan_review`, `plan_rounds` and `review_round` case is a test in "The tests", the second new test in `first_slice.rs`, with `plan_review` as a unit test beside it.

2. **The design text is not in the Docs list.** Accepted. Changed: "The UI", the "Spec text, exact edits" bullet. It gives the sentence edit for `docs/design.md:1785-1786` and the clause added to the Plan tab fallback at `docs/design.md:2160-2163`. `docs/dispatch.md:1046` needs no edit, because `input_where` is unchanged. "The tests" Docs bullet points to these edits.

3. **Citation for the refusal.** Partly accepted. The citation was wrong. The refusal row is at `docs/dispatch.md:2975`, not `docs/design.md:2975`, and that row does say a refusal removes the revision. Changed: "Differs from the investigation notes" now cites the code, `scheduler.rs:11313` (the refused-`REVISE` arm, which pops the revision), and the `docs/dispatch.md:2975` row.

4. **The rerun label asserts a future.** Accepted. Changed: "The actions and effects" (`rerun_ask` wording is now conditional, with `asks_again` and `may_rerun` cited) and "The UI" (the `show` label and the Plan tab text now read "review failed; not reviewed again yet"). I did not use the pending-decision state, because the label describes the copy, not the rerun.

5. **Prior copies and lanes.** Accepted. Changed: "The actions and effects", `ticket_paths` bullet. Prior copies go in a new `PathsView.prior_plans` field, and `plan_files` keeps its meaning as the copy as it stands. Each prior copy's `lane` follows `lane_files_by` (`scheduler.rs:10908`): the attempt's `context` when its stage runs per lane, else none. "The UI" has the `show` loop for them, and "The tests" covers a per-lane case. "The commit message" contract list gains `PathsView.prior_plans` and drops `LaneFile.prior`.

6. **`review_source` ignores `forgotten`.** Accepted. Changed: "The actions and effects", the `review_subject` bullet. The `review_source` function is removed and its search is folded into `review_subject`, which now applies the same `forgotten` check as `input_where`. "The tests" has the new `scheduler.rs` test beside `lane_files_leave_out_a_forgotten_plan_and_keep_pipeline_order`.

One correction the feedback did not raise, found while editing point 1: the plan said `review_subject` takes its session "from the newest Complete attempt, as today". Today's code takes it from the same attempt that holds the artifact. A failed review's session is dead, so the plan now says the session stays with the newest `Complete` attempt that holds the subject. That is the plan's author, which is what the reviewer's planner clone needs. Changed: "The actions and effects", the `review_subject` bullet.

Not changed: the "Name check" and "Where this departs" points about `RECORD_VERSION`, and the no-schema-bump reasoning. You checked those and they stand.
