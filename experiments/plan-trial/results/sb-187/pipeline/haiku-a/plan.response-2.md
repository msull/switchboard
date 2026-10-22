# Response to feedback on plan.md (issue #187), round 2

No edit this round. The feedback accepts all six round-1 points and says "No further feedback", so the copy at `review/1/repo/plan.md` is unchanged. The earlier plan at `plan/1/repo/plan.md` is also untouched. I checked the copy's text for each point below before writing this.

1. **Revisions no longer copied; round-1 collision is a test.** Already in the plan. "The record" explains why the rerun does not copy `revisions`, and the source search in "The actions and effects" walks back past a revision-less failed attempt. The collision test is the second new test in "The tests" (`first_slice.rs`), with `plan_review` as a unit test beside it.

2. **Spec edits listed exactly.** Already in the plan. "The UI" has the "Spec text, exact edits" bullet, covering `docs/design.md:1785-1786` and `docs/design.md:2160-2163`, and says `docs/dispatch.md:1046` needs no edit.

3. **Refusal citation.** Already in the plan. "Where this departs from the issue" cites `scheduler.rs:11313` and the `docs/dispatch.md:2975` row. I confirmed that row exists in the worktree.

4. **Conditional rerun label.** Already in the plan. "The actions and effects" has the conditional `rerun_ask` wording, and "The UI" has "review failed; not reviewed again yet" for both `show` and the Plan tab.

5. **Prior copies and lanes.** Already in the plan. "The actions and effects", `ticket_paths` bullet, adds `PathsView.prior_plans` and takes each lane from `lane_files_by`. `plan_files` keeps its meaning. The "Contract additions" list has the new field.

6. **`forgotten` in `review_subject`.** Already in the plan. The `review_subject` bullet in "The actions and effects" applies the `forgotten` check, and "The tests" has the `scheduler.rs` test beside `lane_files_leave_out_a_forgotten_plan_and_keep_pipeline_order` (`scheduler.rs:12615`).
