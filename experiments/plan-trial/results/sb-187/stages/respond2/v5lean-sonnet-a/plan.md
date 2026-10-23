# Plan: #187, a plan-review rerun starts from the newest copy the stage holds

Checked against the worktree at HEAD `7fe8949`. "Facts" is `facts.md` for this issue.

## Where this departs from the issue

- **"The `revise` note … should survive a runner failure the same way an answered decision does."** It already does, so the plan changes nothing for the note. `revise` writes `plan.feedback-<n>.md` beside the reviewed copy (`dispatch/src/scheduler.rs:7616-7618`) and pushes a `Revision` on the attempt (7621). What is lost is the *revised copy as the rerun's input*, not the note.
- **"Nothing warned … `show` … names only the attempt-2 copy."** True of the rerun question (`rerun_carries` returns `""` for a workflow stage, `scheduler.rs:9382-9390`). The plan fixes the question; the `files:` block of `show` is left alone.
- **Repro step 2, "a runner restart mid-run" gives `review run gone`.** The tree does not bear this out (`remove_workflow`, `src/core/workflow.rs:1009-1028`, is the only drop; Facts, Settled). The plan fixes any failed review attempt, and the test models the failure with `runs.clear()` (`dispatch/tests/support/mod.rs:690`).

## The record

No change, no bump. `Attempt.artifacts["plan"]` of the failed `review/1` already holds the revised copy's path, and `Attempt.revisions` the owner's round.

## The actions and effects

No new action or effect. Three edits in `dispatch/src/scheduler.rs`:

1. **One helper `rerun_copy(t, stage, ctx, subject) -> Option<PathBuf>`** next to `review_subject` (10534). It walks `t.attempts` backwards (the vector is chronological, which `review_subject`'s `.rev()` already relies on; `n` is per stage and cannot order across stages). The first attempt that matches decides: a `Complete` attempt of any stage with the subject artifact, as `review_subject` filters (10534-10548), returns `None` and the rerun copies from it as today; a `Failed` workflow attempt of the same stage and context with the subject artifact, whose file exists, returns that path. Everything else, `Cancelled` included, is skipped: a parked or discarded review's copy is not the plan going forward (`ticket.rs:1856-1864`). A failed `review/1` followed by a re-run `plan/2` therefore loses to `plan/2`.
2. **`start_workflow` (7465-7472).** Keep `review_subject` for the session: `(source_path, source_session)`. Then `let source_path = rerun_copy(t, stage, ctx, subject).unwrap_or(source_path)` and copy that. The session stays `plan/1`'s, because `poll_workflow` overwrites `Attempt.session` of a workflow attempt with the run's face (7784-7788; Facts, Settled), and `WorkflowStart.source` must be the planning session.
3. **`rerun_carries` (9382-9390).** For a `Workflow` attempt that is `Failed` and has a copy `rerun_copy` accepts, return ` The next attempt reviews this attempt's copy, with its revisions: <path>.` Otherwise keep `""`.

Defaults on the Unresolved lines are what the code does today: the copy is taken as it is with `std::fs::copy`, and `plan/1`'s session is passed as `source` (a refusal fails the attempt through the existing `Reply::Failed` path, 7522 onward). Re-sending the owner's objection to the new run is not built.

## The tests

- `dispatch/tests/first_slice.rs`, one new test next to `3257`: `at_finalize`, `revise` with a note (as `3442`), overwrite `review/1`'s copy with marked text to stand in for the app's in-place edit, `sb().runs.clear()`, `step` to the `review run gone` failure, assert the `rerun` question contains `review/1/joined/plan.md`, answer `rerun`. Assert `review/2/joined/plan.md` equals the marked text (the fake never reads the file, Facts), the recorded `WorkflowStart.plan` is that path, and `source` is `plan/1`'s session.
- Same file: a *Cancelled* review (parked, then restarted) still copies from `plan/1`.
- Unit test beside `review_subject`: a failed `review/1` then a complete `plan/2` gives `None`; a complete `plan/2` then a failed `review/2` gives the failed copy; a Cancelled attempt gives `None`.

## Docs and release

- `CHANGELOG.md`, under Unreleased: "A Dispatch plan review that is run again after a failed attempt starts from the failed attempt's copy, with the owner's revisions, and the rerun question says which copy."
- `docs/design.md`: one paragraph in the Dispatch plan-review text near line 2213 — a rerun after a failed attempt copies the failed attempt's copy, `Cancelled` copies are not used, and the cloned planner has not seen the owner's objection (the file it reviews has) — plus a sentence in that paragraph for the status section.

## Commit

Message: `Dispatch reruns a failed plan review from the failed attempt's copy and says so`

Contract addition: `pub(crate) fn rerun_copy(t: &Ticket, stage: &str, ctx: &str, subject: &str) -> Option<PathBuf>` in `dispatch/src/scheduler.rs`.
