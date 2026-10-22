# Issue #187: a plan review rerun after a failed attempt drops the owner's revise

Read CLAUDE.md, docs/design.md (the "Plan review workflow" section at
1000, and the Dispatch runner's review and rerun paragraphs around
1776-1800) and docs/lessons.md. Line numbers below are from this
worktree (branch `dispatch/187-...`, at 7fe8949).

## Root cause

A plan review is a `StageKind::Workflow` stage. A rerun of it goes
through `workflow_stage` and then `start_workflow`, and `start_workflow`
takes its input only from attempts that are **Complete**.

- `dispatch/src/scheduler.rs:7379` `workflow_stage`: when the latest
  attempt in the context is failed and `may_rerun` (10665) holds, it
  calls `start_workflow(.., next_n(..))` (7401-7411).
- `dispatch/src/scheduler.rs:7436` `start_workflow`: at 7465 it calls
  `review_subject(t, &subject)`. At 7470-7472 it copies that path into
  `<attempt dir>/<subject>.md`, and at 7509 it starts the run on that
  copy with the same session as `source`.
- `dispatch/src/scheduler.rs:10534` `review_subject`: it walks the
  attempts newest first and **filters on `AttemptState::Complete`**,
  then takes the first one with a `plan` artifact. `review/1` failed
  (`poll_workflow`, 7755-7762, `review run gone: no such run`), so the
  filter skips it, and the newest complete writer of `plan` is
  `plan/1`. Attempt 2 is therefore a copy of `plan/1/joined/plan.md`.

The `revise` path itself works as designed. `revise`
(`scheduler.rs:7565`, called from the `("finalize","revise")` arm at
3495) writes the owner's note as `plan.feedback-<round>.md` beside the
reviewed copy (`report::round_file`, `report.rs:156`). It records a
`Revision { round, by, at_ms }` on the review attempt (7618-7626,
struct at `ticket.rs:510`) and sends `workflow.object`. The planner
clone then edits `review/1/joined/plan.md` in place. So the revision
reached disk correctly and is still there. The problem is that the
rerun never reads attempt 1's directory.

Related gaps on the same path:

- `rerun_note` (`scheduler.rs:8988`) returns `None` for any stage that
  is not `Agent | Review` (8992-8997). A note typed on the `rerun`
  answer for a plan review is therefore dropped as well.
  `WorkflowStart` has no field to carry one in any case.
- `rerun_ask` (`scheduler.rs:9335`) builds the question with
  `rerun_carries` (9382), which says nothing for a Workflow attempt.
  The question never says which copy the rerun will review.
- `Ticket::shown_where` (`ticket.rs:1476`) counts only Complete
  attempts, plus open Workflow ones. `Ticket::input_where`
  (`ticket.rs:1452`) counts only Complete ones. So once `review/1`
  failed, nothing shows its copy any more. There is a test that pins
  this on purpose: `ticket.rs:1827`
  `the_plan_shown_is_an_open_review_copy_but_an_input_never_is`
  ("A parked review's copy is not the plan going forward", 1857-1865).
  That rule is right for downstream stages: implement must not read an
  unfinalized copy. The plan review stage's own rerun is a different
  case.

## On-disk record and rehydration

Everything needed is already on the record and in the attempt
directories. No new file is required.

- The ticket record (`<data dir>/tickets/<id>.json`, through the
  store's atomic write) holds `attempts[]`. `review/1` is
  `kind: workflow, state: failed{"review run gone: no such run"}`. Its
  `artifacts.plan` is `.../review/1/joined/plan.md`, its `revisions`
  holds `[{round: k, by, at_ms}]` (record v19, `store.rs:555`), and
  `run` and `run_sessions` name the dead run.
- The attempt directory `review/1/joined/` holds `plan.md` (the
  revised copy), `plan.feedback-<n>.md` and `plan.response-<n>.md` for
  each round, and the owner's `plan.feedback-<k>.md`, whose first line
  is `OWNER_ROUND_HEADING`.

**Proposed rehydration rule.** When `start_workflow` starts attempt
`n` of stage S in context C, it starts from the newest copy S already
holds. That is the latest earlier attempt of S in C that is a Workflow
attempt, ended Failed or Cancelled (not by a restart that discards it),
has a `plan` artifact not in `forgotten`, and whose file still exists.
Only when there is no such attempt does it fall back to
`review_subject`'s newest complete writer, as now. This can be derived
entirely from the record on every pass, so a restart in the middle of
it rehydrates the same way. No schema change is needed for the input.

Optionally, to make the choice explicit and readable offline, add one
field to `Attempt`, such as `started_from: Option<(String, u32)>`
(stage, n), set when the copy is made. It is additive, but it is a
serialized field, so it needs a dispatch `RECORD_VERSION` bump and a
`store.rs` migrate step, like the v18-to-v19 `revisions` step. I would
add it: `show`, the rerun question and the report can then name the
copy's source without re-deriving it.

**Session to clone.** `review_subject` also returns the source session
for the planner clone. Keep that one: the session of the newest
complete writer, `plan/1`'s planner. Do not use `review/1`'s
`session`. That field is the run's face (set at `poll_workflow`,
7794-7798, to the planner clone), and the clone may have died with the
run. The plan file is the planner's real input, so cloning the
original planner and handing it the revised copy is enough. Split
`review_subject` into "which path" and "which session", rather than
taking both from one attempt.

**The owner's note.** Starting from the revised copy carries the effect
of the `revise`. The note's text survives in attempt 1's
`plan.feedback-<k>.md` and its `revisions` entry. A further step,
which I would not need for this fix, is to name that file in the
reviewer's first-round prompt, the way code review's `REVIEW_CARRIED`
does (`review.rs:44`), so that round 1 does not re-raise what the
owner already decided. If that is wanted, it is a template variable,
since `WorkflowStart` carries no prompt text.

**`start over`.** To keep a way back to the stage's original output,
honour a rerun note saying "start over" (`review.rs:3639`
`starts_over`) by falling back to `review_subject`. That needs
`rerun_note` (or a sibling) to accept `StageKind::Workflow`, and the
note kept on the replaced attempt or under `rework_key`.

## What the views say (after the record)

- Rerun question (`rerun_ask`, `scheduler.rs:9335`; `rerun_carries`,
  9382): for a Workflow attempt whose copy has rounds or revisions,
  add a line like: "A rerun reviews this attempt's copy
  (`review/1/joined/plan.md`, revised in round k by the owner); a note
  saying \"start over\" reviews `<stage output>` instead."
- The `show` files block (`dispatch/src/main.rs:981` → `print_lane_files`,
  `main.rs:896`) is built from `ticket_paths`
  (`dispatch/src/serve.rs:538`):
  - `plan_files` comes from `shown_lane_files` (`scheduler.rs:10900`)
    and therefore lists only `review/2`'s copy.
  - `plan_rounds` is read from `review`
    (`serve.rs:564-569`, the latest Workflow attempt with artifacts),
    so attempt 1's rounds, including the owner's round, disappear from
    `show` once attempt 2 starts. `plan_rounds` and `review_round` are
    at `serve.rs:504` and `serve.rs:527`.
  - Fix: when more than one Workflow attempt of the stage has a copy,
    list the earlier ones as prior revisions, e.g. `plan (prior
    revision, review/1, failed, round k owner): <path>`, and give the
    current one "reviewed copy, from review/1". `LaneFile`
    (`dispatch-control/src/lib.rs:586`) gains an additive optional
    field (`from: Option<String>`, or a `prior: bool` entry kind). It
    is `#[serde(default)]` wire, so this is additive for the app's
    ticket page too.
- The ticket page's Plan tab reads the same `PathsView`, so it gains
  the same lines for free if `plan_rounds` keeps earlier attempts'
  rounds, tagged with their attempt.

## Tests to add

- `dispatch/tests/first_slice.rs`, next to
  `a_failed_review_run_fails_the_attempt_and_a_rerun_starts_a_new_run`
  (3257) and `a_run_switchboard_no_longer_has_fails_the_attempt`
  (~3810). Use `at_finalize`, answer `revise` with a note, have the
  fake planner write a changed `plan.md` into attempt 1's copy, then
  `env.sb().runs.clear()`, step, answer `rerun`, and assert:
  - attempt 2's `plan.md` equals attempt 1's revised copy;
  - `WorkflowStart.plan` is attempt 2's copy and `source` is still the
    plan stage's session;
  - the `rerun` question text names attempt 1's copy.
- The same test with a `start over` note asserts a copy of
  `plan/1/joined/plan.md` instead.
- `ticket.rs:1827`: leave it as it is (downstream readers still never
  see a failed copy). Add a unit test for the new picker beside
  `review_subject`.
- `serve.rs`: a `ticket_paths` test with two Workflow attempts that
  asserts the prior revision and attempt 1's owner round are listed.

## Docs to touch with the fix

- design.md, the paragraph at ~1782 ("a rerun kills the old run's
  agents and starts a fresh run"): add that the fresh run starts from
  the failed attempt's copy unless the note says "start over".
- CHANGELOG Unreleased: one line.

## Not covered here

Why the run was gone (`no such run` after a runner restart) is a
separate question. Dispatch reads it correctly as a failure. Whether
Switchboard should have lost the `WorkflowRun` record at all would be
a question for the app's store and its pruning of `Dispatch:`
definitions, and I did not look into it.
