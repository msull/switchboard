# Response to feedback 2

1. **Satisfied, no change.** The parked exclusion and the `finish_parking` citation (`scheduler.rs:2654`) stand as written in "Where this departs from the issue", decision 3, and "Actions and effects", item 1.

2. **Satisfied, no change.** `shown_where` and `review_copy` share `carries_revision` ("Actions and effects", items 1 to 3). `shown`, `PathsView.plan` and `plan_files` therefore agree.

3. **Satisfied, no change.** The rerun clause names the copy and the feedback file and says the revision was sent, not applied ("Actions and effects", item 5).

4. **Satisfied, no change.** `revised_copy_carries` names the attempt `review_copy` returns and takes the subject from `a.artifacts`. `rerun_carries` is unchanged ("Actions and effects", item 5).

5. **Satisfied, no change.** "Docs" covers `docs/design.md:2533`–`2537` and the agent guide's `show` labels at `docs/dispatch-agent-guide.md:208`–`216`.

6. **Changed.** The commit message in "Commit message" now names the contract additions on its one line: `Dispatch: a plan review rerun starts from the owner's revised copy; adds LaneFile.prior and LaneFile.failed`. I kept it to one line because the plan asked for a one-line message. "Contract additions" still lists both fields with their serde defaults. I re-read the file and confirmed the line at `plan.md:86`.

7. **Satisfied, no change.** "UI" gives the `serve.rs:541`–`548` literal `prior: false` and `failed`.

8. **Satisfied, no change.** "UI" defines `prior_lane_files` as a walk beside `lane_files_by`, with lanes from `a.context` under `Stage::runs_per_lane`.

9. **Satisfied, no change.** "UI" limits the new labels to a failed shown copy and superseded revised copies. A finished, non-failed copy prints as today.

10. **Satisfied, no change.** "Tests" requires a unit test of `lane_file_note` in the `#[cfg(test)]` module at `dispatch/src/main.rs:1898`.
