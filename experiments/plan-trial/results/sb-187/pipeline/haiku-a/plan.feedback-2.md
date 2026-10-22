1. Accepted: the rerun no longer copies `revisions`; the source search walks back past a revision-less failed attempt, and the round-1 collision is now a test.
2. Accepted: the three design/dispatch spec edits are listed exactly under "Spec text, exact edits".
3. Accepted: the refusal citation is now `scheduler.rs:11313` plus the `docs/dispatch.md:2975` row, which exists.
4. Accepted: the rerun label is conditional, and the shown-copy label reads "review failed; not reviewed again yet" in `show` and the Plan tab.
5. Accepted: prior copies go in `PathsView.prior_plans` with a lane from `lane_files_by`, and `plan_files` keeps its meaning.
6. Accepted: `review_subject` applies the `forgotten` check, with a `scheduler.rs` test beside `lane_files_leave_out_a_forgotten_plan_and_keep_pipeline_order`.

No further feedback.
