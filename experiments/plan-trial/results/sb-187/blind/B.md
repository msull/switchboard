# A plan review rerun reviews the owner's revision (#187)

## Where this departs from the issue

- Repro step 2 says a runner restart in the middle of a run makes the attempt fail with `review run gone`. The code does not support that. A workflow run is a durable record in Switchboard (`docs/design.md:1002`). `poll_workflow` fails the attempt only when the app answers `no such run` for the run id (`dispatch/src/scheduler.rs:7755`), and the app answers that only when it has no record of the run (`src/app.rs:3915-3918`), for example after `remove_workflow` (`src/core/workflow.rs:1009`). The tests use what `a_run_switchboard_no_longer_has_fails_the_attempt` already uses (`dispatch/tests/first_slice.rs:3809`): `env.sb().runs.clear()`. The fix does not depend on why the run went away.
- "The `revise` note should survive a runner failure the same way an answered decision does." The note does survive. It is on the answered `finalize` decision, and it is in `review/1/joined/plan.feedback-<n>.md` (`scheduler.rs:7615-7618`). What gets lost is the plan copy the planner revised in response to it. The plan carries that copy forward. It does not replay the note into the new run. A note the planner never answered is named in the rerun and finalize questions, so the owner can send it again with `revise`. Replaying it automatically would open a paid round that the owner did not ask for in this run.

Everything else in the issue matches the code. `start_workflow` (`scheduler.rs:7436`) takes its input from `review_subject` (`scheduler.rs:10534-10548`), which skips every attempt that is not `Complete`. So a failed review attempt's copy is never a candidate, and the copy is taken from `plan/1`. `rerun_carries` (`scheduler.rs:9382-9390`) adds nothing to the question for a workflow attempt. `show`'s `files:` block prints only `t.shown("plan")`, which is the open attempt-2 copy (`dispatch/src/serve.rs:538-548`, `dispatch/src/main.rs:896-909`).

## The record

No schema change. `RECORD_VERSION` stays at its current value, 28 (`dispatch/src/store.rs:26`), and `USAGE_VERSION` stays 1. If the implementer finds they do need a serialized field after all, they bump from the value in `store.rs` at implementation time plus one and add a `migrate` step.

The existing `Attempt::carried_from: Option<(String, u32)>` (`dispatch/src/ticket.rs:312-317`) is now also set on a workflow attempt. There it means "this attempt's copy started as attempt `m`'s copy, not as the stage's input". Every current reader is limited to code review attempts, so this new meaning does not reach any of them:
- `previous_review` filters `AttemptKind::Review` (`dispatch/src/review.rs:3618-3625`);
- `carry_of` is called only for review attempts (`review.rs:3646`);
- `report::code_review` filters `AttemptKind::Review` (`dispatch/src/report.rs:413`).

Records written before this change have `None` and read as they do today. Widen the field's doc comment to cover both meanings.

## The carry rule

There is one new pure function in `scheduler.rs`, next to `review_subject`:

```rust
/// The copy a new attempt of a plan review starts from instead of the stage's input:
/// `prev`'s, the latest earlier attempt in the context, when the owner revised it or
/// it carried such a copy itself, nothing has rewritten the input since it started,
/// a restart did not discard it, and its copy is still on disk.
fn carried_copy<'t>(t: &'t Ticket, prev: &'t Attempt, subject: &str) -> Option<&'t PathBuf>
```

It returns `Some` only when all of the following hold:
- `prev.kind == AttemptKind::Workflow`.
- `!prev.revisions.is_empty() || prev.carried_from.is_some()`. Carrying the copy forward through `carried_from` makes a chain of failures keep the revision.
- `prev.state` is not `Cancelled { reason }` with `reason.starts_with(DISCARDED_BY)`. `DISCARDED_BY` is already imported at `scheduler.rs:25`.
- The input attempt `review_subject` finds ended at or before `prev.started_ms`. A restart to the plan stage writes a newer input, so that input is reviewed afresh.
- `!prev.forgotten.contains_key(subject)` and `prev.artifacts[subject]` exists on disk.

The plan only carries a failed attempt's copy when the owner revised it. A copy changed only by the planner, in an attempt that failed with no revision, is not carried, because it may be a half-finished edit. That rerun reviews the input as it does today, and its question says so.

A second helper:

```rust
/// The owner's round file in `src` whose round has no response beside the copy:
/// its note never reached the plan.
fn unanswered_revision(src: &Attempt, subject: &str) -> Option<PathBuf>
```

It takes `src`'s last `Revision`. When `report::response_file(copy, round)` (`report.rs:201`) does not exist, it returns `report::round_file(copy, round)` (`report.rs:156`).

## The signals

No new event or decision. Two existing questions gain a clause, and `show` gains a line.

- **The rerun question.** `rerun_carries` (`scheduler.rs:9382`) returns a `String` instead of `&'static str`. Its only caller is `rerun_ask` at `scheduler.rs:9373`. For a workflow attempt `a` it reads the subject name from `a.artifacts`' single key and uses `carried_copy(t, a, subject)` and `review_subject`:
  - When a copy is carried: " The next attempt reviews `<a's copy>`, this attempt's copy with the owner's revision, not `<input>`." If there is an unanswered revision, it adds: " The owner's note in `<round file>` has no response; answer `revise` with it again at finalize if it still stands."
  - Otherwise: " The next attempt reviews `<input>` afresh."
- **The finalize question.** `review_done` (`scheduler.rs:7693-7696`): when `a.carried_from` names attempt `m`, it appends " It started from `<m's copy>`, an earlier attempt's revised copy." and the same unanswered-note sentence, computed on attempt `m`.

## The actions and effects

- `review_subject` returns the input attempt's `ended_ms` as well: `(PathBuf, String, Option<u64>)`. It has one caller, at `scheduler.rs:7465`.
- In `start_workflow`, before the push at `scheduler.rs:7487`, take `prev = latest_attempt(t, &stage.name, ctx)`. If `carried_copy` returns a path, copy that file instead of `source_path` at `scheduler.rs:7471`, and set `carried_from = Some((stage.name.clone(), prev.n))` on the new attempt. `new_attempt` stays as it is; set the field on the returned value, as `review.rs:225` does. `source_session` is still the plan attempt's session, because the planner clone's history has to be the plan author's. The workflow's prompts already name the copy (`docs/dispatch.md:550-557`).
- Log the carry with `log::info!`, naming both attempts.
- There is no new `Effect` or port call. The copy is a local file copy, as it is today.

## The UI

- `dispatch-control/src/lib.rs:586` `LaneFile` gains `pub started_from: Option<PathBuf>`: the earlier attempt's copy this copy started from. The struct is `#[serde(default)]`, which makes the field additive: a reply from an older runner simply reads `None`.
- `serve.rs:541-547` fills it from `a.carried_from` through `find_attempt` and that attempt's `artifacts[&a.stage's subject]`. The subject is the attempt's single artifact key.
- The test literal at `serve.rs:1540` names every field. Add `started_from: None` there.
- `print_lane_files` (`main.rs:896`) prints `    (started from <path>, an earlier attempt's revised copy)` under the file when `started_from` is set. This line prints whether or not the review is still open.
- The app's Dispatch page does not draw `plan_files` (nothing under `src/` reads it), so the app does not change.

## The tests

In `dispatch/tests/first_slice.rs`, next to `a_run_switchboard_no_longer_has_fails_the_attempt`, using `at_finalize`, `revise` and `artifact_of`:

- `a_rerun_after_a_revised_review_fails_reviews_the_revised_copy`:
  1. `revise` with a note, then `step`.
  2. Write the revised plan text into the attempt-1 copy and write `response_file(copy, 2)`.
  3. `env.sb().runs.clear()`, then `step` until the `rerun` decision is pending. Assert its question names the attempt-1 copy and "the owner's revision" and has no "no response".
  4. Answer `rerun` and step until `n == 2` has a run. Assert the attempt-2 copy's text equals the revised text, `carried_from == Some(("review", 1))`, and `env.sb().runs.last().plan` is the attempt-2 copy.
  5. Converge run 2. Assert the finalize question says "started from" with the attempt-1 path.
  6. Assert `ticket_paths(...).plan_files[0].started_from` is the attempt-1 copy.
- `a_rerun_after_an_unanswered_revision_names_the_owners_note`: the same steps without the response file. Both the rerun question and attempt 2's finalize question name `round_file(attempt-1 copy, 2)`. The copy is still carried, since it is the converged plan.
- Extend `a_failed_review_run_fails_the_attempt_and_a_rerun_starts_a_new_run` (`first_slice.rs:3257`). Its rerun question says "afresh" and names the plan attempt's file, attempt 2 has `carried_from == None`, and the attempt-2 copy reads `# plan`.

Unit tests in the `scheduler.rs` test module (`scheduler.rs:11338`), on a hand-built `Ticket`. `carried_copy` returns `None` when:
- an input attempt ended after `prev` started;
- `prev` is cancelled with a `DISCARDED_BY` reason;
- `prev` has no revisions and no `carried_from`;
- `prev`'s copy file is missing.

It returns `Some` for an unrevised `prev` whose `carried_from` is set. `unanswered_revision` returns `None` once the response file exists.

## Docs

- `docs/dispatch.md`:
  - The "A review is a copy" bullet (line 550): a rerun after a failed attempt that the owner revised copies that attempt's copy, and the rerun and finalize questions say so.
  - Design choice 6 (line 3171): a rerun keeps the owner's revision, and an unanswered note is named rather than replayed.
  - A new row in the first-slice table after the `revise --note` row (line 2975), naming the three integration tests.
- `docs/design.md` has no status item for the plan review stage. Dispatch's built state is the first-slice table in `docs/dispatch.md`, which the row above updates.
- README Layout, `dispatch/src/scheduler.rs` line (459): add "a plan review's rerun after the owner's revision reviews the revised copy". Same table, `dispatch-control/` line (448): no change. The dev-aid list and the data directory section do not change.
- `CHANGELOG.md`, under Unreleased, `### Fixed`: "A Dispatch plan review rerun after a failed attempt reviews the owner's revised copy instead of the plan stage's original, and the rerun question and `show` say which copy it reviews."

## Commit

`A plan review rerun after a failed attempt reviews the owner's revised copy (#187)`

Contract additions:
- `dispatch_control::LaneFile::started_from: Option<PathBuf>` (serde default).
- `dispatch::ticket::Attempt::carried_from` is now also set on a workflow attempt, meaning the attempt whose copy it started from. There is no new field and no schema bump.
