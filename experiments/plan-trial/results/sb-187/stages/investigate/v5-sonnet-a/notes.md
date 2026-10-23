# Notes: issue #187, review rerun starts from the plan stage's original output

Read: CLAUDE.md, docs/lessons.md (generic lessons, nothing specific here), docs/design.md in the plan-review parts (about lines 1000-1060, 1775-1800, 2155-2180). Line numbers are from this worktree. The code is the `dispatch` crate (`dispatch/src/`).

"Review stage" here is a *workflow* stage: a plan review run by Switchboard (`AttemptKind::Workflow`). It is not the code review in `dispatch/src/review.rs`. The code review has its own carry-over (`carry_source`, `review.rs:3608`, `Attempt.carried_from`), which the issue's expectation resembles. Plan review has none.

## The path the issue describes

1. `workflow_stage` (`dispatch/src/scheduler.rs:7379`) picks the stage's next move per context. For a failed latest attempt with an acted `rerun` answer (`may_rerun`, `:10665`) it calls `start_workflow` with `next_n`.
2. `start_workflow` (`:7436`) takes the plan from `review_subject(t, &subject)` (call at `:7457`). It copies that file to `<ticket dir>/<stage>/<n>/<ctx>/<subject>.md` (`attempt_dir` `:6303`, copy about `:7471`). It sends `Body::WorkflowStart { source, plan: copy, ... }` (`:7510`).
3. `review_subject` (`:10534`) walks `t.attempts` newest first, keeps only `AttemptState::Complete`, and takes the first whose `artifacts` has the subject name. It returns that path and that attempt's `session`.
   - After a failed review/1 the newest Complete attempt with `plan` is the plan stage's. That is `plan/1/joined/plan.md`, which is the symptom.
   - A failed review attempt's copy is never considered, whatever the planner wrote into it.
   - `source` is the plan stage's session, the original planner. A new run clones that planner (design.md ~1050), so the new planner never saw the owner's note or the revision either.
4. Only a `Finalized`/`HandedOff` run sets the attempt `Complete` (`poll_workflow`, about `:7843`). `revise` edits the copy in place, so the revision is on a non-Complete attempt until finalize.
   - `Ticket::input_where` (`dispatch/src/ticket.rs:1437-1462`) is Complete-only on purpose ("an agent is handed `input_where`'s complete files").
   - `Ticket::shown_where` (`:1467-1488`) also accepts an *open* workflow attempt but not a failed one, so a failed review/1 drops out of `show` too.
5. `revise` (`scheduler.rs:7565`):
   - the note is stored in the decision (`DecisionState::Answered { note, by, acted }`);
   - it writes `<subject>.feedback-<round>.md` beside the copy, headed `OWNER_ROUND_HEADING` (`dispatch/src/lib.rs:96`);
   - it pushes a `Revision { round, by, at_ms }` on `Attempt.revisions` (`ticket.rs:350`);
   - it sends `Body::WorkflowObject { run, round, text }` and the planner clone revises the copy and writes `<subject>.response-<round>.md`.

   So the note survives on disk in the decision, the feedback file and `revisions`. The revised plan exists only in the copy file. Attempt 2's fresh copy never reads any of them.
6. `no such run` comes from `poll_workflow` (`:7761`, `NO_SUCH_RUN` `:8445`) and calls `fail_attempt` (`:7148`, `fail_attempt_with` `:7195`). That sets `Failed`, asks `rerun`/`park` or parks past `max_reruns`, and touches no files. The `rerun` answer goes through `act_on_answers` (`:3524`) and `retire_replaced` (`:3578`), which kills processes and stores a rework note, which a workflow stage does not use.
7. `rerun_ask` (`:9335`) builds the question text; `rerun_carries` (`:9382`) adds a sentence only for code review attempts and returns "" for workflow attempts. That is where "which copy the rerun will review" would go.
8. No test covers revise, then a failed run, then rerun. Neighbours:
   - `dispatch/tests/first_slice.rs:3257` (`a_failed_review_run_fails_the_attempt_and_a_rerun_starts_a_new_run`), with helpers `at_review_run` `:3231`, `review_run_fails` `:3249`;
   - revise tests at `:3417-3600` (`revise` helper `:3417`);
   - unit tests at `scheduler.rs:~12449` and `~12850-12868` (which plan a reader gets while a review is open or complete);
   - `ticket.rs:~1800-1870` (`input` vs `shown`);
   - `serve.rs:~1380-1710` (paths).

## `show`'s `files:` block

- `fn show` in `dispatch/src/main.rs` (`files:` at `:981`) calls `print_lane_files("plan", &p.plan_files)` (`:896`). It prints `  plan: <path>` and, only when `LaneFile.reviewing`, `(reviewed copy, round N, review open)`.
- `ticket_paths` (`dispatch/src/serve.rs:~538`) builds `plan_files` from `shown_lane_files` (`scheduler.rs:~10900`, `lane_files_by`): one file per lane (or one in all) from `shown_where`. A prior attempt's `plan.md` is never listed, and nothing says "revision".
- `plan_rounds` (`serve.rs:~504`) is built from the *latest* workflow attempt with artifacts, so after a rerun attempt 1's rounds and its `revisions` are not listed.
- `LaneFile`/`PathsView` (`plan_files`, `plan_stage`, `plan_reviewed`, `plan_reviewing`, `plan_round`) are wire types in `dispatch-control`, read by the app (`src/ui/ticket.rs:~427`) and the Swift client (`client/Sources/SwitchboardProtocol/Tickets.swift`). CLAUDE.md says extend the contract additively.
- The attempts list ("review/1 joined failed") is `print_attempts`; it says nothing about the copy.

## On-disk record and rehydration (before the pane)

- The record: the ticket in Dispatch's data dir (`Ticket.attempts[]` with `artifacts`, `revisions`, `run`, `session`, `state`; `decisions[]` with the answered note), saved atomically by `save_ticket`. The files sit under `<data>/tickets/<id>/<stage>/<n>/<ctx>/`.
- Rehydration of a rerun is a pure function of those records: `workflow_stage` re-reads the ticket every pass and `review_subject` re-derives the source, so a restart mid-rerun picks the same file. A fix that only changes which attempt's copy `review_subject` picks needs no new field: a failed attempt's `artifacts[subject]` with non-empty `revisions` already says "holds an owner revision".
- If the fix stores a "carried from review/1" marker on the new attempt, that is a serialized `Attempt` field: `#[serde(default)]`, a dispatch `RECORD_VERSION` bump and a migration step (CLAUDE.md "Extending the contract").
- App side, the run is a `WorkflowRun` record (plan path, planner and reviewer sessions, rounds, snapshots; design.md ~1015-1030). A new `workflow.start` makes a new run; the old planner clone's context is not reused.
- The pane: `show` text, the app's Plan tab ("Reviewed copy, round N: review open", design.md ~2161), the rerun question text. All derive from the above.

## What I did not check

- Whether the Switchboard app (`src/`) deletes or alters a failed run's copy or round files when it retires the run.
- Whether a failed attempt's copy can be mid-edit.
- Other readers of `review_subject` or `input("plan")` (reports, resume, restart, `recover.rs`, `lane_plans`); I only traced `start_workflow`.
- How the app UI and the Swift client treat more than one `plan.md`.
- Nothing was built or run; no tests executed.

## Open questions

- [tree] Which other call sites read the plan through `Ticket::input`/`input_where`/`review_subject` (reports, resume, restart, `lane_plans`, `recover.rs`) and would be affected if a failed review attempt's copy counted as the newest plan?
- [tree] Does the Switchboard app delete, move or rewrite a failed run's plan copy, feedback and response files when the run is retired or on `workflow.start` for the next attempt, so that review/1's `plan.md` is intact when attempt 2 starts?
- [tree] Can a failed attempt's copy be half-written (planner clone mid-round; an owner round the app refused, whose feedback file `revise` removes at `scheduler.rs:~7650`), and does any record on the attempt (`revisions`, round and response files, the run's last round) tell a finished revision from a partial one?
- [tree] If the plan stage was itself rerun after review/1 failed, `plan/2` would be newer than review/1; does any record order the two (attempt `started_ms`/`ended_ms`) so "newest plan the stage holds" is well defined?
- [tree] What does the app do with `source` on `workflow.start` (clones the planner from its transcript), and what would the new planner need so the owner's revise note is not lost to it as well as the file?
- [tree] How do `show --json`, the app's ticket page and the Swift client (`TicketFileView`) consume `LaneFile`/`PathsView`, so a "prior revision" file or label can be added without breaking them?
- [run] After revise, then `no such run`, then `rerun`, does the fake Switchboard in `dispatch/tests/first_slice.rs` reproduce review/2 starting from `plan/1` with review/1's `artifacts["plan"]` still pointing at the revised copy? Not run.
- [owner] Was review/1's `plan.md` still the revised text on the owner's disk after the failure, and is the `revise` note held anywhere else? Needs the owner's ticket data directory.
