# Notes: issue #187, review rerun starts from the plan stage's original output

Read: CLAUDE.md, docs/design.md (Durable store, Trust boundary, "Plan review workflow" ~l.1000, runner patience ~l.1780, ticket page Plan tab ~l.2160), docs/lessons.md (general; nothing Dispatch-specific). Line numbers are this worktree (HEAD 7fe8949, Release 0.4.1). The code is in the `dispatch` crate; the app side (`src/core/workflow.rs`) runs the review run.

## The mechanism

The "review stage" is a *workflow* stage (`AttemptKind::Workflow`, stage `review`, context `joined`), not the code-review stage in `dispatch/src/review.rs` (that one has `carried_from`; do not confuse them).

- `dispatch/src/scheduler.rs:7379` `workflow_stage`: per context, takes the latest attempt. A failed attempt with `may_rerun` (the `rerun` answer acted) calls `start_workflow` with `next_n` (l.7401-7412); otherwise `asks_again` -> `ask_rerun` (l.7413).
- `scheduler.rs:7436` `start_workflow`: calls `review_subject(t, &subject)` (l.7463), copies that file to `<attempt dir n>/<subject>.md` (l.7474-7477), records the copy as the new attempt's artifact (l.7499-7506), and sends `Body::WorkflowStart { plan: copy, source: <session>, .. }` (l.7519).
- **Root cause**: `scheduler.rs:10534` `review_subject` walks attempts newest-first but keeps only `state == Complete` (l.10538). Failed `review/1` is skipped, so the newest complete writer of `plan` is `plan/1`, and its copy is what attempt 2 reviews. `Ticket::input_where` (`ticket.rs:1452`) has the same filter.
- Why the revision lives only in attempt 1's copy: the run's planner edits the `plan` path in place. `scheduler.rs:7565` `revise` writes the owner's note to `round_file(subject, round)` beside `review/1/joined/plan.md` (l.7611-7612), pushes `Attempt.revisions` (l.7620-7625), and sends `Body::WorkflowObject { run, round, text }` (l.7632). The planner clone then rewrites `review/1/joined/plan.md`. The attempt is not `Complete` until the run is `Finalized`/`HandedOff` (l.7857-7860), so the revised copy never becomes an input.
- Failure path: `poll_workflow` `scheduler.rs:7741`; `Reply::Failed{reason == NO_SUCH_RUN}` (const at `scheduler.rs:8445`) -> `fail_attempt(.., "review run gone: ..")` (l.7753-7763). `fail_attempt` (l.7148) -> `fail_attempt_with` (l.7195) sets `Failed` and asks `rerun` with options `["rerun","park"]` (l.7270-7290). The question text is built with `rerun_carries` (l.9391), which is empty for a workflow attempt, so it says nothing about which copy will be reviewed. `rerun_ask` (l.9335) rebuilds the same text on a re-ask after a park.
- `Paused{failed:true}` (l.7840-7846) fails the attempt the same way, so a planner or reviewer crash after a `revise` loses the revision identically.
- The finalize question already names its copy (`review_done`, l.7692-7695: "The reviewed copy is {copy}").

## `show` and the plan files

- `show`'s `files:` block: `dispatch/src/main.rs:981-1000`. `print_lane_files` (`main.rs:896-908`) prints `plan: <path>` and, only when `f.reviewing`, "(reviewed copy, round N, review open)". `plan round N (owner): <feedback file>` lines come from `PathsView.plan_rounds`.
- `serve.rs:538` `ticket_paths`: `plan_files` come from `shown_lane_files` (`scheduler.rs:10900`) -> `Ticket::shown_where` (`ticket.rs:1476`), which picks the newest attempt that is `Complete` or an *open* workflow. A failed attempt is excluded, so once `review/2` starts only it is listed. `review/1/joined/plan.md` is listed nowhere though it still exists on disk. `plan_rounds` (`serve.rs:504`, used at l.561-566) reads the newest workflow attempt with artifacts, so attempt 1's owner round and its `revisions` also disappear from `show`.
- `LaneFile` (in the `dispatch-control` crate: lane, stage, path, reviewing, round) is the wire type for a `files:` entry; `PathsView.plan`/`plan_stage`/`plan_reviewed`/`plan_reviewing`/`plan_round` feed the app's Plan tab (design.md ~l.2160). Extending these is additive per CLAUDE.md.
- `review/1 joined failed` / `review/2 joined running` come from `print_attempts` (not read).

## On-disk record and rehydration (before the pane)

- Record: the ticket in Dispatch's data dir (atomic store path). `Attempt{stage, n, context, kind: Workflow, state, artifacts: {plan -> <attempt dir>/plan.md}, run, revisions: Vec<Revision{round, by, at_ms}>, run_sessions}` (`ticket.rs:~347`, `510`). The failed attempt keeps its `artifacts` entry and `revisions`. Nothing records that the copy was edited after it was made, apart from `revisions` and the `plan.feedback-N.md` / `plan.response-N.md` files beside it.
- Owner input: the `revise` note is stored in the answered `Decision.state` (`note`, `by`) and in `plan.feedback-N.md` (heading `# The owner's objection`, `lib.rs:95`). Both survive the failure. What is lost is only the revised plan text: it is never promoted to an input, and attempt 2 neither carries nor mentions the note.
- Rehydration of a rerun: a runner pass re-reads the ticket; `may_rerun` (l.10665) is true once the `rerun` answer is marked acted (after the old processes are confirmed gone); `start_workflow` then recomputes the subject from the ticket's attempts. A fix to the choice of source in `review_subject` is therefore picked up on rehydration with no new field. Recording the chosen source on the new attempt (so `show` can say "reviews the copy from review/1") would be a new serialized `Attempt` field, which needs a version bump and a migration step per CLAUDE.md (design.md ~l.1790 mentions dispatch `RECORD_VERSION` 4; I did not find the current constant).
- Session lineage: `start_workflow` takes `source_session` from the same `review_subject` pick, the plan-stage session whose conversation the new planner clones. A review attempt's own `session` is the planner/reviewer "face" (`scheduler.rs:7777-7781`), so if the path comes from `review/1` the session still has to come from the plan stage. Path and session can no longer come from one attempt.
- Snapshots of rounds live in the app's data dir (`src/core/workflow.rs:43`, `:198`) only while the run record exists; with `no such run` they may be gone.

## Related readers of "the plan"

- `Ticket::input`, `lane_plan`, `lane_input` (`ticket.rs:1437`, `scheduler.rs:10843-10858`) hand later stages only Complete attempts' plans. A fix limited to `review_subject` does not change them.
- Tests that pin the current rules: `ticket.rs:1797-1900` (input vs shown), `scheduler.rs` ~l.12838-12869 (review plan input), `serve.rs:1395-1580` and `1685` (paths, plan_rounds), `events.rs:3313` area (rerun texts).
- Docs to update on completion: design.md plan review / runner patience paragraph (~l.1782: "a rerun kills the old run's agents and starts a fresh run"), CHANGELOG Unreleased.

## Not checked

Ran nothing (no tests, no repro). Did not read `dispatch-control` types, `print_attempts`, `restart.rs` handling of workflow attempts, the `src/core/workflow.rs` planner-write semantics, the review stage's pipeline definition (`pipeline.rs` ~l.2331), or how existing tests fake `NO_SUCH_RUN`.

## Open questions

- [tree] Does any path remove or `forget` a failed workflow attempt's copy or `artifacts` entry (evidence sweep, restart, close), so that `review_subject` could not rely on it?
- [tree] Is `Attempt.revisions` non-empty a sufficient signal that a failed attempt's copy differs from its source, or must the file be compared with the plan stage's file?
- [tree] What do `restart.rs`, `resume`, park and close do with a failed workflow attempt, and do any also call `review_subject` or `start_workflow`?
- [tree] What serde defaults does `dispatch_control::LaneFile`/`PathsView` carry, and does the app's Plan tab match on them so that a new marker is additive?
- [tree] Where is the current dispatch `RECORD_VERSION` and migration step, in case a new `Attempt` field is wanted?
- [tree] How do existing fake-runner tests simulate `NO_SUCH_RUN` on a workflow attempt, so revise -> run gone -> rerun can be tested?
- [source] The app's plan-review planner clone writing `plan.md` in place when the run is lost (Switchboard workflow, killed agent, file write): can a revised copy be half-written or stale when `no such run` is returned?
- [run] Does revise -> kill run -> rerun -> diff `review/1/joined/plan.md` against `review/2/joined/plan.md` show the revision gone on this tree? The reading above says yes; unrun.
- [owner] The issue's claim that a runner restart mid-run produces `review run gone: no such run` (versus the app pruning the run) and that the 09:28 revised copy was complete: needs the owner's ticket record and runner log.
