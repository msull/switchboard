# A plan review rerun starts from the failed attempt's copy (#187)

Line numbers are from this worktree at 7fe8949. Today the ticket
record is at `RECORD_VERSION = 28` (`dispatch/src/store.rs:26`) and the
app's store at `SCHEMA_VERSION = 16` (`src/core/model.rs:12`). This plan
bumps neither.

## Where this departs from the issue

- The issue says the `revise` note should "survive a runner failure".
  It already does. `revise` (`dispatch/src/scheduler.rs:7565`) writes
  the note as `plan.feedback-<k>.md` beside `review/1/joined/plan.md`
  and records a `Revision` on the attempt, and both are still on disk
  and in the record after the failure. What gets lost is the revised
  plan: `start_workflow` (`scheduler.rs:7436`) takes its input from
  `review_subject` (`scheduler.rs:10534`), which only considers
  **Complete** attempts, so it skips the failed `review/1` and copies
  `plan/1`'s output. The plan fixes the input. It does not re-send the
  note's text to attempt 2: starting from the revised copy carries the
  note's effect. Showing the owner's earlier decisions to round 1's
  reviewer is listed as a known gap.
- The issue asks `show` to list the earlier copy as a "prior revision"
  next to the reviewed copy. Once attempt 2 starts from attempt 1's
  copy, that copy is not a separate plan; it is where attempt 2 began.
  So `show` marks the reviewed copy with the copy it started from and
  does not add a second `plan:` line.
- The investigation proposed a new `Attempt::started_from` field and a
  record version bump. It is not needed. `Attempt::carried_from:
  Option<(String, u32)>` (`dispatch/src/ticket.rs:317`) already exists,
  is `#[serde(default)]`, and is read only for `AttemptKind::Review`
  attempts: `carry_of` is called from code review paths only
  (`review.rs:548`, `815`, `1776`), and `report.rs:414` filters on
  `AttemptKind::Review`. A plan review attempt sets the same field.
  The serialized type does not change, so there is no migrate step.

## The record

The record has no new field. A `StageKind::Workflow` attempt now sets
`carried_from = Some((stage, m))` when its copy was made from attempt
`m`'s copy. It also sets `rework` to the rerun note it was started
with. Both fields are written when the attempt is made and never
changed. Update the doc comments on both fields (`ticket.rs:311-322`)
to say they also apply to a plan review attempt.

Rehydration is derived from the record on every pass, so a restart
between the `rerun` answer and the new attempt picks the same copy.

## The signals

There is no new signal. The input rule is one pure function next to
`review_subject` in `scheduler.rs`:

`fn revised_copy<'t>(t: &'t Ticket, stage: &str, ctx: &str, subject: &str) -> Option<&'t Attempt>`

It walks `t.attempts` newest first:

- It stops with `None` at the first Complete attempt that wrote
  `subject` and has not forgotten it. A plan rerun after the failure
  (a send-back, a `restart --from plan`) is newer, so it wins.
- It returns the first attempt that matches all of these: an
  `AttemptKind::Workflow` attempt of `stage` in `ctx`; `Failed`, or
  `Cancelled` with a reason that does not start with
  `restart::DISCARDED_BY`; holds `subject` in `artifacts`; and does not
  have it in `forgotten`.

After a chain of failures this is the latest attempt, whose copy
already holds every earlier revision. The function does no file I/O.
The caller checks that the file exists.

## Actions and effects

There is no new `Body` variant or reply, and `WorkflowStart` is
unchanged.

- `start_workflow` (`scheduler.rs:7436`):
  1. Take the note with `t.rework.remove(&rework_key(&stage.name, ctx))`.
  2. Unless the note `starts_over`, look up `revised_copy`. If the
     attempt it returns still has its copy on disk, copy that file
     instead of `source_path` (7470-7472). If the file is missing, log
     one warning and fall back.
  3. Set `carried_from` and `rework` on the new attempt pushed at 7491.
  4. Keep `source` as `review_subject`'s session, the newest complete
     writer's planner. The failed attempt's `session` is the dead run's
     clone (`poll_workflow`, 7794-7798).
- `rerun_note` (`scheduler.rs:8988`): accept `StageKind::Workflow` in
  the `matches!` at 8992-8995, so a note on a plan review's `rerun`
  answer is kept under `rework_key` by `retire_replaced` (3599-3601),
  not dropped. `sent_back` (10600) still excludes Workflow stages, so
  the kept note never holds the stage as sent back. Update its comment
  to say the note is taken by the next attempt.
- `review::starts_over` (`review.rs:3639`) becomes `pub(crate)` and is
  imported by the scheduler.
- `rerun_carries` (`scheduler.rs:9382`) returns `String` rather than
  `&'static str`. For a Workflow attempt `a` that `revised_copy` would
  pick, it adds: ` A rerun reviews this attempt's copy, which has the
  owner's revision of round {k}; a note saying "start over" reviews
  {writer}/{n}'s {subject} instead.` Here `{k}` is the last
  `a.revisions` round, and the first clause drops "which has …" when
  `revisions` is empty. `{writer}/{n}` is the attempt that
  `review_subject` names. `rerun_ask` (9335) uses it unchanged.

## The UI

- `dispatch-control/src/lib.rs:586` `LaneFile` gains `from:
  Option<PathBuf>`: the copy that a plan review's copy started from.
  The struct is `#[serde(default)]`, so older readers ignore it.
- `serve::ticket_paths` (`dispatch/src/serve.rs:538`) sets `from` from
  `a.carried_from` → `find_attempt` → `artifacts[subject]`, for Workflow
  attempts only.
- `print_lane_files` (`dispatch/src/main.rs:896`) prints `    (started
  from <path>)` under a plan line whose `from` is set, whether the
  review is open or not. That earlier path's directory also holds the
  owner's `plan.feedback-<k>.md`.
- No app change: nothing under `src/` reads `plan_files` or `LaneFile`.

## The tests

- `dispatch/tests/first_slice.rs`, after
  `a_run_switchboard_no_longer_has_fails_the_attempt` (3809):
  `a_rerun_after_a_failed_review_reviews_the_revised_copy`.
  1. Set up with `at_finalize` and `revise(&mut env, &id, note)`, then
     step until `workflow.object` is sent.
  2. Overwrite `review/1`'s `plan` artifact with a revised text.
  3. Run `env.sb().runs.clear()` and step until the failure is
     `review run gone: no such run`.
  4. Assert that the pending `rerun` question names the owner's revision
     of round k and says "start over".
  5. Decide `rerun` with no note, then step until `review` attempt 2
     has a run.
  6. Assert that attempt 2's copy reads the revised text, that
     `sb.runs[0].plan` is attempt 2's copy, that `sb.runs[0].source` is
     still `plan`'s session, that `carried_from == Some(("review", 1))`,
     and that `ticket_paths(..).plan_files[0].from` is `review/1`'s
     copy.
- Same file: `a_start_over_rerun_reviews_the_plan_stage_output`. It has
  the same setup and decides `rerun` with the note `start over`. Assert
  that attempt 2's copy equals `plan/1/joined/plan.md`, that
  `carried_from` is `None`, that `rework` starts with `start over`, and
  that `t.rework` is empty.
- `scheduler.rs` `mod tests` (11338): `revised_copy_is_the_newest_copy_the_stage_holds`
  checks the picker's cases:
  - a failed review after the complete plan is picked;
  - a complete plan rerun after the failed review gives `None`;
  - a review cancelled with a `DISCARDED_BY` reason gives `None`;
  - a forgotten artifact gives `None`;
  - another context gives `None`;
  - two failures pick the later one.
- `serve.rs` tests: `a_plan_copy_says_which_copy_it_started_from`. It
  builds plan/1 complete, review/1 failed, and review/2 running with
  `carried_from`, and asserts `plan_files[0].from`. Without
  `carried_from`, `from` is `None`.
- `ticket.rs:1827`
  `the_plan_shown_is_an_open_review_copy_but_an_input_never_is` stays
  as it is: downstream readers still never see a failed copy.

## Docs

- `docs/design.md`, "The control port (2026-09-29)". At 1785 ("a rerun
  kills the old run's agents and starts a fresh run"), add that the
  fresh run reviews the failed attempt's copy, revisions included,
  unless the rerun note says "start over". Also add that `show` names
  the copy the reviewed copy started from. Add a known-gaps line: round
  1 of the rerun is not told the owner's earlier decisions (no
  `REVIEW_CARRIED`-style prompt for plan reviews), and why the run was
  gone is not covered.
- `CHANGELOG.md` Unreleased: "Fixed: rerunning a failed plan review now
  starts from the revised plan, not the plan stage's original; a note
  saying "start over" reviews the original."
- README: no change. There is no new file, dev aid or data directory
  entry.

## Commit

`Dispatch: a plan review rerun starts from the failed attempt's revised copy`

Contract additions:
- `dispatch_control::LaneFile::from: Option<PathBuf>` (additive,
  `#[serde(default)]`).
- `Attempt::carried_from` and `Attempt::rework` are now also set on
  `AttemptKind::Workflow` attempts. No new field, no `RECORD_VERSION`
  bump.
- `rerun_note` keeps a note for `StageKind::Workflow` stages under
  `rework_key`.
