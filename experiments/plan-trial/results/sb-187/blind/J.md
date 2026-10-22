# Issue #187: a plan review rerun starts from the copy the owner revised

## Where this departs from the issue

- **"The reviser wrote the revised plan to `review/1/joined/plan.md`."** Borne out, with one nuance. `revise` (`dispatch/src/scheduler.rs:7565`) writes only the owner's note, to `plan.feedback-1.md` beside the copy, and sends `workflow.object`. The planner edits the copy in place.
- **"Attempt 2 started from `plan/1/joined/plan.md`."** Borne out. `review_subject` (`scheduler.rs:10534`) takes only `AttemptState::Complete`, so the `Failed` review attempt is skipped. `start_workflow` copies from the plan attempt (`scheduler.rs:7465`–`7468`).
- **"Nothing warned; `show` names only attempt 2's copy."** Borne out. `Ticket::shown_where` (`dispatch/src/ticket.rs:1476`) drops a failed attempt, `ticket_paths` (`dispatch/src/serve.rs:538`) builds `plan_files` from it, and `print_lane_files` (`dispatch/src/main.rs:896`) prints only the copy it is given.
- **"Start from the newest plan the stage holds, or at least say which copy."** The code's rule is "newest complete", and `docs/design.md:2163`–`2165` says the same for agents: an agent's prompt is handed only complete files. The plan changes that for one case and amends the design. It does both things the issue asks: start from the revised copy, and name the copy in the rerun question and in `show`.
- **"The revise note should survive a runner failure the same way an answered decision does."** Partly. The note stays on disk (`plan.feedback-N.md`), in attempt 1's `revisions` (`ticket.rs:350`), and on the answered decision. It reaches the next attempt's copy only if the reviser applied it before the run died. The plan does not re-deliver it, because a fresh run has no converged round to object to. The rerun question names the feedback file so the owner can check it.
- **The earlier notes say the test at `ticket.rs:1858`–`1866` pins the failed case and must change.** It does not need to. The new rule keys on the failed attempt having owner `revisions`, and that test's failed attempt has none, so the test stands. The plan adds cases beside it.
- **"A runner restart mid-run does it."** Not checked. The plan does not depend on the cause of `no such run`.

**Decisions for the owner:**

1. Carry only copies with an owner `revise` (recommended, as below), or every failed review's copy? The broad rule also carries reviewer-round edits, which are not owner input and may come from a reviser that died mid-edit.
2. A parked and resumed review (`Cancelled`) still starts from the plan stage. The existing test comment says a parked copy is not the plan going forward, so this stays, but a resumed review loses its revision the same way. Say if that should change too.
3. A parked ticket's failed, revised copy is not the plan either: parking cancels only open attempts (`finish_parking`, `scheduler.rs:2654`), so without a rule the failed copy would stay the plan on a parked ticket. The rule below excludes parked tickets. Once the owner resumes and answers `rerun`, the ticket is no longer parked and the copy counts again.

## The record

No change. Attempts keep `artifacts`, `revisions` and `session`. `PathsView` and `LaneFile` are never stored in a record (no reference in `ticket.rs` or `store.rs`): they are serialized over the port (`src/ports/dispatch.rs:11`) and `show --json`. So `RECORD_VERSION` stays at its value in `dispatch/src/store.rs:26`, 28 today. If implementation turns out to need a stored field, bump from the value in `store.rs` at implementation time, plus one, and add the step in `migrate` (`store.rs:450`).

## Signals

None. The rerun question and the copy choice read records the runner already writes.

## Actions and effects

No new action or effect. The change is which copy `start_workflow` names in `Body::WorkflowStart { plan }` (`scheduler.rs:7510`), and what the rerun question says about it.

1. **`Ticket::carries_revision(&self, a: &Attempt) -> bool`** in `ticket.rs`, beside `shown_where`. True when `a` is a `Workflow` attempt in state `Failed` with non-empty `revisions`, and `self.state` is neither `Parking` nor `Parked`. This is the one predicate for "a failed plan review's copy the owner revised, which the plan is now". A `Complete` attempt is already covered by the existing rules.
2. **`shown_where`** (`ticket.rs:1476`) filter becomes `Complete`, or an open `Workflow` attempt, or `carries_revision`. `review_copy` and `shown_where` share that predicate, so the plan a reader sees is the plan the next review starts from. The doc comment (`ticket.rs:1470`–`1475`) is updated to say a finished review's copy and a failed revised one are shown, while an open review's copy is shown as it stands.
3. **`Ticket::review_copy(&self, subject: &str) -> Option<(&Attempt, &PathBuf)>`**: the newest attempt that holds `subject` in `artifacts`, is not `forgotten` for it, and is `Complete` or `carries_revision`. Open attempts are excluded: a rerun never starts while another review is open.
4. **`review_subject`** (`scheduler.rs:10534`) takes the path from `review_copy`. The session still comes from the newest `Complete` attempt that wrote the subject, as today, so the new planner clones the plan stage's conversation, not the reviser's. This is the planner session choice noted above.
5. **The rerun question** (`rerun_ask`, `scheduler.rs:9335`) gets one new clause, computed by a new function `revised_copy_carries(t, a)` and appended after `rerun_carries`. `rerun_carries` (`scheduler.rs:9382`) is unchanged. The clause names the attempt `review_copy` returns, not the failed attempt `a`, so the example of `review/1` revised and `review/2` failed with no revision names `review/1`. The subject is the single key of `a.artifacts`: a plan review attempt holds exactly its subject, as `start_workflow` writes it (`scheduler.rs:7493`). The clause is empty unless `review_copy` returns an attempt that is not `Complete`. Otherwise it reads: " The next attempt reviews {copy path}, which {stage} {n} was revised in (owner round {round}; the note is in {feedback path})." `round` is the last of that attempt's `revisions`, and the feedback path is `report::round_file(copy, round)`. The clause states what is on disk and makes no claim that the planner applied it.

If the reviser died mid-edit, the copy holds whatever it had. The plan does not detect that. The rerun question names the copy and the note so the owner can park instead.

## UI

**Plan tab.** `PathsView.plan` comes from `shown_where`, so after the change the tab shows the failed revised copy. `plan_label` (`src/ui/ticket.rs:427`) reads `plan_reviewed` and `!plan_reviewing`, so the label is "Reviewed copy", with no round. No new label and no change to the UI code.

**`show`** (`dispatch/src/main.rs:981`–`993`).

- Two additive fields on `dispatch_control::LaneFile` (`dispatch-control/src/lib.rs:586`), both serde default false:
  - `prior: bool`: a superseded plan review's copy that has owner revisions and is not the shown copy.
  - `failed: bool`: the shown copy is a failed plan review's copy.
- `ticket_paths` (`serve.rs:538`): the `plan_files` literal (`serve.rs:541`–`548`) gains `prior: false` and `failed: a.state` is `Failed`, since it has no `..LaneFile::default()`. Then the prior entries are appended, one per superseded review with revisions, oldest first, each with `prior: true` and `round: review_round(a)`.
- The prior walk is a new function `prior_lane_files(t, p, name)` in `scheduler.rs`, beside `lane_files_by`. It cannot reuse `lane_files_by`, which returns the newest attempt per lane and so would lose the older revisions. It walks `t.attempts` in order and keeps each `Workflow` attempt whose artifacts hold `name`, whose `revisions` are non-empty, and which is not the shown attempt. Its lane is `a.context` when `p`'s stage runs per lane (`Stage::runs_per_lane`), as `lane_files_by` decides, and none otherwise.
- The two new `PathsView.plan_files` and `LaneFile` doc comments (`dispatch-control/src/lib.rs:546`–`550` and `:586`–`600`) are updated: `plan_files` now also lists superseded revised copies, and a `prior` entry is not the plan.
- The note text comes from one new function `lane_file_note(f: &LaneFile) -> Option<String>` in `main.rs`, called by `print_lane_files` (`main.rs:896`):
  - `prior`: "(prior revision, round N)".
  - `failed` and not `reviewing`, with `round` `Some`: "(reviewed copy, round N, review failed)".
  - Otherwise the existing "(reviewed copy, round N, review open)" for `reviewing`, unchanged.

So a finished review's copy, which is shown and not failed, prints exactly as it does today. The new labels appear only for a failed revised copy and for superseded revised copies. Those are the only output changes, and they happen only on tickets with such attempts.

## Tests

- **`ticket.rs` tests**, beside the loop at `:1858`:
  - a `Failed` attempt with `revisions`, on a ticket not parked: `shown` and `review_copy` return its copy, `input` still returns `plan/1`;
  - the same attempt with the ticket `Parked`: `shown` and `review_copy` return `plan/1`;
  - a `Cancelled` attempt with `revisions`: `plan/1`.
  The existing loop is unchanged.
- **`dispatch/tests/first_slice.rs`**, beside `a_run_switchboard_no_longer_has_fails_the_attempt` (near `:3806`):
  - `a_rerun_after_a_revise_starts_from_the_revised_copy`: `at_finalize`, then `revise` with a note. The test writes "revised" into the `review/1` copy in the reviser's place, then `runs.clear()`, then `step`, which fails the attempt. Answer `rerun`, then `step`. Attempt 2's copy holds "revised", `WorkflowStart.plan` names it, its `source` is the plan session, and the rerun question contains the feedback file's path.
  - `a_rerun_after_a_failure_without_revise_starts_from_the_plan_stage`: the same without `revise`; the copy is the plan stage's and the question has no revision clause.
- **`dispatch/src/serve.rs` tests**, beside `:1455`–`:1497`: `ticket_paths` lists the failed revised copy with `failed: true`. With attempt 2 open, it also lists attempt 1 with `prior: true` and `round: 1`.
- **`dispatch/src/main.rs` tests module** (`#[cfg(test)]` at `:1898`): required. A unit test of `lane_file_note` for the three cases above, and for a finished, non-failed copy returning `None`.

Gate: `cargo test --locked --workspace`, `cargo clippy --locked --workspace --all-targets -- -D warnings`, `cargo fmt --all`, and `cargo clippy --locked -p dispatch --all-targets --target x86_64-unknown-linux-gnu -- -D warnings`. The Switchboard app builds unchanged, because `LaneFile` is constructed only in `dispatch/src/serve.rs` and its tests.

## Docs

- **`docs/design.md`**:
  - A new dated section "Plan review rerun copy (2026-10-21)" before "## Open questions" (`:3672`), stating the `carries_revision` rule, the parked exclusion, the rerun clause, the planner-session choice, and the `show` labels.
  - Amend `:1784`–`:1787`: a rerun "starts a fresh run", which now reviews the copy the new section names.
  - Amend `:2163`–`:2165`: agent prompts get complete files, except that a plan review's copy the owner revised is handed to the next review, per the new section.
  - Amend `:2533`–`:2537`: `plan_files` shows an open review's copy and now also a failed revised copy and the prior revisions.
- **`docs/dispatch-agent-guide.md`** (`:208`–`:216`): describes the `plan` line, the `(reviewed copy, round N, review failed)` note, and the `prior` entries `(prior revision, round N)`, so an agent reading `show` knows which file is current.
- **`README.md`**: no line changes. No file is added, and the Layout entries describe the files at the level they already do.
- **`CHANGELOG.md`**, under Unreleased: "- A plan review rerun after a failed attempt starts from the copy the owner revised, not the plan stage's original, and `show` and the rerun question name the copy the review reads."

## Commit message

`Dispatch: a plan review rerun starts from the owner's revised copy; adds LaneFile.prior and LaneFile.failed`

## Contract additions

- `dispatch_control::LaneFile.prior: bool` (serde default false).
- `dispatch_control::LaneFile.failed: bool` (serde default false).
