# Plan: #187, a plan-review rerun starts from the newest copy the stage holds

Checked against the worktree at HEAD `7fe8949`. "Facts" is `facts.md` for this issue.

## Where this departs from the issue

- **"The `revise` note … should survive a runner failure the same way an answered decision does."** It already does, so the plan changes nothing for the note. `revise` writes `plan.feedback-<n>.md` beside the reviewed copy (`dispatch/src/scheduler.rs:7616-7618`) and pushes a `Revision` on the attempt (7621). The answered decision keeps the note, and `print_answered` prints it (`dispatch/src/main.rs:1176-1198`). Nothing deletes an attempt directory (Facts, Settled, first bullet). What is lost is the *revised copy as the rerun's input*, not the note.
- **"Nothing warned … `show` … names only the attempt-2 copy."** True of the `files:` block (`dispatch/src/serve.rs:539-568`) and of the rerun question (`rerun_carries` returns `""` for a workflow stage, `scheduler.rs:9382-9390`). Not true of the answered list.
- **Repro step 2, "a runner restart mid-run" gives `review run gone`.** The tree does not bear this out. The app drops a run only through `remove_workflow` (`src/core/workflow.rs:1009-1028`) or a store reset (Facts, Settled). This is a For-the-owner question (did the app lose the run?). The plan does not depend on the answer: the fix handles a failed review attempt of any reason. The test models the failure with `runs.clear()` (`dispatch/tests/support/mod.rs:690`), which the existing test at `first_slice.rs:3817` already uses.
- **"Take an issue through plan and review; at finalize answer `revise`."** Matches the code. The cause is `review_subject` (`scheduler.rs:10534-10548`), which filters `AttemptState::Complete` and so skips the Failed `review/1`.
- A second loss the issue does not name: `rerun --note` on a plan review is accepted and dropped (`rerun_note`, `scheduler.rs:8988-8997`). It is out of scope; see the end.

## The record

No change. `Attempt.artifacts["plan"]` of the failed `review/1` already holds the revised copy's path, and `Attempt.revisions` already holds the owner's round. `RECORD_VERSION` stays as it is in `dispatch/src/store.rs:26` (28 today); no bump. `SCHEMA_VERSION` in `src/core/model.rs:12` (16 today) is untouched.

Which copy a rerun reads is derived, not stored, by one function, `rerun_copy` (below). It walks `t.attempts` backwards from the attempt being started (or shown), and the first of these decides. A `Complete` attempt of any stage with the subject artifact, as `review_subject` filters (`scheduler.rs:10534-10548`), ends the walk with no failed copy, so the rerun copies from it as today. A `Failed` workflow attempt of the same stage and context with the subject artifact, whose file exists, is the failed copy. Everything else, `Cancelled` included, is skipped. Position in the vector orders attempts across stages (`n` is per stage and cannot); the vector is chronological, which `review_subject`'s `.rev()` already relies on. A failed `review/1` followed by a re-run `plan/2` therefore loses to `plan/2`. `show` calls the same function, so the two cannot drift.

## The signals

None new. `shown` and `input` keep their rules, so the existing tests `dispatch/src/ticket.rs:1797` and `1827-1875` (a Failed or Cancelled review copy is not *the plan* going forward) stand. The failed copy becomes the rerun's input only; it is not promoted to "the plan" for agents or for display between the failure and the rerun.

## The actions and effects

No new action or effect. Three edits in `dispatch/src/scheduler.rs`:

1. **One helper `rerun_copy(t, stage, ctx, subject, before: usize) -> Option<PathBuf>`** next to `review_subject` (10534). `before` is the index in `t.attempts` of the attempt being started or shown (`t.attempts.len()` when it is not pushed yet), so the new attempt can never find itself. It applies the walk described in "The record" over `t.attempts[..before]`. It does not use `latest_attempt` (9444), which looks at one stage only and, run after the attempt exists, would return that attempt. `Cancelled` is skipped on purpose: a parked or discarded review's copy is not the plan going forward (`ticket.rs:1856-1864`).
2. **`start_workflow` (7465-7472).** Keep `review_subject` for the session: `(source_path, source_session)`. Then `let source_path = rerun_copy(t, stage, ctx, subject, t.attempts.len()).unwrap_or(source_path)` and copy that. The session stays `plan/1`'s, because for a workflow attempt `poll_workflow` overwrites `Attempt.session` with the run's reviewer or planner face (7784-7788; Facts, Settled), and `WorkflowStart.source` must be the planning session. Attempt 2 failing in turn copies from `review/2`, which was copied from `review/1`, so revisions chain with no extra code.
3. **`rerun_carries` (9382-9390).** For a `Workflow` attempt that is `Failed` and has a copy `rerun_copy` accepts, return ` The next attempt reviews this attempt's copy, with its revisions: <path>.` Otherwise keep `""`. The path is the one `rerun_copy` would use, so the question and the action agree. `rerun_carries` takes `(t, stage, n)`; it finds the failed attempt with `find_attempt` as it does now, and calls `rerun_copy` with `before` set to `t.attempts.len()`, since the question is about the attempt that would start next.

Decisions resting on Unresolved lines (Facts, Unresolved):
- *Half-written copy after a killed run.* Default: copy the file as it is, with the same `std::fs::copy`. Safe whichever way the answer goes, and the question names the copy so the owner can inspect it. No new atomic write.
- *Planning session still resumable.* Default: pass `plan/1`'s session as `source`, as the first attempt does. If the app refuses, the existing `Reply::Failed` path (7522 onward) fails the attempt and asks `rerun`/`park`. No fallback session is added.
- *Re-sending the owner's objection to a fresh run.* The plan does not depend on it. The revised copy carries the revision, and the note stays on the answered decision. Re-sending is an enhancement for the owner to ask for, not built here.

Residual, stated in the design status: the new run's planner is cloned from `plan/1`'s conversation, so it has not seen the owner's objection; the file it reviews has.

## The UI

Presentation is `show` only; the client's rendering of `plan_reviewing`/`plan_round` was not read (Facts), so the client change is nil and the new field is additive.

- `dispatch-control/src/lib.rs:586` `LaneFile`: add `revised_from: Option<PathBuf>` (the type is `#[serde(default)]`, so older readers are unaffected).
- `dispatch/src/serve.rs:539-546` `ticket_paths`: for a `reviewing` plan file, set `revised_from` to the result of `rerun_copy(t, &a.stage, &a.context, "plan", index_of(a))`, where `index_of(a)` is the open attempt's position in `t.attempts`. It is the same function and signature `start_workflow` and `rerun_carries` use, not a second rule; only `before` differs.
- `dispatch/src/main.rs:896-908` `print_lane_files`: after `(reviewed copy…, review open)`, print `    started from the revised copy of a failed attempt: <path>`. The owner's `plan.feedback-<n>.md` sits beside that path.

The earlier attempt's rounds are not merged into `plan_rounds` (`serve.rs:564-572`): round numbers restart in each run and would collide in `PlanRoundView.n`. Follow-up if wanted.

## The tests

- `dispatch/tests/first_slice.rs`, one new test next to `3257`: `at_finalize`, `revise` with a note (as `3442`), overwrite `review/1`'s copy with marked text to stand in for the app's in-place edit, set `sb().runs.clear()`, `step` to the `review run gone` failure, assert the `rerun` question contains `review/1/joined/plan.md`, answer `rerun`. Assert `review/2/joined/plan.md` equals the marked text (the fake never reads the file, Facts), the recorded `WorkflowStart.plan` is that path, and `source` is `plan/1`'s session. Also assert the `finalize` answer's note and `plan.feedback-2.md` still exist.
- Same file: a failed first attempt with no revise reruns from its copy; a *Cancelled* review (parked, then restarted) still copies from `plan/1`.
- Unit test beside `review_subject`: `rerun_copy` returns the failed attempt's path, `None` for Cancelled, `None` for a missing file, and the newest failure when two have failed. Ordering across stages: a failed `review/1` then a complete `plan/2` gives `None` (the rerun copies from `plan/2`); a complete `plan/2` then a failed `review/2` gives the failed copy. With `before` set to the failed attempt's own index it is not found, so an open attempt never sees itself. I read the code only and did not run a restart to see whether `plan/2` can follow a failed review in a real ticket; the unit test carries the case either way.
- `serve.rs` test beside `1685`: `ticket_paths` sets `revised_from` for an open attempt 2 after a failed attempt 1, and leaves it `None` otherwise, including when a complete `plan/2` sits between them. `dispatch/tests/ui`-style checks for `print_lane_files` follow whatever `main.rs` already tests for it.
- Existing `first_slice.rs:3257` and `3817` and the `ticket.rs` tests must stay green unchanged.

## Docs and release

- `CHANGELOG.md`, under Unreleased: "A Dispatch plan review that is run again after a failed attempt starts from the failed attempt's copy, with the owner's revisions, and the rerun question and `show` say which copy."
- `README.md`: no new file, no new environment variable; no Layout change.
- `docs/design.md`: add a paragraph to the Dispatch plan-review text near line 2213 (a rerun after a failed attempt copies the failed attempt's copy; `Cancelled` copies are not used), and a line in the relevant status section naming the residual (the cloned planner has not seen the objection).

## For the owner (none blocks this plan)

- Does `review/1/joined/plan.md` on the reporter's machine hold the revision, and did the app lose the run (removed run, store reset, changed data directory)? Facts, For the owner: the plan builds the same either way.
- Follow-ups, not in this commit: `rerun --note` on a workflow stage is dropped; earlier attempts' owner rounds are not listed in `plan_rounds`; re-sending the objection to a fresh run.

## Commit

One commit, README and design status updated, the app runs throughout.

Message: `Dispatch reruns a failed plan review from the failed attempt's copy and says so`

Contract additions:
- `dispatch_control::LaneFile.revised_from: Option<PathBuf>` (additive, `#[serde(default)]`).
- `pub(crate) fn rerun_copy(t: &Ticket, stage: &str, ctx: &str, subject: &str, before: usize) -> Option<PathBuf>` in `dispatch/src/scheduler.rs`, the one rule, used by `start_workflow`, `rerun_carries` and `ticket_paths`.
- No change to `model.rs`, `AppAction`, `Effect`, ports, or the record; no `RECORD_VERSION` or `SCHEMA_VERSION` bump.
