# Issue #187: review rerun starts from the plan stage's output, not the revised copy

Read: CLAUDE.md, docs/lessons.md, docs/design.md (Switchboard spec; skimmed by grep),
docs/dispatch.md ("A review is a copy", line ~550). Paths below are in `dispatch/src/`
of this worktree. No code written.

## Root cause

`review_subject` (scheduler.rs:10534) picks the subject for a new workflow attempt.
It walks `t.attempts` newest first, keeps only `AttemptState::Complete`, and takes
the first attempt whose `artifacts` holds the subject name (`plan`). A failed
`review/1` is not Complete, so it is skipped and the walk lands on `plan/1`,
the original output. `start_workflow` (scheduler.rs:7436) calls it at 7465, then
copies that file into `review/2/<ctx>/plan.md` (7470-7472) and starts a fresh run
on the copy (`Body::WorkflowStart`, 7506). The rerun path itself
(`workflow_stage`, 7378-7432: `may_rerun` at 10665, then `start_workflow` with
`next_n`) adds no logic of its own, so a rerun and a first start are the same
code. The source *session* returned by `review_subject` has the same problem:
it is the plan writer's session, so the new planner clone also forks from the
original conversation, not the one that applied the revision.

## On-disk record (what exists after a failed attempt)

- `review/1` stays on the ticket record as `Failed { reason: "review run gone: no such run" }`
  (`poll_workflow` 7755-7762 -> `fail_attempt` 7148/7195). Its `artifacts["plan"]`
  still points at `review/1/<ctx>/plan.md`, the revised copy. Nothing deletes or
  `forgotten`-marks it (forgetting is for secrets only).
- The owner's `revise` note: the `finalize` decision is `Answered { note, by }` on
  the ticket. `revise()` (scheduler.rs:7584-7660) wrote it to
  `round_file(<copy>, view.round+1)` (an owner-round file beside the copy, via
  `atomic_write`) and pushed a `Revision { round, by, at_ms }` on the attempt
  (ticket.rs:510). So the note survives a runner failure on disk and in the
  record (decision, round file, `revisions`). What is lost is only the *use*:
  attempt 2 is a new run with new rounds starting at 1 and nothing carries the
  note or the revised copy into it.
- The `rerun` decision is answered and acted; `rerun_ask` (scheduler.rs:9335)
  builds the question. `rerun_carries` (9382) only talks about code-review
  (`AttemptKind::Review`) attempts, returns "" for `Workflow`, so the question
  says nothing about which plan the rerun reviews.

## Rehydration (what a fix must keep true)

Everything needed is already durable, so the fix needs no new field and no
`SCHEMA_VERSION` bump: `review/1.artifacts["plan"]` path, its revision round
files, and its `revisions`. Rehydration after a runner restart is the same
reconcile that already finds a failed attempt and a `may_rerun` answer; the new
attempt must be derived from the record, not from process state.

## Proposed change (for the implementation turn)

1. `review_subject` (scheduler.rs:10534): when the newest attempt of *this stage*
   (the review stage whose subject it is, passed in or via `stage.name`) holds the
   subject artifact and is Failed/Cancelled with a revised copy (non-empty
   `revisions`, or simply the copy exists on disk), prefer that copy over the
   Complete attempt's file. Decide whether the plain "failed before any revise"
   case also starts from the previous copy: that copy was edited in place by
   planner rounds the owner never finalized, so I would limit it to attempts with
   `revisions` non-empty (owner input), or ask the owner. Check the file exists
   (`Path::exists`) and fall back to the Complete source otherwise.
   Keep the session as the original writer's if the revised session is not
   recorded (check `a.run_sessions["planner"]` on the failed attempt: it is
   a better source session when present, but the flag is `run_sessions`, not
   `a.session`; verify the clone works from it before relying on it).
2. Say it in the `rerun` question: extend `rerun_carries` (9382) with a
   `Workflow` arm, e.g. "The next attempt reviews the revised copy at <path>"
   or "reviews the stage's original plan at <path>". Needs the ticket and attempt;
   it already has both through `find_attempt`.
3. Make `finalize_done`-style text accurate too: the finalize question
   (7694) names the reviewed copy; the rerun one should name the same thing.
4. `show` (`main.rs:981-1000`, `print_lane_files` at 896; data from
   `ticket_paths`, serve.rs:539-543 via `shown_lane_files`, scheduler.rs:10900,
   which uses `Ticket::shown_where`, ticket.rs:1475, newest-only): it lists one
   `plan` path per lane, the newest Complete-or-open workflow attempt. A failed
   `review/1` is neither, so only `review/2` appears. Add the earlier copies:
   either extra `LaneFile` rows marked prior (new additive field on
   `dispatch_control::LaneFile`, dispatch-control/src/lib.rs:586, with serde
   default; additive per CLAUDE.md contract rule) and print
   `(prior revision from review/1, not reviewed)` in `print_lane_files`.
   Update docs/dispatch-agent-guide.md ~211-214, which documents the line.
5. Owner note: already durable (see above). Also add the note text, or its round
   file path, to the next attempt's first-round context if the reviewer is not
   told: out of scope unless asked; mention in the PR.

## Tests (follow CLAUDE.md "Adding a feature")

- scheduler.rs tests (core, fakes, `Clock::at`, no sleeping; model on the lanes
  fixtures near scheduler.rs:12358 and `lanes_ticket`): a ticket with
  `plan/1` Complete, `review/1` Failed with `revisions` and a real temp file,
  rerun answered -> assert `WorkflowStart.plan` copy content equals the revised
  file; and the control: failed with no revisions, or revised file missing,
  starts from `plan/1`.
- `review_subject` unit test beside it (10534); `rerun_ask` test for the new
  sentence.
- serve.rs `ticket_paths` tests (serve.rs ~1395-1570) for the prior-copy row;
  main.rs show formatting if it has tests.
- CHANGELOG.md line under Unreleased (user visible); docs/dispatch.md
  "A review is a copy" (line ~550) gets one sentence: a rerun after a failed
  attempt starts from the failed attempt's copy when the owner has revised it.

## Risks / open questions

- Choosing the failed attempt's copy when no revise happened could discard
  nothing but may carry unfinished planner edits; hence the proposed
  `revisions`-only rule. Confirm with the owner.
- A lane-per-context stage: match the attempt by context (`ctx`), like
  `latest_attempt(t, &stage.name, &ctx)` at 7384, not just stage name.
- `review_subject` is called only at scheduler.rs:7465.
