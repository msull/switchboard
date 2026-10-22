# Issue #187: a review rerun after a failed attempt drops the owner's revise

Read: CLAUDE.md, docs/design.md (Durable store, Trust boundary, Plan review
workflow, Dispatch ticket page), docs/lessons.md. All line numbers are as
they are in this worktree (branch `dispatch/187-...`, at 7fe8949).

## Root cause, in one line

A new plan review attempt takes its source plan from the newest **Complete**
attempt that wrote `plan`. A failed review attempt is never Complete, so its
revised copy is skipped and the copy falls back to `plan/1/joined/plan.md`.

## The code path

1. `run gone` fails the attempt.
   `dispatch/src/scheduler.rs:7741` `poll_workflow`: a `Reply::Failed`
   with `NO_SUCH_RUN` (`scheduler.rs:8445`) calls `fail_attempt` with
   `"review run gone: no such run"` (`scheduler.rs:7755-7763`). A failed
   `Paused { failed: true }` does the same (`scheduler.rs:7811-7823`). The
   comment there says outright: "a rerun starts a fresh review". That
   intent is the bug once a `revise` has landed.

2. The rerun question.
   `scheduler.rs:7404-7414` (`workflow_stage`, starting at `7379`): the
   latest attempt failed, so `asks_again` (`10608`) leads to
   `ask_rerun` (`7308`) and then `rerun_ask` (`9335`). The question is
   `"review (joined) attempt 1 failed: review run gone: no such run. Run it again?"`.
   `rerun_carries` (`9380-9388`) adds a sentence only for a code review
   (`AttemptKind::Review`) or a resolution, and nothing for a workflow
   (plan review) attempt. So the question never says which plan copy the
   rerun will review.

3. The `rerun` answer.
   `scheduler.rs:3523-3528`, then `retire_replaced` (`3578`): it kills the
   attempt's processes (`retire_processes`, kill only, so session records
   stay) and marks the answer acted. Nothing is deleted on disk:
   `review/1/joined/plan.md` and its `plan.feedback-N.md` survive.
   `may_rerun` (`10665`) then lets `workflow_stage` call `start_workflow`
   with `next_n` (`scheduler.rs:7400-7411`).

4. **The input choice. This is the bug.**
   `start_workflow` (`scheduler.rs:7434`) calls `review_subject(t, &subject)`
   (`scheduler.rs:7462`) and copies the result into the new attempt's
   directory (`7469-7472`):
   ```rust
   fn review_subject(t: &Ticket, subject: &str) -> Result<(PathBuf, String), String> {
       t.attempts.iter().rev()
           .filter(|a| a.state == AttemptState::Complete)   // scheduler.rs:10539
           .find_map(|a| a.artifacts.get(subject).map(|path| (path.clone(), a.session.clone())))
   ```
   `review/1` is `Failed`, so this picks `plan/1`'s `plan.md` and the plan
   stage's session. The same rule (`Complete` only) is `Ticket::input_where`
   (`dispatch/src/ticket.rs:1452-1464`), which agents' `{inputs.plan}` use.
   `Ticket::shown_where` (`ticket.rs:1476-1490`) also counts an *open*
   workflow attempt, but not a failed one.

5. Why `show` says nothing.
   `dispatch/src/serve.rs:538` `ticket_paths`:
   - `plan_files` comes from `shown_lane_files(t, p, "plan")`
     (`scheduler.rs:10900`, via `lane_files_by` and `shown_where`). That is
     one newest file, attempt 2's open copy. Attempt 1's revised copy is
     not listed at all.
   - `review` (`serve.rs:562-567`) is the newest workflow attempt with
     artifacts, so `plan_rounds` (`serve.rs:504`) lists only attempt 2's
     rounds. Attempt 1's owner round (`plan.feedback-N.md`, the `revise`
     note, with `OWNER_ROUND_HEADING`) disappears from `files:` too.
   - Printing: `dispatch/src/main.rs:896` `print_lane_files` and
     `main.rs:980-997`.

## On-disk record (what exists after the repro)

The ticket record (`Ticket`, Dispatch's data dir, written through
`store::atomic_write`) already holds everything a fix needs. **No schema
change is required.**

- `attempts[review/1]`: `kind: Workflow`,
  `state: Failed { reason: "review run gone: no such run" }`,
  `artifacts: { plan: <tickets>/<id>/review/1/joined/plan.md }` (set at
  `scheduler.rs:7487-7495`), `revisions: [Revision { round, by, at_ms }]`
  (`ticket.rs:510`, pushed by `revise` at `scheduler.rs:7614-7621`),
  `session`: the planner clone (`poll_workflow`, `7786-7790`),
  `run_sessions`.
- Files beside the copy: `review/1/joined/plan.md` (revised in place by the
  planner clone), `plan.feedback-<n>.md` (the reviewer's and the owner's
  rounds; `report::round_file`, `report.rs:156`), and `plan.response-<n>.md`.
- The `finalize` decision is answered `revise` with the note, `acted`, and
  the `rerun` decision is answered `rerun`, `acted`.
- `attempts[review/2]`: `artifacts: { plan: .../review/2/joined/plan.md }`,
  a byte copy of `plan/1/joined/plan.md`.

## Rehydration (what the fix should do with that record)

The rule: **a new review attempt starts from the newest plan the stage
holds**. That is the previous review attempt's copy when that attempt has
at least one `revision` (the owner's input landed and the planner was told
to revise), else the newest Complete writer as today.

Proposed change, all in the dispatch crate:

1. A pure helper beside `review_subject` (`scheduler.rs:10534`) that picks
   the source copy. For a failed or cancelled `Workflow` attempt of *this
   stage and context* with `!revisions.is_empty()` and its copy still on
   disk, newer than the newest Complete writer, take its `artifacts[subject]`.
   Otherwise take the current pick. Return which one it chose, so the
   question and the log can say so.
   - Source session: keep the plan stage's session (the Complete writer's)
     as `WorkflowStart.source`. The failed attempt's planner clone has been
     killed by `retire_replaced`. Its record probably survives, but cloning
     from a session whose run died mid-revise is a new mechanism, not worth
     the risk here. The revised file carries the revision, and the planner
     sees it as the plan it is handed.
   - Optionally, also copy the previous attempt's owner round files
     (`plan.feedback-<n>.md` whose `Revision` exists) into the new attempt's
     directory, or name them in the reviewer's context, so "two of three
     round-1 points were already fixed" does not recur. The minimum fix is
     the copy choice; this part is a nice-to-have.
   - Do **not** change `Ticket::input_where`. An agent stage after the
     review must still read only a finalized plan. A failed review's
     half-revised copy is not a finished plan for an implementer, only a
     better starting point for another review.
   - What revisions count: a revision whose round never got a response
     (the run died before the planner wrote anything) still means the copy
     on disk is whatever the planner last wrote. That is still the newest
     plan. Taking the copy whenever `revisions` is non-empty is simple and
     safe, because the copy is never worse than `plan/1` plus the
     reviewer's own earlier edits. An open question for the owner: should a
     failed attempt with reviewer rounds but *no* owner `revise` also carry
     its copy? The issue asks only about `revise`. Carrying it whenever the
     copy differs from the source would be the general rule, but it changes
     the "fresh review" behaviour that existing tests rely on.

2. The rerun question. In `rerun_carries` (`scheduler.rs:9380`), add a
   `Workflow` arm: "A rerun reviews <path of the copy it will start from>
   (attempt 1's revised copy, with the owner's round N)" or "... the plan
   stage's plan". It must use the same helper, so the question and the
   launch cannot disagree.

3. `show` and the port. In `ticket_paths` (`serve.rs:538`), when an earlier
   workflow attempt of the stage has a copy, list it as a prior revision
   (for example `plan (attempt 1, revised, failed): <path>`), and keep its
   owner rounds in `plan_rounds`. This is an additive field on
   `PathsView`/`LaneFile` in `dispatch-control/src/lib.rs:560-599`
   (`#[serde(default)]` is already on both, so old clients read it). Then
   print it in `main.rs:980-997`. The app's Plan tab
   (`src/ui/ticket.rs:154` reads `plan_rounds`) can follow later.

4. The log line and event: `start_workflow` should log or say "review/2
   starts from review/1's revised copy" so the timeline shows it.

## Pane (last)

Once the record is right, the Plan tab already shows attempt 2's open copy
labelled "Reviewed copy, round N: review open". After the fix, that copy
holds the revision. A follow-up could list prior attempts' copies under it,
the way `show` would.

## Tests to add (`dispatch/tests/first_slice.rs`)

Helpers already exist: `at_finalize`, `revise` (`first_slice.rs:3417`),
`artifact_of`, and the run-gone setup from
`a_run_switchboard_no_longer_has_fails_the_attempt` (`first_slice.rs:3801`,
which does `env.sb().runs.clear()`).

- `a_rerun_after_revise_reviews_the_revised_copy`: at_finalize, revise
  with a note, then write distinct text into `review/1`'s copy (standing in
  for the planner), clear runs, step, answer `rerun`, step. Assert that
  `review/2`'s copy equals `review/1`'s, and that `WorkflowStart.plan` is
  the new copy.
- `a_rerun_with_no_revise_starts_from_the_plan_stage`: the existing
  behaviour, pinned.
- The rerun question for a revised workflow attempt names the copy it will
  review.
- `ticket_paths` lists the prior attempt's copy and its owner round
  (unit test beside `serve.rs:1586`).
- A later agent stage's `{inputs.plan}` still reads the finalized copy,
  not the failed attempt's (guards against changing `input_where`).

## Docs to touch with the fix

- `docs/design.md`, "Plan review workflow" / "As built": one sentence on
  what a rerun after a failed attempt reviews.
- `CHANGELOG.md` Unreleased: a user-visible change.
- The comment at `scheduler.rs:7812-7814` ("a rerun starts a fresh
  review") needs the new rule.
