# Response to feedback on plan.md (issue #187), round 6

No edit this round. The feedback accepts all six round-1 points, marks each as already in the plan, and says "No further feedback", so the copy at `review/1/repo/plan.md` is unchanged. The earlier plan at `plan/1/repo/plan.md` is also untouched. I checked the copy for each point before writing this.

1. **Revisions not copied.** Already in the plan: "The record" has the walk-back, and "The tests" has the round-1 test (`a_rerun_reviewing_its_own_round_one_reads_it_as_the_reviewers`).

2. **Spec edits.** Already in the plan: "The UI", the "Spec text, exact edits" bullet. `docs/dispatch.md:1046` needs none.

3. **Refusal citation.** Already in the plan: "Where this departs from the issue" cites `scheduler.rs:11313` and the `docs/dispatch.md:2975` row.

4. **Conditional `rerun_ask` sentence.** Already in the plan: "The actions and effects", the `rerun_ask` bullet. The labels are in "The UI".

5. **Prior copies and lanes.** Already in the plan: the `ticket_paths` bullet in "The actions and effects" uses `PathsView.prior_plans` and the `lane_files_by` lane rule. The contract list has the field.

6. **`forgotten` in `review_subject`.** Already in the plan: the `review_subject` bullet in "The actions and effects", and the `scheduler.rs` test in "The tests" beside `lane_files_leave_out_a_forgotten_plan_and_keep_pipeline_order`.
