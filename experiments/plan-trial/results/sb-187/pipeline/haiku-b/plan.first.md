# Issue #187: a plan review rerun starts from the copy the owner revised

## Where this departs from the issue

- **"The reviser wrote the revised plan to `review/1/joined/plan.md`."** Borne out, with one nuance. `revise` (`dispatch/src/scheduler.rs:7565`) writes only the owner's note, to `plan.feedback-1.md` beside the copy, and sends `workflow.object`. The planner edits the copy in place.
- **"Attempt 2 started from `plan/1/joined/plan.md`."** Borne out. `review_subject` (`scheduler.rs:10534`) takes only `AttemptState::Complete`, so the `Failed` review attempt is skipped. `start_workflow` copies from the plan attempt (`scheduler.rs:7465`–`7468`).
- **"Nothing warned; `show` names only attempt 2's copy."** Borne out. `Ticket::shown_where` (`dispatch/src/ticket.rs:1476`) drops a failed attempt, `ticket_paths` (`dispatch/src/serve.rs:538`) builds `plan_files` from it, and `print_lane_files` (`dispatch/src/main.rs:896`) prints only the copy it is given.
- **"Start from the newest plan the stage holds, or at least say which copy."** The code's rule is "newest complete", and `docs/design.md:2163`–`2165` says the same for agents: an agent's prompt is handed only complete files. The plan changes that for one case and amends the design. It does both things the issue asks: start from the revised copy, and name the copy in the rerun question and in `show`.
- **"The revise note should survive a runner failure the same way an answered decision does."** Partly. The note stays on disk (`plan.feedback-N.md`), in attempt 1's `revisions` (`ticket.rs:350`), and on the answered decision. It reaches the next attempt's copy only if the reviser applied it before the run died. The plan does not re-deliver it, because a fresh run has no converged round to object to. The rerun question and `show` say where the note is.
- **The earlier notes say the test at `ticket.rs:1858`–`1866` pins the failed case and must change.** It does not need to. The new rule keys on the failed attempt having owner `revisions`, and that test's failed attempt has none, so the test stands. The plan adds a case beside it.
- **"A runner restart mid-run does it."** Not checked. The plan does not depend on the cause of `no such run`.

**Decisions for the owner:**

1. Carry only copies with an owner `revise` (recommended, as below), or every failed review's copy? The broad rule also carries reviewer-round edits, which are not owner input and may come from a reviser that died mid-edit.
2. A parked and resumed review (`Cancelled`) still starts from the plan stage. The existing test comment says a parked copy is not the plan going forward, so this stays, but a resumed review loses its revision the same way. Say if that should change too.

## The record

No change. Attempts keep `artifacts`, `revisions` and `session`. The current version is `RECORD_VERSION = 28` (`dispatch/src/store.rs:26`). If implementation turns out to need a new field, bump from the value in `store.rs` at implementation time, plus one, and add the step in `migrate` (`store.rs:450`).

## Signals

None. The rerun question and the copy choice read records the runner already writes.

## Actions and effects

No new action or effect. The change is which copy `start_workflow` names in `Body::WorkflowStart { plan }` (`scheduler.rs:7510`):

1. `Ticket::review_copy(subject)`, beside `shown_where` in `ticket.rs`: the newest attempt that holds `subject` in `artifacts` and is either `Complete`, or `Failed` with `kind == Workflow` and non-empty `revisions`. Skip an attempt whose `forgotten` names `subject`.
2. `review_subject` (`scheduler.rs:10534`) takes the path from `review_copy`. The session still comes from the newest `Complete` attempt that wrote the subject, as today. So the new planner clones the plan stage's conversation, not the reviser's. This is the planner session choice noted above.
3. `rerun_carries` (`scheduler.rs:9382`) gets a `Workflow` arm. When `review_copy` names a `Workflow` attempt, it adds " The next attempt reviews the copy {stage} {n} left, with the owner's revision." The clause is left out when that attempt has no revisions. Otherwise it returns "" as today.

If the reviser died mid-edit, the copy holds whatever it had. The plan does not detect that. The rerun question names the copy so the owner can park instead.

## UI

The Plan tab reads `PathsView.plan`, which comes from `shown_where`. After the change it shows the revised copy under the existing "Reviewed copy" label (`src/ui/ticket.rs:427`, `plan_label`), so it matches the copy the next review starts from. No new label. The attempts list already says `review/1 … failed`.

`show` (`dispatch/src/main.rs:981`–`993`):

- `ticket_paths` (`serve.rs:538`): `plan_files` keeps the shown copy as now, including a failed revised one. Each other `Workflow` attempt with a `plan` artifact and non-empty `revisions` adds a `LaneFile` with `prior: true` and `round: review_round(a)`. Give it the lane `lane_files_by` gives that attempt.
- `print_lane_files` (`main.rs:896`): a `prior` entry prints "(prior revision, round N)". A shown copy that is a workflow copy and not open (`round` is `Some`, `reviewing` is false) prints "(reviewed copy, round N, review not open)".

## Tests

- `ticket.rs` tests, beside the loop at `:1858`: a `Failed` attempt with `revisions` makes `shown` and `review_copy` return the review's copy, while `input` still returns `plan/1`. A `Cancelled` attempt with `revisions` still returns `plan/1`. The existing loop is unchanged.
- `dispatch/tests/first_slice.rs`, beside `a_run_switchboard_no_longer_has_fails_the_attempt` (near `:3806`), new `a_rerun_after_a_revise_starts_from_the_revised_copy`: `at_finalize`, then `revise` with a note. The test writes "revised" into the `review/1` copy in the reviser's place, then `runs.clear()`, then `step`, which fails the attempt. Answer `rerun`, then `step`. Attempt 2's copy holds "revised", `WorkflowStart.plan` names it, its `source` is the plan session, and the rerun question contains "with the owner's revision". A second test without `revise` shows the copy is still the plan stage's.
- `serve.rs` tests, beside `:1455`–`:1497`: `ticket_paths` lists the revised failed copy as shown. With attempt 2 open, it also lists attempt 1 with `prior: true` and `round: 1`.
- `main.rs`: if it has a tests module, a unit test for the three `print_lane_files` labels. If not, the serve test covers the data.

Gate: `cargo test --locked --workspace`, `cargo clippy --locked --workspace --all-targets -- -D warnings`, `cargo fmt --all`, and `cargo clippy --locked -p dispatch --all-targets --target x86_64-unknown-linux-gnu -- -D warnings`. The Switchboard app builds unchanged because nothing in `src/` constructs `LaneFile`.

## Docs

- `docs/design.md`: a new dated section "Plan review rerun copy (2026-10-21)" before "## Open questions" (`:3672`), stating the rule, the note, and the planner-session choice. Amend `:1784`–`:1787` (a rerun "starts a fresh run" and now reviews the copy the new section names), and `:2163`–`:2165` (agent prompts get complete files, except the review copy the new section names).
- `README.md`: no line changes. No file or dev aid is added, and the Layout entries describe the files at the level they already do.
- `CHANGELOG.md`, under Unreleased: "- A plan review rerun after a failed attempt starts from the copy the owner revised, not the plan stage's original, and `show` and the rerun question name the copy the review reads."

## Commit message

`Dispatch: a plan review rerun starts from the owner's revised copy`

## Contract additions

- `dispatch_control::LaneFile.prior: bool` (serde default; existing literals take `..LaneFile::default()`).
