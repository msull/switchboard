# Investigation notes: issue #187 (review rerun discards the revised plan)

Worktree: `/Users/sully/code_repos/plan-trial/sb-187-stage-investigate-v5-sonnet-b`. Line numbers are from this tree.
Read: CLAUDE.md, docs/lessons.md (generic Prompt Box lessons, nothing on Dispatch), the design.md "Plan review workflow" section (line 1000) and the Dispatch sections around lines 1775-1800, and docs/dispatch.md (decisions 6 and 16 at ~3170-3215, failure rules at ~2490-2510, the test table at ~2975 and ~3078).
I did not read all 3678 lines of design.md.
The issue is in the `dispatch/` crate, not the egui app. The egui app only hosts the workflow run (`src/core/control.rs:1384`, `ControlAction::StartWorkflow` at `src/core/control.rs:125`).

## On-disk record, then rehydration (before any pane or CLI output)

The ticket record is `<data>/.../tickets/<id>` (`Ticket` in `dispatch/src/ticket.rs`). Saved through `save_ticket`.
- A workflow attempt (`AttemptKind::Workflow`) is an `Attempt` in `t.attempts`. Its `artifacts` map holds one entry, `subject -> <attempt_dir>/<subject>.md`, e.g. `review/1/joined/plan.md`.
  - The directory comes from `attempt_dir` (`scheduler.rs:6303`): `<ticket_dir>/<stage>/<n>/<ctx>`.
  - `start_workflow` creates that copy with `std::fs::copy` of the source plan (`scheduler.rs:7436-7500`, copy at ~7469).
- The review run edits that copy in place. The planner clone applies accepted points to it.
- Round files sit beside the copy: `<stem>.feedback-<n>.md` and `<stem>.response-<n>.md` (`report.rs:156-210`).
- `revise` (`scheduler.rs:7565`) does these things:
  - It writes the owner's note as the next round's feedback file, headed `OWNER_ROUND_HEADING`.
  - It pushes `Revision{round,by,at_ms}` onto `attempt.revisions` (`ticket.rs:510`).
  - It sends `Body::WorkflowObject` to the app.
- The note is also kept on the decision. `DecisionState::Answered{note,by,..}` stays in `t.decisions`, and the `Answered` event is in the event log. This is how "an answered decision survives" today.
- The only copy of the revised plan is the file in `review/1/joined/`. The ticket record holds just its path.

When the run fails:
- `poll_workflow` (`scheduler.rs:7740-7765`) gets `NO_SUCH_RUN` (`scheduler.rs:8445`) and calls `fail_attempt(.., "review run gone: no such run")`.
- That reaches `fail_attempt_with` (`scheduler.rs:7195`). The attempt becomes `AttemptState::Failed`, it is saved, and a `rerun` decision is asked via `ensure_decision` / `ask_rerun` (`scheduler.rs:7308`).
- `rerun_ask` (`scheduler.rs:9335`) builds the question text. For a Workflow attempt `rerun_carries` (`scheduler.rs:9382`) returns `""`, so the question says nothing about which plan the rerun reviews.
- The existing test `a_run_switchboard_no_longer_has_fails_the_attempt` is at `dispatch/tests/first_slice.rs:3809`.

Rehydration after the `rerun` answer:
- `rerun` is acted on at `scheduler.rs:3524` (`("rerun","rerun")` calls `rerun_note` and `retire_replaced`). The workflow lane is then relaunched by `may_rerun` at `scheduler.rs:7402` and `7421`. Both call `start_workflow` with `next_n`.
- `rerun_note` (`scheduler.rs:8988`) returns `None` unless the stage kind is `Agent` or `Review`. The code-review kind `StageKind::Review` is what the code calls "Review". A `Workflow` stage returns `None`, so a note on a workflow `rerun` goes nowhere.
- **Root cause:** `review_subject` (`scheduler.rs:10534`) picks the source plan.
  - It walks `t.attempts` newest first and keeps only `state == Complete` attempts that have `artifacts[subject]`.
  - The failed `review/1` is not Complete, so it is skipped. The result is `plan/1/joined/plan.md` plus plan/1's session as `source`.
  - Repro line `review/1/joined/plan.md -> review/2/joined/plan.md` is then a copy of the original plan.
  - `review/1/joined/plan.md` is left on disk, still holding the revision, and nothing points to it except the old failed attempt's `artifacts`.
- The same exclusion is asserted on purpose in the `shown` test, `ticket.rs:1850-1866`: "A parked review's copy is not the plan going forward" for Failed and Cancelled. Any fix has to be reconciled with that test and with `input_where` (`ticket.rs:1452`).
  - `input`/`input_where` (`ticket.rs:1437-1462`) feed later agent stages and also require Complete.
  - The comment on `shown_where` says "an agent is handed `input_where`'s complete files".
- `source_session` is the same `a.session` of the plan attempt (`review_subject` returns it). The run clones that planner's transcript (design.md ~1000-1050). A rerun from a revised copy would still clone the original planner, which never saw the revision exchange. The tree does not say what that implies for the clone's behaviour.

## What the user sees (after the above)

- `show` -> `ticket_paths` (`serve.rs:~545`) builds `plan_files` from `shown_lane_files(t,p,"plan")` (`scheduler.rs:10900`, `lane_files_by` at 10908), which calls `Ticket::shown_where` (`ticket.rs:1476`).
  - `shown_where` keeps `Complete` attempts plus open Workflow attempts only.
  - With `review/1` Failed and `review/2` open, only `review/2`'s copy is returned. `LaneFile{reviewing, round}` (`serve.rs:~548`, `review_round` at ~527) is computed for that one file.
  - `main.rs:~895-908` `print_lane_files` prints `plan: <path>` and `(reviewed copy, round N, review open)`.
  - There is no output that names a superseded or prior revised copy, and none that says what attempt 2 was built from. The `attempts` list shows `review/1 joined failed` and `review/2 joined running` (the issue's wording), with no link between them.
- The `rerun` question text (`rerun_ask`, `scheduler.rs:9335`; and the `fail_attempt_with` version at 7263) names no plan.
- `finalize` question (`review_done`, `scheduler.rs:7690`) says "The reviewed copy is {copy}", which is the only place a path is named.
- `report.rs` `plan_review` (~340) and `serve.rs` `plan_rounds` (~500) read round files per workflow attempt from `a.artifacts.values().next()`. Attempt 1's rounds (including the owner's round) will still appear under attempt 1.

## What the issue asks, and what the tree gives

- Expected: rerun starts from the newest plan the stage holds, or at least says which copy it reviews in the rerun question and in `show`. The `revise` note should survive a runner failure like an answered decision does.
- The note itself already survives: it is on the decision (`Answered.note`), in `review/1/joined/plan.feedback-2.md`, and in `attempt.revisions`. What does not survive is the effect of the note (the revised copy) as the next attempt's input.
- Nothing in the tree currently records "this attempt's input came from X". `new_attempt(..)` (called at `scheduler.rs:~7480`) takes only the artifacts map. There is no field for the source path on `Attempt`. `Attempt` has `carried_from` for code review only (design.md ~1795).
- Adding a field to `Attempt`/`Ticket` means the record version rule in CLAUDE.md applies for Switchboard's `model.rs`; for Dispatch it is `RECORD_VERSION` (design.md ~1799 mentions "dispatch `RECORD_VERSION` 4") and `store.rs` migrations (`store.rs:~518-547`). I did not read the version or migrate code in full.

## Tests near the change

- `dispatch/tests/first_slice.rs`: `at_finalize`, `at_review_run`, `revise`, `artifact_of` helpers; `revise_writes_the_owners_round...` at 3442; `a_run_switchboard_no_longer_has_fails_the_attempt` at 3809. A test for this issue would drive: finalize, `revise`, then clear `env.sb().runs`, step, answer `rerun`, step, and compare which `plan` file `WorkflowStart` got. The fake Switchboard in `dispatch/tests/support` records `runs` and `objections`.
- `dispatch/src/ticket.rs:1836-1905` covers `shown` / `input` rules.
- `scheduler.rs:12838-12869` has the "finalized: next stage follows the reviewed copy" tests.
- Per CLAUDE.md a user-visible change needs a CHANGELOG.md Unreleased line, and README/design status updates. CLAUDE.md's Dispatch guidance is mostly for the app; Dispatch's own spec is `docs/dispatch.md`, which has the test tables and decision list (item 16, "A review runs on a copy of the reviewed artifact").

## Not checked

- I did not run anything: no tests, no reproduction.
- I did not read `retire_replaced`, `next_n`, `latest_attempt`, `may_rerun` bodies, `new_attempt`, `Attempt` field list, or `store.rs` migration code.
- I did not check other callers of `review_subject` or of `input`/`input_where` for other stages that could take a failed review's copy (e.g. a code-review or a later stage reading `plan`).
- I did not check how `restart`, `park`, or `resume` interact with a failed review attempt's copy (the `Cancelled` case in `ticket.rs:1850` is deliberate).
- I did not check the app side (`ControlAction::StartWorkflow` at `src/core/control.rs:441`) for what it does with a plan path or source session on a second run in the same ticket.
- I did not check the claim that attempt 2's round-1 points were "things the revised copy had already fixed" beyond the code path above; that is the issue's account of a client ticket.

## Open questions

- [tree] What does `StartWorkflow` in the app (`src/core/control.rs:441`, the workflow start in `src/core/`) do with `source` and `plan`: does it read the plan from the path given, clone the planner from `source` at its transcript end, and would a plan newer than that transcript change the clone's behaviour?
- [tree] Which other callers use `review_subject`, `input`, `input_where` or `shown_where` for a Workflow stage's subject, and would a change to "newest plan the stage holds" alter what a later stage, a restart, or `forgotten` handling reads?
- [tree] Does the failed review attempt's copy and round files remain on disk through `retire_replaced`, `restart`, the close, and any sweep (so a rerun could still read `review/1/joined/plan.md`)?
- [tree] How are `Attempt` fields added and versioned in Dispatch (`RECORD_VERSION`, `store.rs` migrate), if the plan needs to record which copy an attempt reviewed?
- [tree] Does a `rerun` question's `--note` on a Workflow stage have any consumer today (`rerun_note` returns `None` for a non-Agent/Review kind), and what does the CLI/port do with a note there?
- [run] Does the failed `review/1` copy actually contain the revised plan in the issue's flow, i.e. does the planner clone write its edits into the `plan` path given (the copy) rather than elsewhere, and is the file complete when the run is gone mid-round? Needs a test run with a real or fake Switchboard.
- [run] What do `show` and the `rerun` question print today for the repro in the issue (Failed `review/1` after `revise`, then a rerun answer)? Needs the first_slice harness or a live data dir to see the exact lines.
- [owner] On the owner's client ticket, was the revised plan the only unpersisted effect of `revise`, or did the planner clone's session state (what it accepted and refuted) also matter to the owner? The tree cannot see their ticket's sessions.
