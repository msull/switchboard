# Facts: issue #187

The issue and the investigation claim nothing about a browser, platform, crate, protocol or tool outside this tree. Every open question is about the tree, so no web search was run and no spike was written (git status is empty). Line numbers are from this worktree.

## Settled

Plan readers (open question 1):
- `review_subject` has one caller, `start_workflow`. It reads `t.attempts` newest first by `Vec` position and keeps only `AttemptState::Complete`. `dispatch/src/scheduler.rs:7465`, `:10534-10547`.
- Every other reader of the plan goes through `Ticket::input_where`, which is also Complete-only and skips `forgotten`. These are `lane_input` (`scheduler.rs:10843`), then `lane_plan`, `lane_plans` and `plan_clause`, and the prompt variables `vars_for` (`:10776`). `dispatch/src/ticket.rs:1452-1462`.
- Report plan lists use `lane_files` (`dispatch/src/report.rs:300`). `serve.rs:587` reads only `notes`.
- Changing `review_subject` alone leaves all of these on Complete attempts, so downstream stages and prompts never see an unfinalized or failed copy.
- Making `input_where` count a failed review attempt's copy would change what implement and code review are handed. That is wider than the issue.
- `Ticket::shown_where` also takes an open workflow attempt, never a failed one. `ticket.rs:1476-1488`. It is display only (`scheduler.rs:10917-10919`).

The app does not touch the copy (question 2):
- The app has no code that deletes or moves the plan copy or its feedback and response files. `RemoveWorkflow` only drops the run record and probes: `src/core/workflow.rs:1009-1031`.
- `start_workflow` in the app only reads `plan` and clones the source's transcript. It writes nothing to that path: `src/core/workflow.rs:289-345`.
- The only `remove_file` calls in `src/` are `app.rs:1497` (`remove_quietly`, unrelated), the hook binary's own files, the updater, and a PNG temp in `ui/document.rs`. `grep remove_file|remove_dir_all|rename src` found no others.
- Dispatch's `fail_attempt` also touches no files (investigation item 6).
- So review/1's `plan.md` is intact when attempt 2 starts. Dispatch deletes one file only: the owner's feedback file for a round the app refused (`scheduler.rs:7640-7647`).

Partial or finished revision (question 3):
- `revise` writes the feedback file, then pushes the `Revision` onto `Attempt.revisions`, then sends `WorkflowObject` (`scheduler.rs:7607-7628`).
- If the app refuses the round, the feedback file is removed but the `Revision` stays (`:7640-7647`). A non-empty `revisions` therefore does not prove the planner revised anything.
- `revise` runs only while the run is `Converged` or `AtCap` (`:7600-7606`).
- The investigation's suggestion that "non-empty `revisions` says the copy holds an owner revision" does not hold. A finished revision is evidenced by the response file `<subject>.response-<round>.md` beside the copy, which the planner writes.

Attempt order (question 4):
- Attempts are appended to `t.attempts` (`scheduler.rs:7479`), and every reader above orders by `Vec` position. `started_ms` and `ended_ms` exist (`ticket.rs:375-376`) but no reader uses them to rank attempts.
- Any "newest plan" rule built on position is well defined, with ties impossible.

`source` and the new planner (question 5):
- The app requires `source` to be a Claude Code session with a transcript and clones it as the planner (`src/core/workflow.rs:306-322`).
- `review_subject` returns the plan stage's session as `source`. The new planner is therefore a clone of the original planner, which has seen neither the `revise` note nor the revision.
- The reviser's context lives only in review/1's run planner clone (an app record the failed run leaves behind).
- The owner's note survives on disk in the decision's `DecisionState::Answered { note }` and in `<subject>.feedback-<round>.md`. The `OWNER_ROUND_HEADING` text is in the feedback file.

Wire types (question 6):
- `LaneFile` is `#[serde(default)]`, so a new field is additive on the wire (`dispatch-control/src/lib.rs:584-599`).
- The app reads only `plan_reviewing`, `plan_reviewed`, `plan_round` and `plan_stage` (`src/ui/ticket.rs:427-431`).
- The CLI reads `plan_files` through `print_lane_files` (`dispatch/src/main.rs:896`, `:982`).
- `grep planFiles|plan_files|reviewing client/` found nothing in the Swift client. The investigation's mention of `TicketFileView` could not be matched to a use of these fields.
- A new `LaneFile` field or a new `PathsView` list should be additive and break none of the three readers.

The `[run]` question:
- The fake already reproduces the path up to the rerun: `a_failed_review_run_fails_the_attempt_and_a_rerun_starts_a_new_run` (`dispatch/tests/first_slice.rs:3257`), with `review_run_fails` (`:3249`).
- Whether attempt 2 starts from `plan/1` follows from the read above (Complete filter at `scheduler.rs:10537`; no step marks a failed attempt Complete).
- The plan stage is Complete in that test, and the test's review attempt has no revise before the failure.
- A new test only needs a `revise` step (helper at `first_slice.rs:3417`) between `at_review_run` and `review_run_fails`. This is a test the plan writes, not a question for a spike.

## Unresolved

U1. Can the plan stage itself be rerun after review/1 failed, so `plan/2` is appended after review/1?
- Tried: read `workflow_stage`, `review_subject` and `start_workflow` only. I did not trace the stage-ordering code.
- Branch where it can: a "newest plan" rule by `Vec` position then prefers `plan/2` over review/1's revision. That is correct, since a fresh plan supersedes the old revision. The plan must test this order.
- Branch where it cannot: the rule is simply "newest of failed review copy and Complete plan".
- Build to: rank by `Vec` position in both cases. Add a test with a later `plan` attempt, and do not wait on this line.

U2. Is a failed attempt's copy ever mid-edit when the rerun starts? The planner clone may still be running when `poll_workflow` sees `no such run` or a failed state.
- Tried: nothing runs here (needs a live app and planner), and the code does not stop the planner on `fail_attempt`.
- Build to: if the planner is quiet, take the failed copy. If not, the plan cannot tell. So the rule that copies a failed attempt's `plan.md` must require a finished revision. The evidence available is the response file for the latest `Revision.round` on that attempt. Otherwise fall back to the Complete plan, and the rerun question and `show` must say which file is used.

## For the owner

O1. Was `review/1/joined/plan.md` still the revised text on disk after the failure, and is the `revise` note held anywhere else?
- Needs the ticket's data directory.
- Both branches work: the plan builds to "the failed attempt's copy is intact" (the tree shows nothing deletes it, above) and to "the note lives in the decision and in `plan.feedback-<round>.md`".
- If the owner reports the copy was empty or reverted, the plan adds nothing different. It falls back to the Complete plan and names it in the rerun question and `show`.
