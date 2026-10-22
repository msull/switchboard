# Response to feedback on plan.md (issue #187), round 3

No edit this round. The feedback accepts all six round-1 points, marks each as already in the plan, and says "No further feedback", so the copy at `review/1/repo/plan.md` is unchanged. The earlier plan at `plan/1/repo/plan.md` is also untouched. I checked the copy for each point before writing this.

1. **Revisions not copied; round-1 collision test.** Already in the plan: "The record" (no copy, and the walk-back), "The actions and effects" (`review_subject` source search), and the second new test in "The tests" (`a_rerun_reviewing_its_own_round_one_reads_it_as_the_reviewers`).

2. **Spec edits.** Already in the plan: "The UI", the "Spec text, exact edits" bullet. `docs/dispatch.md:1046` is marked as needing no edit.

3. **Refusal citation.** Already in the plan: "Where this departs from the issue" cites `scheduler.rs:11313` and the `docs/dispatch.md:2975` row.

4. **Conditional rerun label.** Already in the plan: the `rerun_ask` bullet in "The actions and effects", and the labels in "The UI".

5. **Prior copies and lanes.** Already in the plan: the `ticket_paths` bullet in "The actions and effects", with `PathsView.prior_plans` and the lane rule from `lane_files_by`. The contract list in the commit section has the field.

6. **`forgotten` in `review_subject`.** Already in the plan: the `review_subject` bullet in "The actions and effects", and the `scheduler.rs` test in "The tests" beside `lane_files_leave_out_a_forgotten_plan_and_keep_pipeline_order`.
