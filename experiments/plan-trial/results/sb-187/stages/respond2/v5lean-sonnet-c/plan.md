# Plan: issue #187, a review rerun starts from the newest plan the stage holds

## Where this departs from the issue

- The issue says "kill the review run … a runner restart mid-run does it". The tree does not bear that out as the cause. The plan does not depend on why the run vanished (facts, Unresolved: "Why the app answered `no such run`"); it reads the failed attempt's record, which `fail_attempt_with` leaves intact (`scheduler.rs:7195-7230`, facts Settled).
- "The `revise` note … should survive a runner failure the same way an answered decision does." It already does; what is lost is the pointer to the revised copy.
- The issue says `show` "names only the attempt-2 copy". Before the rerun answer it is worse: `shown_where` (`ticket.rs:1476-1491`) skips the failed review, so `show` names `plan/1/joined/plan.md` and the revised copy is not listed.

## The actions and effects

No new `AppAction` or `Effect`. Three changes in `dispatch/`:

1. **`review_subject` (`scheduler.rs:10534-10548`) picks the copy and the session separately.**
   - The path is the file of the newest attempt that wrote `subject` and is either `Complete`, or a `Workflow` attempt in `Failed` whose file exists (`Path::is_file`; if it does not, skip that attempt, so a deleted copy degrades to today's behaviour instead of making `fs::copy` fail).
   - `Failed` only; a `Cancelled` copy may be one Dispatch discarded (`ticket.rs:228-232`).
   - A failed review is accepted whether or not it has `revisions`. Risk: a review edits its copy in place until `finalize` (`ticket.rs:1446-1450`), so one that failed mid-round may leave a half-edited copy. The alternative, counting only revised attempts, drops the owner's `revise` on every chain (review/2 fails with no `revisions` of its own, and review/3 falls back to `plan/1`).
   - The session stays what it is today: the newest `Complete` attempt that wrote `subject` and has a session. It must not come from the failed review, whose `session` is the planner clone's face (`scheduler.rs:7786-7788`).
   - Return `(PathBuf, String)` as today; the only change is how the path is chosen. The one caller is `start_workflow` (`scheduler.rs:7465`), which keeps copying into the new attempt's directory.
   - `Ticket::input_where` (`ticket.rs:1452-1463`) is unchanged: it feeds downstream stages, which read only a completed attempt's output (test at `ticket.rs:1797-1824`).
2. **The rerun question names the copy.** `rerun_carries` (`scheduler.rs:9382-9390`) returns `&'static str` and gives a `Workflow` attempt nothing. Change it to return `String` and, for a `Workflow` attempt, append ` The next attempt reviews <path>`, where the path is the one item 1 picks. Both callers (`scheduler.rs:7266`, `9373`) change type only.
3. **`Ticket::shown_where` (`ticket.rs:1476-1491`) also accepts a `Failed` `Workflow` attempt**, so `show`, the ticket page and `shown_lane_files` name the copy the rerun reviews. Put the predicate on `Attempt` once (`holds_review_copy`) and use it in both places. `lane_files_by` already narrows to `Complete` for the input case (`scheduler.rs:10917-10921`).

## The tests

- `dispatch/src/scheduler.rs` unit tests beside `review_subject`: (a) plan/1 `Complete` plus review/1 `Failed` with a copy on disk → path is review/1's, session is plan/1's; (b) review/1 failed with the file missing → falls back to plan/1's path; (c) a `Cancelled` review with a copy on disk is never chosen; (d) review/1 `Complete` newer than plan/1 → unchanged from today.
- `dispatch/tests/first_slice.rs`, beside `a_run_switchboard_no_longer_has_fails_the_attempt` (line 3809): (1) drive plan, review, answer `finalize` with `revise --note`, clear `env.sb().runs` so the attempt fails, answer `rerun`. Assert the `WorkflowStart` for attempt 2 carries a `plan` under `review-plan/2/` whose bytes equal the revised `review-plan/1` file, that `source` is the plan attempt's session, and that the `rerun` question text names the review/1 copy. (2) The chained failure: a second failure, and attempt 3 starts from attempt 2's copy.
- `dispatch/src/ticket.rs` tests: `shown("plan")` returns a failed workflow attempt's copy; `input("plan")` still returns the completed attempt's (extend the test near line 1840).

## Docs, one commit

- `docs/dispatch.md`: the rerun rules near 318-350 get one sentence on a review rerun's input; the `revise --note` row at 2975 gets "a failed review attempt's rerun starts from its copy".
- `docs/design.md`: the Dispatch known-gaps list (line 3098) does not list this gap, so add one status sentence to the rerun paragraph at line 1702: a review rerun starts from the newest reviewed copy.
- `CHANGELOG.md`: under Unreleased, "Dispatch: a plan review rerun after a failed attempt starts from the newest reviewed copy, so an owner's `revise` survives, and the rerun question and `show` name the copy."

## Commit message

Dispatch: a plan review rerun starts from the newest reviewed copy, not the plan stage's output, and the rerun question and `show` name it

## Contract additions

- `Attempt::holds_review_copy` (dispatch crate, internal helper).
- No record, signal, `AppAction` or `Effect` change; no schema bump.
