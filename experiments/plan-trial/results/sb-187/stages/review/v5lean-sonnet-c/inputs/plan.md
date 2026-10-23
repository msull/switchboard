# Plan: issue #187, a review rerun starts from the newest plan the stage holds

## Where this departs from the issue

- The issue says "kill the review run … a runner restart mid-run does it". The tree does not bear that out as the cause. `no such run` is the app's answer when the run is absent from `workspaces[].workflows` (`src/core/control.rs:865`, `src/core/workflow.rs:217-224`). A runner restart does not edit workflow attempts (`dispatch/src/restart.rs:883` only clears `failed_at_checks`; `recover.rs` never mentions `Workflow`). The plan does not depend on why the run vanished (facts, Unresolved: "Why the app answered `no such run`"). The fix reads the failed attempt's record, which `fail_attempt_with` leaves intact (`scheduler.rs:7195-7230`, facts Settled).
- "The `revise` note … should survive a runner failure the same way an answered decision does." It already does: the note is the round file beside the reviewed copy and a `Revision` on the attempt, and a failed attempt keeps both. What is lost is only the *pointer* to that copy. Once the rerun reviews the revised copy, the round file beside it travels with the file on disk. No new storage.
- The issue says `show` "names only the attempt-2 copy". True after the rerun. Before the rerun answer it is worse: `shown_where` (`ticket.rs:1476-1491`) skips the failed review, so `show` names `plan/1/joined/plan.md` and the revised copy is not listed.

Everything else in the issue is as the code does it: `review_subject` keeps only `Complete` attempts (`scheduler.rs:10539`) and `start_workflow` copies that file (`scheduler.rs:7465-7472`).

## The record

Nothing. No field is added to `Ticket`, `Attempt` or `Revision`, so `RECORD_VERSION` (`dispatch/src/store.rs:26`) and `SCHEMA_VERSION` (`src/core/model.rs:12`) are not touched and no migration step is written. The failed attempt's `artifacts` already name the revised copy (facts, Settled: `fail_attempt_with` leaves `artifacts` and `revisions` alone).

## The signals

Nothing new. No new hook event, tmux signal or wire message. `WorkflowStart` already takes any absolute `plan` path; the only check is `is_absolute` (`src/core/workflow.rs:289-325`, `src/core/control.rs:1384-1394`; facts Settled), so the app side needs no change. `source` stays the plan attempt's Claude Code session, which must have a transcript. That is today's behaviour. (Claude Code removes transcripts older than `cleanupPeriodDays`, default 30; this matters only for a very old rerun and is unchanged by this plan.)

## The actions and effects

No new `AppAction` or `Effect`. Three changes in `dispatch/`:

1. **`review_subject` (`scheduler.rs:10534-10548`) picks the copy and the session separately.**
   - The *path* is the file of the newest attempt that wrote `subject` and is either `Complete`, or a `Workflow` attempt in `Failed` or `Cancelled` whose file exists (`Path::is_file`; if it does not, skip that attempt and fall through, so a deleted copy degrades to today's behaviour instead of making `fs::copy` fail).
   - A failed review is accepted whether or not it has `revisions`. If only revised attempts counted, a chain breaks: review/2 fails after starting from review/1's revised copy, its own `revisions` is empty, and review/3 would fall back to `plan/1`. A failed attempt's copy is either the bytes it started from or a revision of them, so newest wins.
   - The *session* stays what it is today: the newest `Complete` attempt that wrote `subject` and has a session. It must not come from the failed review, whose `session` is the planner clone's face (`scheduler.rs:7786-7788`), not a source for a new clone.
   - Return a small struct `{ path, session, from: (stage, n), revised: bool }` instead of the tuple (`revised` = the attempt has `revisions`, or its `n` differs from the attempt that is the source of the session). The one caller is `start_workflow` (`scheduler.rs:7465`); it keeps copying into the new attempt's directory, so the new attempt still owns its file and the failed one stays as the record.
   - `Ticket::input_where` (`ticket.rs:1452-1463`) is unchanged. It feeds downstream stages, which must read only a completed attempt's output; the test at `ticket.rs:1797-1824` keeps pinning that.
2. **The rerun question names the copy.** `rerun_carries` (`scheduler.rs:9382-9390`) returns `&'static str` and gives a `Workflow` attempt nothing. Change it to return `String` and, for a `Workflow` attempt, append ` The next attempt reviews <path>` plus ` (attempt <stage>/<n>'s copy, revised after your note)` when the chosen file comes from an attempt with `revisions`, else ` (the plan stage's output)`. The subject is the single key of the attempt's `artifacts`. Both callers (`scheduler.rs:7266`, `9373`) change type only. A question already asked keeps its old text; that is fine, the picking logic is what matters.
3. **`Ticket::shown_where` (`ticket.rs:1476-1491`) also accepts a `Failed` or `Cancelled` `Workflow` attempt**, the same predicate as above, so `show`, the ticket page and `shown_lane_files` name the copy the rerun will review. Put the predicate on `Attempt` once (`holds_review_copy`) and use it in both places. `lane_files_by` already narrows to `Complete` for the input case (`scheduler.rs:10917-10921`), so downstream readers are unaffected.

## The UI

- **`show`**: add two additive fields to `dispatch_control::LaneFile` (`dispatch-control/src/lib.rs:586`, `#[serde(default)]` so an older runner and an older client both still read it): `review_failed: bool` and `earlier: Vec<PathBuf>` (the other `plan.md` copies from earlier attempts of the same lane, oldest first, newest-written excluded). `ticket_paths` (`serve.rs:539-548`) fills them. `print_lane_files` (`main.rs:896-909`) prints, under the path, `(reviewed copy, review failed; a rerun reviews this copy)` when `review_failed`, and `(earlier copies: …)` when `earlier` is non-empty, so with more than one `plan.md` it is clear which is reviewed and which are prior revisions.
- **Ticket page**: no change in `src/ui/`. `plan_label` (`src/ui/ticket.rs:425-433`) already reads a non-open reviewed copy as "Reviewed copy", which is right for a failed review. `PathsView` is untouched.

## The tests

All in the existing files; none sleeps.

- `dispatch/src/scheduler.rs` unit tests beside `review_subject`: (a) plan/1 `Complete` plus review/1 `Failed` with a copy on disk → path is review/1's, session is plan/1's; (b) review/1 failed with the file missing → falls back to plan/1's path; (c) review/1 failed, review/2 failed, none `Complete` after plan/1 → review/2's copy; (d) review/1 `Complete` newer than plan/1 → unchanged from today; (e) a `Starting` or `Running` review is never chosen.
- `dispatch/tests/first_slice.rs`, beside `a_run_switchboard_no_longer_has_fails_the_attempt` (line 3809): drive plan, review, answer `finalize` with `revise --note`, clear `env.sb().runs` so the attempt fails, answer `rerun`. Assert the `WorkflowStart` sent for attempt 2 carries a `plan` under `review-plan/2/` whose bytes equal the revised `review-plan/1` file (write distinct bytes into review/1's copy where the fake planner would), that `source` is the plan attempt's session, and that the `rerun` question text names the review/1 copy. A second test chains a second failure and asserts attempt 3 starts from attempt 2's copy.
- `dispatch/src/ticket.rs` tests: `shown("plan")` returns a failed workflow attempt's copy; `input("plan")` still returns the completed attempt's (extend the test at line 1840 area).
- `dispatch/src/serve.rs` tests beside line 1455-1500: `ticket_paths` after a failed review sets `review_failed`, and with two copies fills `earlier`. A `dispatch_control` round-trip test (near `lib.rs:1011`) that a `LaneFile` JSON without the new fields still deserializes.
- `dispatch/src/main.rs`: the `print_lane_files` lines for the two new notes, if `main.rs` has a printing test to extend; otherwise assert on the `LaneFile` values only.

## Docs, one commit

- `docs/dispatch.md`: the rerun rules near 318-350 get one sentence on a review rerun's input; the `revise --note` row at 2975 gets "a failed review attempt's rerun starts from its copy" and the new test names.
- `docs/design.md`: the Dispatch/plan review status section's known-gaps list loses the "rerun restarts from the plan stage's output" gap if it is listed; otherwise one status line.
- `README.md`: the Layout table needs no row (no new file). Nothing in the dev-aid list changes.
- `CHANGELOG.md`: under Unreleased, "Dispatch: a plan review rerun after a failed attempt starts from the newest reviewed copy, so an owner's `revise` survives, and the rerun question and `show` name the copy."
- The app runs at every step; no `src/` change.

## Follow-ups for the owner (not in this plan)

- Why the app answered `no such run` (facts, Unresolved) needs a live reproduction on the owner's data. Whether anything sent `workflow.remove` or `Clean up` does not change the fix.
- The owner's ticket should be checked for `review/1/joined/plan.md` holding the revision and `review/2/joined/plan.md` being byte-identical to `plan/1`'s; if not, the cause is elsewhere and this fix still covers the failed-attempt case.
- Codex `resume` on a missing rollout (design.md:1173-1174) stays unverified; this plan resumes no reviewer.

## Commit message

Dispatch: a plan review rerun starts from the newest reviewed copy, not the plan stage's output, and the rerun question and `show` name it

## Contract additions

- `dispatch_control::LaneFile.review_failed: bool` (additive, `#[serde(default)]`).
- `dispatch_control::LaneFile.earlier: Vec<PathBuf>` (additive, `#[serde(default)]`).
- `Attempt::holds_review_copy` (dispatch crate, internal helper).
- No `AppAction`, `Effect`, port, `model.rs` or record change; no schema bump.
