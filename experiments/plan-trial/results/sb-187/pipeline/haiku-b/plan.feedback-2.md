1. Satisfied: `carries_revision` excludes `Parking` and `Parked`, and `finish_parking` (`scheduler.rs:2620`, cancelling open attempts at `:2654`) is the path the plan cites.
2. Satisfied: `shown_where` and `review_copy` share the `carries_revision` predicate, so `shown`, `PathsView.plan` and `plan_files` agree.
3. Satisfied: the rerun clause names the copy and the feedback file and says the revision was sent, not applied.
4. Satisfied: `revised_copy_carries` names the attempt `review_copy` returns, takes the subject from `a.artifacts`, and leaves `rerun_carries` unchanged.
5. Satisfied: the Docs section now covers `design.md:2533`–`2537` and the agent guide's `show` labels at `:208`–`:216`.
6. Not satisfied: the commit message is still a title only, but CLAUDE.md requires a contract addition to be named in the commit message, and `src/ports/dispatch.rs:11` makes `LaneFile` part of that contract; the message must name `LaneFile.prior` and `LaneFile.failed`.
7. Satisfied: the `serve.rs:541`–`548` literal gains `prior: false` and `failed`.
8. Satisfied: `prior_lane_files` is a new walk beside `lane_files_by` that keeps superseded revised attempts, with lanes from `a.context` under `Stage::runs_per_lane`.
9. Satisfied: the new labels print only for a failed shown copy and for superseded revised copies; a finished, non-failed copy prints as today.
10. Satisfied: a unit test of `lane_file_note` is required in the `#[cfg(test)]` module at `dispatch/src/main.rs:1898`.
