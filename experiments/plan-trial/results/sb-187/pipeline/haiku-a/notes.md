# Issue #187: a review rerun after a failed attempt starts from the original plan

Read-only investigation. Nothing in the repo was changed and no tests were run.
Line numbers are for the worktree `/Users/sbgragent/.dispatch/worktrees/b0b9af6f`
(branch `dispatch/187-dispatch-a-review-rerun-after-a-failed-a`, clean at start).
Paths under `dispatch/` are the Dispatch crate.

## Short answer

The revised copy is not discarded by a bug in a copy step. It is dropped
because the source lookup only accepts **complete** attempts. Attempt 1
(`review/1`) failed with `review run gone`, so it is no longer complete, and
the rerun falls back to `plan/1`. Two things make this silent:

- The copy's display (`shown_where`) also skips failed attempts, so `show` and
  the Plan tab name only attempt 2's copy and never mention attempt 1.
- The owner's `revise` note is kept only on attempt 1 (its `revisions` entry and a
  round file in attempt 1's directory). Nothing carries it into attempt 2, and
  the rerun question does not mention plan copies.

The spec does not decide this case either. `docs/dispatch.md:1046` says a
`{inputs.<artifact>}` is "the path from the newest completed attempt that wrote
it". The code does exactly that. A failed review's revised copy has no rule
behind it.

## The code path, step by step

### 1. `revise` (the owner's answer)

- `dispatch/src/scheduler.rs:3495-3497`: `("finalize", "revise")` routes to `revise`.
- `dispatch/src/scheduler.rs:7565`: `fn revise`. It reads the answered decision's
  note, then `find_attempt(t, stage, number)` for the review attempt (here `review/1`).
- `dispatch/src/scheduler.rs:7616-7617`: writes the round file
  `round_file(&subject, round)` beside the reviewed copy. That is
  `review/1/joined/plan.feedback-<N>.md`, body = `# The owner's objection` + note.
  `dispatch/src/report.rs:156` defines `round_file`; `report.rs:208` is `beside`.
- `dispatch/src/scheduler.rs:7620-7625`: pushes `Revision { round, by, at_ms }`
  onto `review/1`'s `revisions`.
- Then `Body::WorkflowObject` is sent to the run (`REVISE`). The run is the
  review the planner works in place on `review/1/joined/plan.md`, so this is where
  the revised text lives.

### 2. The run fails

- `dispatch/src/scheduler.rs:7741` `poll_workflow`. On `NO_SUCH_RUN`
  (`scheduler.rs:7755-7763`) it calls `fail_attempt` with
  `review run gone: no such run`. That is the reason on the issue.
- `dispatch/src/scheduler.rs:7195-7215` `fail_attempt_with` sets the state to
  `Failed { reason }`, and sets `failed_at_checks` / `failed_at_rewrite`. It touches no
  artifacts. `review/1`'s `plan` artifact still points at `review/1/joined/plan.md`.
- `dispatch/src/scheduler.rs:7308-7332` `ask_rerun` asks the `rerun` question.

### 3. The rerun question

- `dispatch/src/scheduler.rs:9335-9377` `rerun_ask`. For a failed attempt the text is
  `failed: <reason> ... Run it again?` with options `rerun | park`.
- `dispatch/src/scheduler.rs:9382-9390` `rerun_carries` adds carry wording **only** for
  `AttemptKind::Review` (code review). A plan review (`AttemptKind::Workflow`)
  gets no sentence about which copy it reviews, so the question does not say that the
  revision will be dropped.

### 4. The rerun starts from the wrong copy

- `dispatch/src/scheduler.rs:7403` and `:7421`: the stage loop calls
  `start_workflow(..., next_n(...))` when `may_rerun` is set.
- `dispatch/src/scheduler.rs:7436` `fn start_workflow`.
- `dispatch/src/scheduler.rs:7465` `review_subject(t, &subject)`.
- `dispatch/src/scheduler.rs:10534-10548` `review_subject` takes the newest attempt
  with `state == Complete` that has the subject artifact. `review/1` is `Failed`, so it
  is skipped. The result is `plan/1/joined/plan.md`, the original plan stage output.
- `dispatch/src/scheduler.rs:7470-7472`: `copy = dir.join("plan.md")` and
  `std::fs::copy(&source_path, &copy)`. That makes `review/2/joined/plan.md` as a copy
  of `plan/1`, not of `review/1`.
- `dispatch/src/scheduler.rs:7487-7494`: `t.attempts.push(new_attempt(...,
  Workflow, Starting, {plan: copy}))`. There is no `carried_from` and no `revisions`.
- `dispatch/src/scheduler.rs:7510` `Body::WorkflowStart { source, plan: copy, definition,
  reviewer_cwd, reviewer_args }`. **No note is sent.**
- `carried_from` and the carry code (`dispatch/src/review.rs:225`, `carry_source` at
  `review.rs:3608`, `carry_of` at `review.rs:3645`) are for code review attempts only.
  They never run for plan reviews.

### 5. The rule for "newest plan" is the same everywhere

- `dispatch/src/ticket.rs:1452-1462` `Ticket::input_where`: keeps only
  `AttemptState::Complete` (and `!forgotten`). Used for `{inputs.plan}` and for
  agents' prompts.
- `dispatch/src/scheduler.rs:10833-10877` `visible`, `lane_input`, `lane_plan`,
  `lane_plans`: built on `input_where`.
- `dispatch/src/scheduler.rs:10886-10930` `lane_files` / `lane_files_by`:
  same rule for notes and plans.
- `review.rs` (`plan_value`, `decided_text`, `plan_lines`, `review.rs:3784-3860`) only
  renders a plan that `lane_plans` has already picked. None of them looks at
  attempts itself.

### 6. What `show` and the Plan tab display

- `dispatch/src/ticket.rs:1476-1490` `Ticket::shown_where`: keeps `Complete`, or
  `Workflow && is_open()`. `review/1` (failed) is excluded from both. This is why the
  revised copy has disappeared from the display.
- `dispatch/src/serve.rs:538` `ticket_paths`. `plan_files` (`serve.rs:539`) calls
  `shown_lane_files`, which uses `shown_where`. `find_map` returns the **first** match.
  That is `review/2` (open, Workflow), so `plan/1` (complete, the original) is hidden
  too. The result: `files:` names only `review/2`, with `reviewing: true`.
- `serve.rs:564-567`: `review` = the latest Workflow attempt that has any artifact =
  `review/2`. So `plan_rounds` (`serve.rs:504-523`) reads round files beside
  `review/2/joined/`. The owner's `plan.feedback-N.md` in `review/1` is never seen. The owner
  line in `show` (`plan round N (owner)`) disappears for the same reason.
- `serve.rs:526-535` `review_round` reads the attempt's `revisions`. `review/2` has none,
  so `round` is empty.
- `dispatch/src/main.rs:896-909` `print_lane_files` prints `plan (joined): <path>` and
  `(reviewed copy, round N, review open)` for the open one.
- `dispatch/src/main.rs:979-994` the `files:` block. It prints
  `plan round N (owner): ...` only for `view.paths.plan_rounds`, which is empty here.

## What is on disk, then what the pane shows

Ticket directory: `<data>/tickets/<id>/` (`dispatch/src/store.rs:147-149`). Attempt
directory: `<ticket>/<stage>/<n>/<ctx>/` (`scheduler.rs:6303-6315`).

**After `revise` and the failure, before the rerun:**

- `review/1/joined/plan.md`: the revised plan. The planner edits it in place in the run.
  This is the only copy with the owner's change, if the planner's edit landed.
- `review/1/joined/plan.feedback-<N>.md`: the owner's note, under
  `# The owner's objection`.
- `ticket.json`, `attempts[review,1]`: `state: Failed { "review run gone: no such run" }`,
  `run` = the dead run id, `artifacts.plan` = `review/1/joined/plan.md`,
  `revisions: [{ round: N, by, at_ms }]`.
- `ticket.json`, `decisions`: the `finalize` decision, `Answered { revise, note }`. It is
  kept, so the note survives on the record as an answered decision (it is not withdrawn
  or cancelled by the failure).
- `plan/1/joined/plan.md`: the original plan stage output. Nothing has changed it.

**After the rerun (`rerun` answered), before the first round finishes:**

- `review/2/joined/plan.md`: a fresh copy of `plan/1/joined/plan.md`
  (`scheduler.rs:7471`). It is the original text, so the owner's revision is not in it.
- `ticket.json`, `attempts[review,2]`: `state: Running`, a new `run`, `artifacts.plan` =
  `review/2/joined/plan.md`, no `revisions`, no `carried_from`.
- `review/1/...`: unchanged, still holding the revised copy and the feedback file.

**Rehydration.** On a runner restart, nothing re-reads `review/1`. The attempt's run id
and copy path come from `ticket.json`, and `poll_workflow` asks the app for the run. The
app has no run 1, so the attempt fails with `NO_SUCH_RUN`, as described. Attempt 2 is
rehydrated from its own record, which holds only the original text.

**The pane.** The card shows the planner's session (`scheduler.rs:7784`, `face`).
The planner works from `review/2/joined/plan.md`, which is the original text. Its prompt
(`WorkflowStart`) has no owner note, and there is no feedback file beside `review/2`.
So the pane shows a planner reviewing the original plan, and nothing on it mentions the
earlier revision. The Plan tab and `show` show `review/2`'s copy as "Reviewed copy: review
open", with no sign that a revision was discarded. The rerun question does not say so
either.

## Why the expected behaviour is not free

Starting from a failed run's copy has a risk. The run can die mid-edit, so the copy may be
half-written. The planner's answer to a round lands as `plan.response-<N>.md` beside the
copy (`dispatch/src/report.rs:202`, `response_file`). That file is a good signal that the
revision landed. Without it, the copy may not contain the owner's change at all.

The note cannot simply be copied into the new run. `workflow.object` is refused unless the
run is `Converged` or `AtCap` and `round` is the run's round count + 1
(`docs/design.md:1190-1200`). A new run has no rounds yet, so the note can only be
sent after its first review converges. That is a protocol decision, not a local change.

## Options (not implemented)

1. **Show what will be reviewed (the minimum the issue asks for).** Change
   `rerun_ask` (`scheduler.rs:9335`) to name the source copy when a plan review's latest
   attempt has a revision: "the rerun reviews plan/1's original; review/1's revised copy
   (round N) is not carried." Change `shown_lane_files` (`scheduler.rs:10900`) or
   `ticket_paths` (`serve.rs:538`) so `files:` lists both copies, each labelled with its
   attempt and whether it is the reviewed copy or a prior revision. This is a display
   change; no data changes. It fixes "nothing warned".
2. **Start from the revision when it landed.** In `start_workflow` (`scheduler.rs:7465`),
   use the newest Workflow attempt in the same context that has the subject artifact,
   failed or cancelled, and has a `revisions` entry whose round has a
   `response_file` (`report.rs:202`). Copy from that attempt's artifact. If there is no
   response file, fall back to `review_subject` and say so in the question (option 1).
3. **Carry the owner's note.** Option 2 gives the revised text, but the new run's planner
   still does not see the note as a new objection. Either the note is re-sent once the
   new run converges (the existing `revise` path, `scheduler.rs:7565`), or the new
   attempt's round file is written as a record only. This needs a decision from the
   protocol owner before code.

My recommendation: do option 1 now (cheap, removes the silence), and option 2 with the
`response_file` gate, since that is what the owner expects. Leave the note carry (option
3) for a separate change, after checking what the run protocol accepts.

## Tests

- `dispatch/tests/first_slice.rs:3257` `a_failed_review_run_fails_the_attempt_and_a_rerun_starts_a_new_run`:
  covers the failure and a second run, not which copy it reviews.
- `dispatch/tests/first_slice.rs:3807` `a_run_switchboard_no_longer_has_fails_the_attempt`:
  covers the `review run gone: no such run` reason.
- `dispatch/tests/first_slice.rs:3442` (revise writes the round and sends it) and
  `:3600-3690` (revise and re-revise): the revise path, not followed by a failure.
- No test covers revise, then a failed run, then a rerun whose copy must equal the revised
  text. A test next to 3257 would show the bug: `review_run_fails` (`first_slice.rs:3249`),
  then `rerun`, then assert `review/2/joined/plan.md` equals the revised text (or that
  `show` names it).

## Spec notes

- `docs/dispatch.md:1046`: the input rule is "newest completed attempt". The code matches
  the rule. The rule says nothing about a revised copy in a failed review, which is the gap.
- `docs/design.md:1628`: a review runs on a copy of the plan; the copy is the reviewed
  artifact.
- `docs/design.md:2160-2163`: the Plan tab shows "an open review's copy" with its
  round, and "else the newest complete plan". So the spec lets a failed review's copy vanish
  from the Plan tab. That is the documented behaviour, and the issue shows it is a silent
  data loss, so the doc should say which copy wins.
- `docs/design.md:1785-1787`: "a rerun kills the old run's agents and starts a fresh run". It
  does not say what the fresh run reviews.

## Not verified

- No test run, no reproduction, and no app-side read. The claim that the run is killed
  mid-edit (partial copy) is an inference from how the planner writes in place; the app
  code in `src/` was not checked for what a killed run leaves on disk.
- How `print_answered` (`dispatch/src/main.rs:1176`) prints an answered `revise` with its
  note. I did not read it, so I cannot say whether `show` lists the note.
- Whether `review_run_fails` (`first_slice.rs:3249`) and the review fixtures set up
  `revisions` the way a real revise does. I took the test helpers as given.
