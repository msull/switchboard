# Notes: issue #187, review rerun discards the owner's revise

Read: CLAUDE.md in full, docs/lessons.md (generic template lessons, nothing
specific), docs/design.md by grep only (Dispatch sections ~1522-1640, the
workflow/objection paragraph ~1185-1200; not the whole 3678 lines),
docs/dispatch.md at the lines cited below. The code is in the `dispatch`
crate, not `src/core`; `src/core/workflow.rs` is only the app side of the
port. Line numbers are as in this worktree.

## Cause (confirmed by reading, not run)

A new plan-review attempt takes its input from the newest **Complete**
attempt that wrote `plan`, and a Failed attempt is never Complete.

- `dispatch/src/scheduler.rs:7436` `start_workflow` is called for the first
  attempt and for every rerun (call sites at ~7400-7425: `Some(a)` failed and
  `may_rerun`, or `None`).
- `scheduler.rs:7465` calls `review_subject(t, &subject)`;
  `scheduler.rs:10534-10549` `review_subject` does
  `t.attempts.iter().rev().filter(state == Complete).find_map(artifacts[subject])`
  and also returns that attempt's `session`. With review/1 Failed, it lands on
  `plan/1`.
- `scheduler.rs:7469-7472` makes `<attempt dir>/plan.md` by `std::fs::copy`
  from that path. So review/2/joined/plan.md is a copy of plan/1's output.
- `scheduler.rs:7515` sends `Body::WorkflowStart { source, plan: copy, .. }`.
  `source` is the plan stage's session. Switchboard clones that session's
  transcript for the planner
  (`src/core/workflow.rs:289` `start_workflow`, the
  `CloneAllTranscript` effect). So attempt 2's planner also has no memory of
  attempt 1's rounds or the owner's objection, even if the file were right.
  `Attempt.session` of review/1 is the planner clone (`poll_workflow`,
  `scheduler.rs:~7790`, "the card for the ticket is the planner's"), a
  different session that holds that history.
- The same `Complete` filter is used by `Ticket::input_where`
  (`dispatch/src/ticket.rs:1452-1465`), which is what agents are handed.
  Only `Ticket::shown_where` (`ticket.rs:1476-1492`) also counts an open
  Workflow attempt. Display and input already differ on purpose
  ("an agent is handed `input_where`'s complete files").

## Where the revision lives

- The reviewed copy is review/N's `artifacts["plan"]`, edited in place by the
  planner clone through the rounds (docs/dispatch.md:2909: "The copy in the
  review attempt changes; the plan attempt's file does not"). The revised plan
  of the issue is review/1/joined/plan.md.
- `revise` is `scheduler.rs:7565`. It writes
  `<stem>.feedback-<round>.md` beside the reviewed copy via
  `report::round_file` (`dispatch/src/report.rs:156`), headed
  `OWNER_ROUND_HEADING` (`dispatch/src/lib.rs:96`), pushes a `Revision
  {round, by, at_ms}` on review/1 (`dispatch/src/ticket.rs:350`, struct at
  ~510), then sends `WorkflowObject`. So the owner's note survives on disk in
  review/1's attempt dir and in `review/1.revisions`; nothing reads it for
  attempt 2.
- The failure itself: `poll_workflow` (`scheduler.rs:7731-7765`) fails the
  attempt on Switchboard's `no such run` reply (`NO_SUCH_RUN`,
  `scheduler.rs:8445`) with "review run gone: no such run".
  Test: `dispatch/tests/first_slice.rs:3807`
  `a_run_switchboard_no_longer_has_fails_the_attempt`.
- Other attempts' artifacts are kept: nothing deletes review/1's copy on
  failure (not exhaustively checked; see questions).

## On-disk record and rehydration (before the pane)

- Record: `Ticket.attempts[]` in the ticket JSON under Dispatch's data dir
  (`dispatch/src/ticket.rs:266` `Attempt`). Review/1 holds `state: Failed
  {reason}`, `artifacts{plan -> .../review/1/joined/plan.md}`, `revisions`,
  `run`, `session` (planner clone), `run_sessions`. Files sit beside the
  artifact path.
- Rehydration is not a process resume. The scheduler pass `workflow_stage`
  (`scheduler.rs:7379`) re-derives everything from the record: latest attempt
  per context, `may_rerun` (`scheduler.rs:10665`) after a `rerun` answer, then
  `start_workflow(.., next_n(..))` (`next_n` at 10205). So the fix that fits
  "workflows are data" is a pure function of the record: choose the
  rerun's input from the record (the failed attempt's artifact, if its file
  still exists and the attempt holds revisions or edits), not a new field.
  Whether the failed attempt's copy counts as "newest plan the stage holds"
  is a rule the record can state: the latest attempt of this stage with a
  `plan` artifact whose file is a file, Complete or not. A new serialized
  field would need a bump of the schema version and a step in the store's
  `migrate` (CLAUDE.md "Extending the contract"; Dispatch's version history
  is in docs/dispatch.md ~360-400 and I did not locate the const).
- Rerun question text: `rerun_ask` `scheduler.rs:9335`; `rerun_carries`
  `scheduler.rs:9382` is the one place a rerun question says what the next
  attempt carries, and only for code reviews (`AttemptKind::Review`). A
  workflow attempt returns an empty string, so the question says nothing about
  which copy a plan rerun will review. The question is created by `ask_rerun`
  (`scheduler.rs:7308`) through `ensure_decision` and re-asked by
  `ask_again_unslotted`; `rerun_ask` takes `(&Ticket, &Attempt)` only, so
  naming a path needs no new input.
- Code review reruns already have a carry mechanism (`carried_from`,
  docs/dispatch.md:3058, "start over" phrase at 3060). The plan review has
  none; that is the nearest precedent for design.

## `show` (the pane/CLI)

- `dispatch/src/main.rs:981-1001` prints `files:`; `print_lane_files`
  (`main.rs:896-910`) prints `plan: <path>` and, for an open workflow attempt,
  `(reviewed copy, round N, review open)`.
- Data: `dispatch/src/serve.rs:539` `ticket_paths` builds `plan_files` from
  `shown_lane_files` (`scheduler.rs:10900`) -> `lane_files_by`
  (`scheduler.rs:10908`) -> `shown_where` (ticket.rs). Only the single newest
  match is returned (Failed attempts are excluded; Complete or open Workflow
  only), so with review/1 Failed and review/2 Running, `plan_files` has just
  review/2's copy. That matches the issue's "files line names only the
  attempt-2 copy".
- `plan_rounds` (`serve.rs:504`) and `review` (`serve.rs:~563`) use the newest
  Workflow attempt with artifacts, i.e. review/2. review/1's owner objection
  file and rounds are not listed anywhere in `show` once review/2 exists.
- `LaneFile` is in `dispatch-control/src/lib.rs:586` (wire type shared with
  the app; extend additively per CLAUDE.md). The app also reads these
  (design.md "Dispatch ticket page", ~1201 "Plan tab"): a new field there
  reaches the app UI, which I did not read.

## Tests that exist nearby (not run)

`dispatch/tests/first_slice.rs`: `at_review_run` 3231, `at_finalize` 3365,
revise tests ~3439-3640, the gone-run test 3807. No test covers
revise -> failure -> rerun. docs/dispatch.md table rows ~2975 (revise) and
~2502 (gone run) would get a new row; CHANGELOG Unreleased entry needed
(CLAUDE.md).

## Not checked

- Did not run anything (no tests, no app, no Switchboard).
- Did not read `store.rs` migrate, the schema-version const, `resume_with`
  (`scheduler.rs:8262`), `park_reruns` (8344), `may_rerun` bodies, or
  `rerun_note` (8988), or `new_attempt`/`next_n` bodies.
- Did not read the app-side Dispatch ticket page for how `plan_files` /
  `plan_rounds` render.
- Did not verify that failing an attempt leaves its artifact file on disk in
  every path (fail_attempt at `scheduler.rs:7148`, evidence sweep/prune at
  close).

## Open questions

- [tree] Does `fail_attempt` / `fail_attempt_with` (scheduler.rs:7148, 7195) or any sweep delete or move a failed workflow attempt's `plan.md` and feedback files before the rerun?
- [tree] Does anything else besides `start_workflow` pick a plan for a later stage after a failed-then-rerun review, e.g. `lane_input`/`vars_for` `inputs.plan` for the implementer, so a fix in `review_subject` alone is or is not enough?
- [tree] What do `may_rerun`, `resume_with` and `park_reruns` do with a failed workflow attempt's record (artifacts, `revisions`, `run`) when the owner answers `rerun` or `dispatch resume`, and do they route through the same `start_workflow`?
- [tree] Which schema version is Dispatch's current one and does a derived rule (no new field) avoid a bump, versus the cases a field would be needed?
- [tree] How does the app's Dispatch ticket page use `LaneFile`, `plan_files` and `plan_rounds`, and what would adding a prior-revision entry do there?
- [tree] What does a failed run with no revisions look like (failed in round 1, before any owner input): is its copy the plan stage's file edited by the planner, and should it count as newer than plan/1?
- [source] Switchboard `workflow.start` with a `source` session: is the planner clone's transcript taken from the source session only, and can the source be the previous planner clone (review/1's `session`) so that attempt 2's planner has seen the rounds and the owner's objection? (Switchboard's clone of a Claude Code transcript, `CloneAllTranscript`.)
- [run] After a runner restart mid-run, which `Reply::Failed` words does Switchboard actually return for the lost run (`no such run`), and does the run's reviewed copy on disk keep the planner's last edits at that moment (is review/1's `plan.md` complete when the failure is noticed)?
- [owner] The issue says the revised copy was written at 09:28 and attempt 2's round-1 points were things the revision had already fixed; whether the revised file in the owner's real ticket directory is still on disk and unchanged is only observable on the owner's machine.
- [owner] The issue claims the `rerun` question and `show` gave no warning; the exact `show` and decision text the owner saw on their build (version before 0.4.1) is the owner's.
