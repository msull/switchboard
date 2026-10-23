# Issue #187: a plan review rerun starts from the plan stage's output, not the revision

Worktree: `/Users/sully/code_repos/plan-trial/sb-187-stage-investigate-v5lean-haiku-a`, branch `stage/sb-187-investigate-v5lean-haiku-a`, HEAD `7fe8949` (Release 0.4.1). Tree was clean. No file in the repo was edited.

Read: `CLAUDE.md` (given), `docs/design.md` (plan review and Dispatch sections: ~1000-1030, 1180-1200, 1620-1640, 1738-1790, 2155-2220, 2525-2545, 3612-3645), `docs/lessons.md` (84 lines; nothing on reruns or revisions, so it adds no constraint here). Read the code paths listed below in full.

## Summary

The bug is in the code, not the issue reporter's setup. A plan-review rerun copies its input from the newest **Complete** attempt that has a `plan` artifact. A review attempt that failed with `review run gone` is never Complete, so the rerun takes `plan/1`. The owner's `revise` lives in two places that the rerun never reads: the revised copy in `review/1/joined/plan.md`, and the owner's round file `review/1/joined/plan.feedback-<n>.md` (with a `Revision` entry on `review/1`). The `rerun` note path also drops everything for a workflow stage. `show` prints only the newest workflow attempt's copy, so the lost revision is not named anywhere.

## Code that the issue touches (this tree)

### 1. The failure: `review run gone`
- `dispatch/src/scheduler.rs:7741-7790` `poll_workflow`: asks the app `Body::Workflow { run }` (7753). On `Reply::Failed` with `NO_SUCH_RUN` (7755) it calls `fail_attempt` with `review run gone: no such run` (7761).
- `dispatch/src/scheduler.rs:8445` `NO_SUCH_RUN = "no such run"`.
- `dispatch/src/scheduler.rs:7148-7158` `fail_attempt` → `fail_attempt_with(... &["rerun", "park"])`. Failure reason is stored on the attempt; state becomes `Failed`.
- App side: `src/app.rs:3915-3919` answers `Body::Workflow` from `core.run_view(WorkflowId)`; `None` → `"no such run"`. `src/core/control.rs:865` `run_view`; `src/core/workflow.rs:222` `workflow(id)` looks in `self.workflows()`. Runs are persisted in the project record: `src/core/model.rs:337` (`WorkflowRun`), `src/adapters/store.rs:1454` (`workflows: Vec<WorkflowRun>`).

### 2. The rerun question
- `dispatch/src/scheduler.rs:7308-7332` `ask_rerun` → `rerun_ask` at `9335-9377`. The question reads `"<stage> (<ctx>) attempt <n> failed: <reason>. Run it again?"` plus `rerun_carries` (9382-9390), which returns `""` for a workflow stage. The question says nothing about which copy will be reviewed.
- `scheduler.rs:9382-9390` `rerun_carries`: carry text only for `AttemptKind::Review` (code review), not plan review.

### 3. The rerun decision and the note
- `dispatch/src/scheduler.rs:3524-3530`: the `("rerun", "rerun")` arm reads `rerun_note(t, p, i)` then `retire_replaced(...)`.
- `dispatch/src/scheduler.rs:8988-9013` `rerun_note`: returns `None` unless the stage kind is `Agent` or `Review` (8993-8997). A plan review is `StageKind::Workflow` (`dispatch/src/pipeline.rs:715-725`, `kind()`), so the `rerun` answer's own note is discarded for a plan review, and nothing is put into `t.rework`.
- `dispatch/src/scheduler.rs:10599-10601` `sent_back`: "A workflow stage takes no note."
- `dispatch/src/scheduler.rs:3578-3605` `retire_replaced`: marks the decision acted, saves, unmarks. It deletes no directory.

### 4. The rerun itself
- `dispatch/src/scheduler.rs:7379-7434` `workflow_stage`: for a latest attempt that is not open and not Complete, `may_rerun` (7402) → `start_workflow(... next_n ...)`.
- `dispatch/src/scheduler.rs:7436-7530` `start_workflow`:
  - 7465-7470: `review_subject(t, &subject)` gives `(source_path, source_session)`.
  - 7471-7472: `std::fs::copy(&source_path, &copy)` where `copy = <attempt dir>/plan.md`, a new file under `review/<n>/joined/`. Note the new attempt is `n = next_n`, so attempt 2 gets `review/2/joined/plan.md`.
  - 7480-7487: `new_attempt(... AttemptState::Starting, BTreeMap::from([(subject, copy)]))`.
  - 7510-7520: `Body::WorkflowStart { source: source_session, plan: copy, ... }`.
- `dispatch/src/scheduler.rs:10534-10548` `review_subject`: `t.attempts.iter().rev().filter(|a| a.state == AttemptState::Complete)` → first attempt with artifact `subject`; returns that attempt's path and `a.session`. Attempt 1 (Failed) is skipped; attempt `plan/1` (Complete) is picked. This is the reason for the bug. The rule ignores `forgotten` and lane visibility, so it is a third reader of the same question.

### 5. The revise path (where the owner's note goes)
- `dispatch/src/scheduler.rs:3495-3498`: `("finalize", "revise")` → `revise(...)` then `unmark`.
- `dispatch/src/scheduler.rs:7565-7650` `revise`:
  - needs the run to be `Converged` or `AtCap` (7589-7603 region);
  - `round = view.round + 1`; `file = crate::report::round_file(&subject, round)` at 7616: `<stem>.feedback-<n>.md` **beside the reviewed copy**, i.e. `review/1/joined/plan.feedback-<round>.md`;
  - writes `"<OWNER_ROUND_HEADING>\n\n<note>"` with `atomic_write` (7617-7618);
  - `review.revisions.push(Revision { round, by, at_ms })` at 7621 on the attempt (`review/1`);
  - sends `Body::WorkflowObject { run, round, text }` (REVISE) to the app. On refusal it removes the file (7642).
- `dispatch/src/ticket.rs:510-516` `Revision { round, by, at_ms }`; `dispatch/src/ticket.rs:350` `Attempt.revisions`.
- `dispatch/src/report.rs:151-158` `round_file` / `beside`; `report.rs:165` `plan_round_numbers` lists the files that exist.
- `dispatch/src/lib.rs:102` `revisable` (finalize/revise pair); `lib.rs:95` mentions `revisions`.

### 6. The revised copy
- The copy is `review/1/joined/plan.md`, the attempt's `artifacts["plan"]` (set in `start_workflow`'s `new_attempt` for attempt 1). Per `docs/design.md:2213-2216`, the review edits its copy in place under that one path. Nothing in the tree copies it out at revise time, and nothing reads it after the attempt fails.
- The app keeps round snapshots at `<data dir>/workflows/<run>/round-<n>/` (`src/core/workflow.rs:41`), per `docs/design.md:1000-1030`. Dispatch does not read them.

### 7. The readers that disagree (the core of the bug)
| Reader | Rule | Location |
|---|---|---|
| Agent inputs (`input`, `input_where`, `lane_input`, `lane_plan`) | newest **Complete**, not forgotten | `ticket.rs:1437-1462`; `scheduler.rs:10843-10856` |
| Display (`shown`, `shown_where`, `shown_lane_files`) | newest Complete, or an **open** Workflow attempt | `ticket.rs:1467-1490`; `scheduler.rs:10900-10960` |
| Plan-review rerun source (`review_subject`) | newest **Complete** with artifact, plus its session | `scheduler.rs:10534-10548` |

The existing unit test `ticket.rs:1827-1875` (`the_plan_shown_is_an_open_review_copy_but_an_input_never_is`) asserts that a Cancelled or Failed review copy is **not** shown as the plan (1856-1864: "A parked review's copy is not the plan going forward"). So "failed" and "parked" are treated the same by the current design. A fix that makes the failed copy the rerun's input must change that rule or add a distinction; the test is the constraint the planner has to face. The first test, `ticket.rs:1797`, asserts that a Failed plan attempt's input is skipped.

Note on the session: `poll_workflow` writes the card's face into `Attempt.session` (`scheduler.rs:7784-7788`: `view.planner` or `view.reviewer`). So `review/1.session` is the review run's face, not the planning session. `review_subject` returns `a.session` alongside the path, so a fix that picks `review/1`'s copy by the same function would hand `WorkflowStart { source }` the wrong session unless it takes the planning session from `plan/1` (or from `review/1.run_sessions`, which `note_run_sessions` fills at `scheduler.rs:7710`). Design says the source is the *original* planning session (`docs/design.md:1000-1010`).

### 8. What `show` prints (the "files:" block)
- `dispatch/src/main.rs:911-997` `show`. The `files:` block is at 980-995:
  - 982 `print_lane_files("plan", &p.plan_files)`: `plan: <path>` and, if `reviewing`, `(reviewed copy[, round N], review open)` (main.rs:896-908).
  - 984-989: `round`, `review summary`.
  - 991-994: `plan round N (owner): <feedback path>` from `p.plan_rounds`.
- `dispatch/src/serve.rs:538-600` `ticket_paths`:
  - 539-546 `plan_files = shown_lane_files(t, p, "plan")`; `reviewing` is `Workflow && is_open()`; `round = review_round(a)`.
  - 564-568 `review` = the **latest** Workflow attempt with artifacts. That is attempt 2 in the issue's repro, which has no round files yet. So `plan_rounds` (572) comes from attempt 2's copy and lists nothing, and the owner's round 2 that sits beside attempt 1's copy is not listed.
  - 570-571 `shown = t.shown("plan")`; `plan: shown.path`.
  - `serve.rs:504-523` `plan_rounds` and `serve.rs:526-534` `review_round` read round files and `Revision` entries of the given attempt only.
- The answered decision keeps the owner's note: `main.rs:1176-1198` `print_answered` prints `finalize [..]: answered revise by <who> — <note>` for every answered decision still on the ticket. So the note is visible under `answered:` while the decision is kept. The issue's "nothing warned" is true of the `files:` block and of the rerun question, not of the answered list.

## On-disk record and rehydration (before the pane)

Record and files, with the repro's paths:
- Ticket record under `<data>/tickets/<id>/` (`store.rs:147-149` `ticket_dir`). It holds `attempts` (`ticket.rs:266` `Attempt`), `decisions` (with the answered `revise` note), and `revisions` on `review/1`.
- Attempt directories: `ticket_dir/<stage>/<n>/<ctx>/` (`scheduler.rs:6303-6320` `attempt_dir`). Repro:
  - `plan/1/joined/plan.md`: the plan stage's output. Complete. Session: the plan stage's session on `plan/1`.
  - `review/1/joined/plan.md`: the copy of the plan at review start, edited in place by the review run, containing the revision. `review/1` is `Failed` with reason `review run gone: no such run`.
  - `review/1/joined/plan.feedback-2.md`: the owner's objection (`OWNER_ROUND_HEADING` + note). Keyed to round 2 of the app run. Also `plan.response-2.md` if the planner answered.
  - `review/1` `revisions`: `[{round: 2, by: "you", at_ms}]`.
  - `review/2/joined/plan.md`: fresh copy of `plan/1/joined/plan.md` made by `start_workflow` (7471).
- App side: the `WorkflowRun` in the project record of the Switchboard app's store (`src/core/model.rs:337`; persisted, `src/adapters/store.rs:1454`). It holds `plan` path, `rounds`, `state`, and round snapshots under `<data dir>/workflows/<run>/`.

Rehydration after a Dispatch runner restart (the issue's trigger):
- The runner loads the ticket record and sees `review/1` as an open attempt with `run` set. `poll_workflow` asks the app. The app's answer is `no such run` (the issue's report). That fails the attempt, the record is saved as Failed, and a `rerun` question is asked (`rerun_ask`). Nothing reads `review/1/joined/plan.md` or the feedback file again.
- `rerun` answer: `rerun_note` → None; `retire_replaced` (saved); next pass `workflow_stage` → `may_rerun` true → `start_workflow(n=2)` → `review_subject` → `plan/1` → copy → `review/2/joined/plan.md` → `WorkflowStart` with the plan copy. The new app run starts from `plan/1`'s text; no objection is sent. `review/2` is then the only open workflow attempt, so `shown` names it.

## What the pane shows now (from the code, not from running it)

`show`, after the repro (attempt 2 running), prints in the `files:` block:
```
files:
  plan: <data>/tickets/<id>/review/2/joined/plan.md
    (reviewed copy, review open)
```
`round` is absent because `review_round(review/2)` is None (no round files, no revisions on review/2). There is no `plan round` line for round 2, and nothing names `review/1/joined/plan.md` or its feedback file as a plan. The `attempts` block shows `review/1 joined failed` and `review/2 joined running` (per the issue; `print_attempts` not read). The `answered:` list shows the revise note.

The rerun question (for the failed review, before answering) is `review (joined) attempt 1 failed: review run gone: no such run. Run it again?` with options `rerun, park`. It does not say which copy will be reviewed.

The Plan tab in the client reads `PathsView.plan` (= `review/2` copy), `plan_reviewed` and `plan_reviewing` (serve.rs:577-579). I did not read the Swift rendering in `client/`.

## Tests in the tree that touch this

- `dispatch/tests/first_slice.rs:3257-3287` `a_failed_review_run_fails_the_attempt_and_a_rerun_starts_a_new_run`: asserts a second run exists (`a.run.is_some()`) but does **not** assert which plan the rerun copied.
- `dispatch/tests/first_slice.rs:3817-3831` `a_run_switchboard_no_longer_has_fails_the_attempt` (`env.sb().runs.clear()`): asserts the `review run gone: no such run` reason and a `rerun` question.
- `dispatch/tests/first_slice.rs:3442-3500` `revise_writes_the_owners_round_and_sends_it_as_an_objection`: asserts the round file and `Revision`, not a rerun.
- No test combines `revise` with a failed review and a rerun. The gap is the test, not only the code.
- `dispatch/tests/support/mod.rs:690` fakes `no such run` for an absent run.

## Other observations (tree facts)

- `scheduler.rs:10081` `remove_dir_all(&ev.dir)` and `scheduler.rs:10154` `remove_file(path)`: evidence/secret removal. I did not check whether either can reach `review/<n>/` dirs.
- `review_subject` ignores `forgotten`, so a forgotten plan artifact on a Complete attempt is still copied. Pre-existing, unrelated to #187 except that a fix touching `review_subject` will meet it.
- The `rerun --note` on a plan review is dropped (see 3). The issue does not mention this, but it is the same loss.

## What I did not check

- I did not run `cargo test`, the dispatch tests, or any Dispatch/Switchboard command, and I did not reproduce the repro.
- I did not read the app's planner/reviewer prompts or `src/core/workflow.rs` in full (only `round_file`/snapshot line and `workflow` lookup). I did not confirm how the app edits the copy, or whether a killed run leaves it half-written.
- I did not read `print_attempts`, the `decide` CLI path, `events.rs` needs_note, or the Swift client.
- I did not read the rest of `design.md` (3,600 lines), only the sections above; the Milestone status sections are unread.
- I did not verify the issue's claims about attempt 2's content (round-1 points, the finalize result) or about the `09:28` timestamp.
- Claims about the Claude/Codex sessions are not in scope and were not checked.

## Open questions

- [tree] Does anything in `dispatch/src/` remove or overwrite `<ticket>/review/<n>/<ctx>/` (attempt dir) on retire, rerun, or restart? Settle by reading `retire_processes`, `remove_dir_all` at `scheduler.rs:10081`, and every `remove_file` on attempt paths.
- [tree] Does the app's per-round snapshot (`src/core/workflow.rs:41`, `<data dir>/workflows/<run>/round-<n>/`) hold the plan as of the end of the last completed round, and can Dispatch learn its path through the `Workflow` reply (`src/core/control.rs:865` `run_view`) without reading the app's store?
- [tree] On which paths can the app remove a `WorkflowRun` from its project record other than a store reset (`src/core/workflow.rs` `workflows()`, `src/adapters/store.rs` reads and writes of `workflows`)? That settles what `no such run` after a restart means.
- [tree] Does `dispatch decide ... --note` on a workflow stage's `rerun` get refused, or accepted and dropped, given `rerun_note` returning `None` for `StageKind::Workflow` (`scheduler.rs:8988-8997`) and `sent_back` (`scheduler.rs:10599-10601`)? Settle by reading the decide path in `dispatch/src/main.rs` and `needs_note` in `dispatch/src/lib.rs`.
- [tree] For a failed plan-review attempt, which session is the planning session to name as `WorkflowStart { source }`: `plan/1`'s session, or a session in `review/1.run_sessions` (`scheduler.rs:7710` `note_run_sessions`)? Settle by reading `note_run_sessions` and the `source` use in `start_workflow` (7510).
- [tree] Does `dispatch/tests/support/mod.rs` (the fake app) keep the plan copy in its run's `plan` path and read it on `WorkflowStart`, so a test can assert the rerun's copy carries an edit? Settle by reading the fake's workflow handlers near `support/mod.rs:690`.
- [tree] How does `client/Sources` render `plan_reviewing`, `plan_round` and the copy's label when a plan review is open with no round (`serve.rs:577-579`)? Settle by reading the Swift view that consumes `PathsView`.
- [tree] Does the `review_round` / `plan_rounds` pair need a change for a rerun where round files live beside an older attempt's copy? Settle by reading the `shown` and `plan_rounds` callers in `serve.rs` and `main.rs`, and the test at `serve.rs:1685`.
- [run] Does `cargo test --locked -p dispatch` pass on this tree before any change? (Not run here.)
- [run] Does the reproduction (review run killed mid-run, then `rerun`) give `no such run` with the Dispatch runner restarted and the app still running, or only after the app restarts? The fake `runs.clear()` models the app losing the run; the real trigger is not confirmed in the tree.
- [run] After the review run is killed, is the planning session (`plan/1`'s session id) still answerable by Switchboard, so a rerun's `WorkflowStart { source }` can name it?
- [run] Does the app's copy at `review/1/joined/plan.md` survive a kill mid-round intact, or can a killed run leave it half-written? (Settle by a test run against a throwaway data dir and a killed run.)
- [owner] Does `review/1/joined/plan.md` on the reporter's machine still hold the revision (diff against `plan/1/joined/plan.md` and `review/2/joined/plan.md`), and does `review/1/joined/plan.feedback-2.md` hold the owner's note?
- [owner] Did the reporter's Switchboard app restart, or its data dir change, between the review's start and the `rerun` answer? (The `no such run` answer needs an explanation from the reporter's side.)
- [owner] Does attempt 2's plan contain the issue's "still lists every question as open" content, and do its round-1 points match the revised copy? (Content check on the reporter's files; not checked here.)
