# SB-187 investigation notes

Worktree: `/Users/sully/code_repos/plan-trial/sb-187-stage-investigate-v2-sonnet-a`. Line numbers are as in this tree.

Read: `CLAUDE.md`, `docs/lessons.md` in full. `docs/design.md` only by grep and sections near lines 1185-1215, 1522-1630 and 2205-2222. I did not read it end to end, and I did not read `docs/dispatch.md` or `docs/review-stage-plan.md`.

The lessons file is about Prompt Box and says nothing relevant to the bug. The issue is in the `dispatch` crate (the runner), not the egui app.

## Root cause

The bug is in `review_subject`, `dispatch/src/scheduler.rs:10534-10548`. It picks the input for a plan-review attempt:

- It walks `t.attempts` in reverse and keeps only `AttemptState::Complete` attempts.
- It takes the first one whose `artifacts` holds the subject (`plan`).
- It returns that path plus that attempt's `session`.

A failed `review/1` is not `Complete`, so it is skipped. The walk falls through to the plan stage's attempt, `plan/1/joined/plan.md`. That matches the issue.

`start_workflow` calls it at `scheduler.rs:7465`. The same function (`start_workflow`, `scheduler.rs:7436-7530`) then does two things:

- It copies that source into the new attempt dir as `<subject>.md` (`7471-7474`).
- It sends `Body::WorkflowStart { source: source_session, plan: copy, ... }` (`7511-7522`).

`source_session` is the session of the attempt that wrote the source, here the plan stage's agent session. So a rerun is wrong in two ways:

- The file is the original plan.
- The planner conversation it clones is the original planner's, not attempt 1's planner clone, which had already answered the owner's note.

## Where the rerun path is entered

- `review_workflow_stage` (name not confirmed; the function around `scheduler.rs:7380-7430`) handles the plan review stage. Its failed-attempt arm is at about `7401-7418`. If `!held && may_rerun(t, &a)`, it calls `start_workflow(.., next_n(..))`. Otherwise `asks_again` leads to `ask_rerun`.
- `ask_rerun` is at `scheduler.rs:7308-7332`. The question text comes from `rerun_ask` (`9335-9377`): "{stage} ({ctx}) attempt {n} failed: {reason}. Run it again?" plus `rerun_carries` (`9382`) and `rewrite_choices`. `rerun_carries` only speaks about code-review (`AttemptKind::Review`) attempts, so a plan review's rerun question says nothing about which copy it will review.
- The rerun answer is acted on at `scheduler.rs:3462-3530`. In `("rerun","rerun")`, `rerun_note` (`8988-9014`) puts a note back only for `StageKind::Agent | StageKind::Review` stages. A workflow (plan review) stage returns `None` at `8991-8996`. This is how a code review's or agent's note survives a failure (compare the test `a_note_survives_a_review_that_fails_before_its_fix_pass`, `dispatch/tests/first_slice.rs:8386`).
- `fail_attempt` is at `scheduler.rs:7148`; `fail_attempt_with` is at `7195-7230`. The `review run gone` failure comes from `poll_workflow` at `scheduler.rs:7755-7764`, on `Reply::Failed { reason } == NO_SUCH_RUN`. `NO_SUCH_RUN` is defined at `8445`. A paused-failed run fails the same way at about `7813-7829`, with the reason `review: ...`.
- The existing test `a_failed_review_run_fails_the_attempt_and_a_rerun_starts_a_new_run` (`dispatch/tests/first_slice.rs:3257`) covers a rerun starting a fresh run. It checks only that attempt 2 has a run (`env.sb().runs.len() == 2`). It does not check which file or session was the source.

## What the `revise` answer wrote

`revise` is at `scheduler.rs:7565-7662`:

- The owner's note is written to `round_file(&subject, round)` (`report.rs`), beside attempt 1's reviewed copy. It has the `OWNER_ROUND_HEADING` heading. The write goes through `store::atomic_write` (`7631`).
- `Attempt.revisions` (`ticket.rs:346-349`, struct `Revision` at `ticket.rs:510`) gets `{round, by, at_ms}`. It is pushed before `workflow.object` is sent and popped when Switchboard refuses.
- `Body::WorkflowObject { run, round, text }` is sent to Switchboard. Switchboard's planner (a clone of the plan session) rewrites the copy in place, according to the comment at `ticket.rs:1473` and `docs/design.md:2215`.

So the on-disk facts about the owner's input are:

- The note text lives in `<attempt-1 dir>/plan.feedback-N.md`.
- `revisions` lives on attempt 1's record in the ticket file.
- The revised plan lives at `review/1/joined/plan.md`, edited in place.
- Attempt 1's record has `state = Failed`.

None of this is read by `start_workflow` for attempt 2.

## On-disk record and rehydration (before the pane)

Records involved, all in Dispatch's data dir (ticket file is written through `store`'s atomic path):

- The `Ticket` record's `attempts[]`. Each `Attempt` has `stage`, `n`, `context`, `kind` (`Agent` or `Workflow`), `state`, `artifacts: name -> path`, `session`, `run`, `revisions`, `run_sessions`, `forgotten`.
- The artifact files themselves, one per attempt dir: `<attempt dir>/plan.md` (the reviewed copy), `plan.feedback-N.md` (round feedback) and the matching response file.
- A rerun rehydrates only from this record plus those files. The Switchboard run (`run` id) is a cache that can vanish; it did (`no such run`).

The plan at the revised stage of a failed attempt is therefore recoverable from disk. It is `attempts[review/1].artifacts["plan"]`, and the file exists. Dispatch simply never consults a non-Complete attempt for it. The `revise` note is on disk too (round file and `revisions`) but is not carried as a `rework` note for workflow stages.

How the other pickers treat the same data:

- `Ticket::input_where` (`ticket.rs:1452-1463`) is `Complete` only and is what agents are handed. Tests at `ticket.rs:1835-1900` pin its behavior.
- `Ticket::shown_where` (`ticket.rs:1476-1491`) accepts `Complete` or an open `Workflow` attempt. A `Failed` workflow attempt is not shown. The test at `ticket.rs:1862` asserts "A parked review's copy is not the plan going forward" for `Cancelled` and `Failed` states. That assertion states the current intended behavior and would have to change or be narrowed if a failed attempt's copy is to count.
- `review_subject` has its own filter (Complete only) that duplicates `input_where` without `forgotten` and lane handling.

Three ids to keep apart (CLAUDE.md): record id (the ticket and attempt `n`), host id (Switchboard run and session ids), resume handle (the plan session being cloned as `source`). The source session in the new `WorkflowStart` is the plan stage's session, not the first review's planner.

## What `show` prints

- `show` is `main.rs:911-994`. The `files:` block is `main.rs:981-1000`: `print_lane_files("plan", &p.plan_files)` (`main.rs:896-909`), then `round`, `review summary`, `plan round N (owner)` lines from `plan_rounds`, then notes and PR.
- `plan_files` comes from `ticket_paths` (`serve.rs:536-593`), which uses `shown_lane_files` -> `lane_files_by(.., shown = true)` (`scheduler.rs:10900-10950`) -> `Ticket::shown_where`.
- With attempt 1 `Failed`, attempt 2 `Running`: `shown_where` skips attempt 1 (not Complete, not open) and shows attempt 2's copy, marked `(reviewed copy, round N, review open)`. That is the issue's "the files line names only the attempt-2 copy".
- `plan_rounds` (`serve.rs:513-530`) and `review` (`serve.rs:561-565`) pick the latest Workflow attempt with artifacts, which is attempt 2 after the rerun. Attempt 1's `plan round N (owner)` line is therefore no longer listed once attempt 2 exists. So `show` loses the owner's note path as well.
- Nothing in `print_attempts` or elsewhere says which attempt's copy was the source of attempt 2. The `Attempt` record has no field for "reviewed copy derived from X"; `carried_from` exists (used by code review, `review.rs:~190`) but is for rounds' points on `AttemptKind::Review`.
- The ticket page in the app reads the same `PathsView` (`dispatch-control` `PathsView`, `lib.rs:548-578`).

## Code review's carry mechanism, for comparison

`start_review` (`dispatch/src/review.rs:~125-175`) removes the stage's rework note from `t.rework` or takes `unspent_note`, and sets `attempt.carried_from = carry_source(..)` unless the note says "start over". So the repo already has a convention where a rerun carries from the failed attempt, and a note survives a failure. The plan review (`Workflow` attempt) has none of it. `unspent_note` and `carry_source` were not read.

## Behaviour that would have to be decided before a plan

- Which attempts count as "the newest plan the stage holds": the latest attempt of the review stage whose copy exists on disk, whatever its state; or only attempts that have a `revisions` entry; or only `Failed`/`Cancelled` reviews of the same context.
- Whether a `Cancelled` review (parked, restarted) should behave like `Failed`. The test at `ticket.rs:1862-1869` says a parked review's copy is not the plan going forward.
- Whether `restart` (`dispatch/src/restart.rs:802-835`) quoting notes and clearing attempts interacts with this. I read only the headers.
- What the `source` session is for the rerun when the copy comes from attempt 1: the plan session, attempt 1's planner clone (`run_sessions["planner"]`, which `note_run_sessions` records at `scheduler.rs:7690-7720`), or neither.

## Not checked

- `unspent_note`, `carry_source`, `rework_key`, `sent_back`, `asks_again` and `may_rerun` bodies.
- The `restart.rs` interaction beyond comments.
- `lane_files_by` for per-lane plan review stages (`runs_per_lane`); I only read the non-lane path.
- The Switchboard side of `workflow.start` and `workflow.object` (`src/core/dispatch.rs`, the workflow runner): what it does with the `plan:` file and `source:` session.
- Whether any pipeline in `docs/dispatch-pipeline-guide.md` or the supervisor prompt (`supervisor.rs:~69-96`) documents a rerun of a review.
- I ran nothing: no build, no test, no repro.
- I did not read `docs/dispatch.md`, `docs/review-stage-plan.md`, or `CHANGELOG.md`.

## Open questions

- [tree] Does the failed attempt's copy (`review/1/.../plan.md`) always exist on disk when the attempt fails with `review run gone`, or can cleanup/sweep (the `forgotten` map, evidence sweeps, run snapshot cleanup) remove it before the rerun?
- [tree] Does the plan review stage ever run per lane, so that the right source differs by context? `lane_files_by` (`scheduler.rs:10908`) and `review_subject` (no context argument) disagree on lane handling.
- [tree] What do `carry_source`, `unspent_note` and `rework_key` do for a `Workflow` stage, and can one of them carry the `revise` note without a new field?
- [tree] Which callers of `shown_where`, `input_where` and `review_subject` rely on a failed review's copy being ignored (the app's Plan tab, supervisor, `report.rs` plan_review, `restart.rs`)?
- [tree] Does a `restart` of the plan stage, or a `revise` on a later round, already create a "newer plan" case that `review_subject` must keep ignoring (plan rerun after a finished review, `ticket.rs:1866`)?
- [tree] What does the attempt record need so `show` can say which copy attempt N reviews (a new additive field on `Attempt`, which means a `SCHEMA_VERSION`-style bump? Dispatch's `RECORD_VERSION` rule needs reading)?
- [tree] How does the control-port `PathsView`/`LaneFile` change (additive only per CLAUDE.md) reach the app's Plan tab, so the app and `show` stay consistent?
- [source] What `workflow.start` (`Body::WorkflowStart`) does in Switchboard with `source` and `plan`: whether it clones `source`'s transcript into a planner and whether a planner cloned from the original plan session, given a revised plan file, behaves coherently (the mechanism is the `clone_all` transcript port in `docs/design.md`).
- [source] Why does the planner's in-place edit of the copy at `review/1/.../plan.md` happen only in the file, while the planner's conversation holds the owner's note: if attempt 2 clones the original plan session, does the owner's note exist anywhere in its context?
- [run] Reproduce the issue's steps in the fake-based integration environment (`dispatch/tests/first_slice.rs`, `Env`, `at_review_run`, `review_run_fails`) with a `revise` before the failure: which path does attempt 2's `WorkflowStart.plan` copy and which `source` does it send?
- [run] Does `review run gone` on a restart of the runner occur because the Switchboard app restarted (run state lost), and does the new run on the same ticket see the old run's snapshot directory? (A runner restart was the issue's trigger.)
- [run] Does `show --json` of the issue's ticket list attempt 1's `plan round N (owner)` after attempt 2 starts? Reading says no; unverified.
- [owner] Should the rerun silently start from the failed attempt's revised copy, or ask in the `rerun` question which copy it will review, or both? (The issue says "or at least say".)
- [owner] If the revised copy is used, should the owner's `revise` note and the planner's answer to it (round file and response) also be shown to attempt 2's reviewer, or only the revised plan?
- [owner] Is a revised copy in a `Cancelled` review (parked mid-review, then resumed) meant to count the same as in a `Failed` one?
- [owner] How is the issue's claim "Two of attempt 2's three round-1 points were things the revised copy had already fixed" and "contradicts one of the owner's answers" to be treated: accepted as the symptom, or does it need the owner's ticket to check? I did not have the ticket.
- [owner] What wording and where in `show`: is a line such as `plan (reviewed copy of review/1)` acceptable, and does it need the app's Plan tab to match?
