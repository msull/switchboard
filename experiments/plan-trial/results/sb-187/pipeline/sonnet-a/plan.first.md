# Plan: a review rerun starts from the owner's revised plan (issue #187)

## Where this departs from the issue

- The issue says the revision is discarded. On disk it is not: `review/1/joined/plan.md` and `plan.feedback-<round>.md` stay where they were written (`revise`, `dispatch/src/scheduler.rs:7565`, file at `:7616`). What is lost is the record's way to reach them. `review_subject` (`scheduler.rs:10534`) keeps only `AttemptState::Complete` attempts, and a review attempt completes only on `finalize`, after the revise. So the plan stage's output is picked, and `start_workflow` (`:7436`) copies it over the new attempt's `plan.md` (`:7465` to `:7471`).
- The issue says the `revise` note should survive a runner failure like an answered decision. It already does: the note is on the `finalize` `Decision` and in the round file. Nothing needs to be saved. The new attempt just has to start from the revised copy, which already holds the revision. The round files are not copied forward: the new attempt's rounds start at 1 and would overwrite `plan.feedback-1.md`. `show` lists them instead.
- The issue's "at least say which copy it will review" is not satisfied by the code: `rerun_carries` (`:9382`) says nothing for a plan review. `show` prints only review/2's copy because `Ticket::shown_where` (`ticket.rs:1476`) keeps Complete and open attempts, so the failed review/1 is invisible.
- Not every failed review attempt's copy is newer in a way that matters. A copy with no owner `revise` may hold only the planner's mid-run round edits, possibly half-written when the run died, and the rerun redoes those rounds anyway. The fix applies only to a failed or cancelled attempt whose `revisions` is non-empty.

## The record

No change. The selector reads `Attempt.artifacts`, `Attempt.revisions` (`ticket.rs:350`) and `Attempt.session`, which already exist. `RECORD_VERSION` in `dispatch/src/store.rs:26` is 28 today and is not bumped. If an implementer finds a serialized field is needed after all, the bump is the value in `store.rs` at implementation time, plus one, with a step in `migrate`; a ticket landing in parallel may have taken the next number first. `SCHEMA_VERSION` (16) and `VIEWS_SCHEMA_VERSION` (7) in `src/core/model.rs` belong to the app and are not touched.

The source of a rerun is not recorded on the new attempt. `show` therefore labels prior copies (below) but cannot say after the fact which one an attempt started from; the rerun question is where that is stated.

## The signals

None. No new poll, hook or port message. The revise round files already exist.

## The actions and effects

No new `Effect` or action. Two changes in `dispatch/src/scheduler.rs`:

1. Replace `review_subject(t, subject)` with `review_source(t, stage, ctx, subject, exists)`:
   - Walk `t.attempts` newest first. The first match wins:
     - a Complete attempt with the `subject` artifact (today's rule, `session` taken from it); or
     - a Failed or Cancelled `AttemptKind::Workflow` attempt with the same `stage` and `context`, a non-empty `revisions`, the `subject` artifact, and `exists(path)` true. It gives the path only.
   - The session always comes from the first Complete attempt with a session at or before that point in the walk. A review attempt's own `session` is the planner clone (`poll_workflow`, ~`:7790`), not the plan author, so it is never used for the source.
   - Return a small struct `ReviewSource { path, session, revised_from: Option<(String, u32)> }`. `exists` is a parameter so the selector is a pure function over the record; `start_workflow` passes `Path::is_file`. A revised copy whose file is gone falls through to the plan stage's output, and `revised_from` is `None`.
   - `start_workflow` uses `path` and `session`, and logs one line when `revised_from` is set, naming the attempt.
2. `rerun_carries(t, stage, n)` (callers `:7266` and `:9373`) returns `String` instead of `&'static str`. For a Workflow attempt that has a subject artifact (read the key from `a.artifacts`, so `p` is not needed), it calls `review_source` and appends one sentence:
   - "The next attempt reviews review/1's revised copy at <path>, which holds your revise note's changes." or
   - "The next attempt reviews the plan stage's output at <path>."
   - If an owner-revised copy exists but its file is gone, the second form adds "(review/1's revised copy at <path> is missing)".
   The code review and resolution review sentences are unchanged. `rerun_ask` (`:9335`) already calls it, so the question asked on a restart (`:8294`) and by `ask_rerun` (`:7315`) is consistent.

Contract additions are listed at the end.

## The UI

`show` and the port's `PathsView`, no egui (this is the Dispatch CLI).

- `LaneFile` (`dispatch-control/src/lib.rs:586`) gains `others: Vec<OtherCopy>`, `#[serde(default)]` through the struct's existing `#[serde(default)]`. `OtherCopy { stage, n, path, revised }` is a prior attempt's `plan.md` that is not the shown one. `revised` is true when that attempt has `revisions`.
- `ticket_paths` (`dispatch/src/serve.rs:538`) fills `others` in the `plan_files` map from `t.attempts`: attempts in any state with the `plan` artifact that is not the shown path, not `forgotten`, in the same lane or `root`/`joined` context as `lane_files_by` (`scheduler.rs:10908`) uses. Oldest first.
- `print_lane_files` (`dispatch/src/main.rs:896`) prints one line per other copy under the file, after the existing "(reviewed copy ...)" line: `(prior copy, plan/1: <path>)` or `(prior revision, review/1, owner revised: <path>)`. The notes lines are untouched.

## The tests

- `dispatch/tests/first_slice.rs`, near `a_run_switchboard_no_longer_has_fails_the_attempt` (`:3809`), using `at_finalize`, `env.sb().runs.clear()` and the fake in `dispatch/tests/support/mod.rs`:
  - `a_review_rerun_after_a_revise_starts_from_the_revised_copy`: plan, review, `revise` with a note, then `runs.clear()` so the attempt fails, answer `rerun`. Assert the `WorkflowStart.plan` file of attempt 2 has the revised bytes and `source` is the plan author's session, not the planner clone.
  - `a_review_rerun_without_a_revise_starts_from_the_plan_output`: same without `revise`; the plan stage's bytes.
  - `a_review_rerun_falls_back_when_the_revised_copy_is_gone`: delete the copy first; plan output used, question says the copy is missing.
  - The pending `rerun` question names the revised copy's path.
- Unit tests next to `lane_plans` in `scheduler.rs` for `review_source` with an injected `exists`: revised failed attempt wins; revised but older than a newer Complete plan attempt loses; other stage or context ignored; Cancelled counts; no revisions ignored.
- `serve.rs` tests near `:1469` and `:1540`: `plan_files` carries `others` with `revised` set. A `main.rs` test (or the existing print tests) for the two label forms.
- Run `cargo test --locked -p dispatch` before and after. Existing tests that assert the review reads the plan stage's output after a failure need checking.
- Also `cargo clippy --locked -p dispatch --all-targets -- -D warnings`, and the x86_64-unknown-linux-gnu check from CLAUDE.md.

## Docs, in the same commit

- `CHANGELOG.md`: a line under Unreleased: a review rerun after a failed attempt starts from the owner's revised plan, the rerun question names the copy it will review, and `show` lists prior copies.
- `docs/design.md`: "Plan review workflow" (`:1000`) gets the selection rule; the note at `:1787` is unchanged except that a failed run no longer discards a revise. The status section for the Dispatch plan review lists the known gap that the source of a rerun is not recorded.
- `docs/dispatch.md`: the `no such run` paragraph (`:2497`) and the failure table row at `:3078` (add the new test names); the `show` files block.
- `README.md` Layout table: no new file, so no line. The `dispatch/` entry's `serve.rs` line (`:456`) mentions the plan views and needs no change.

## Commit message

Dispatch: a review rerun after a failed attempt starts from the owner's revised plan and says which copy it reviews

## Contract additions

- `dispatch_control::OtherCopy { stage: String, n: u32, path: PathBuf, revised: bool }` (new, `Serialize`/`Deserialize`, `Default`).
- `dispatch_control::LaneFile.others: Vec<OtherCopy>` (new field, defaulted when absent).
- `scheduler::review_source` and `ReviewSource` replace the private `review_subject`; `rerun_carries` returns `String`. Both are crate-private.
- No `AppAction`, `Effect`, `model.rs` or record field changes; no schema bump.
