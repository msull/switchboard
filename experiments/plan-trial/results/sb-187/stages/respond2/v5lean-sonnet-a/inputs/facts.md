# Facts for #187 (tree HEAD 7fe8949)

The issue and the investigation claim nothing about a browser, platform, crate or protocol outside this tree. Every open question is about this tree, a run, or the owner's machine, so I ran no web searches and wrote no spike. No spike is needed: no question the plan turns on is left to an outside source.

## Settled

Evidence is `file:line` in this worktree.

- Nothing in `dispatch/src/` deletes a plan-review attempt directory or its `plan.md` or `plan.feedback-<n>.md` files.
  - The `remove_dir_all` at `dispatch/src/scheduler.rs:10081` removes only `Evidence.dir`, in `drop_evidence`.
  - The `remove_file` at `scheduler.rs:10154` is in `prepare_writes` ("A gate-only command's artifacts made ready", 10136). It is for gate commands, not workflow attempts.
  - The `remove_file` at `scheduler.rs:7642` removes only the just-written owner round file, and only when the app refuses the revise.
  - `retire_replaced` (3578-3605) deletes no directory.
  - So `review/1/joined/plan.md` and its feedback file survive the failure and the rerun. A fix can read them.
- The app can drop a `WorkflowRun` from the project record only through `remove_workflow` (`src/core/workflow.rs:1009-1028`). It refuses while the run `waiting()`. It is also lost on a store reset.
  - `no such run` therefore means the run was removed by a user or the store was reset, not that the app lost it on a plain restart. The tree does not say which happened for the reporter (see For the owner).
  - Evidence: `workflow.rs:1023` is the only `workflows.retain` on records. `core/control.rs:433` and `workflow.rs:54` retain definitions in settings, not runs.
- A `rerun` answer's `--note` is accepted and then dropped for a plan review.
  - `needs_note` (`dispatch/src/lib.rs:101-103`) requires a note only for `finalize`/`revise`, so `decide` for `rerun` takes one. `scheduler.rs:8150` enforces the requirement.
  - `rerun_note` (`scheduler.rs:8988-8997`) returns `None` unless the stage is `Agent` or `Review`. A plan review is `StageKind::Workflow` (`pipeline.rs:715-725`).
- `RunView` (`control/src/lib.rs:754-767`) carries `id, project, definition, state, round, cap, source, plan, reviewer, planner, op`.
  - It carries no round-snapshot path. Dispatch learns the snapshot directory (`snapshot_dir`, `src/core/workflow.rs:41-48`, `<data dir>/workflows/<run>/round-<n>/`) only by reading the app's data directory itself.
  - `RunView.plan` is the path of the copy Dispatch gave the app at start. In the repro that is `review/1/joined/plan.md`, which Dispatch already holds as `Attempt.artifacts["plan"]`. The snapshot is not needed to find the revised copy.
- `WorkflowStart { source }` must name a live, resumable planning Claude session.
  - The fake app refuses an unknown source with `no such session` and a non-resumable one with `cannot be the planner` (`dispatch/tests/support/mod.rs:397-405`). `design.md` is read as saying the same for the real app.
  - `review_subject` (`scheduler.rs:10534-10548`) returns `a.session` of the chosen attempt. For a workflow attempt `poll_workflow` overwrites `Attempt.session` with the run's `reviewer`/`planner` face (`scheduler.rs:7784-7788`). Picking `review/1` as the subject there would pass a reviewer or planner id as `source`.
  - The planning session to name is `plan/1`'s session. `review/1.run_sessions` (filled by `note_run_sessions`, `scheduler.rs:7710-7735`, keys `reviewer`/`planner`) holds the run's cloned `planner` session. The fake creates that session as "planner planner" (`support/mod.rs:416-423`), and a rerun may use it only if it is still resumable.
- The fake app does not read the plan file on `WorkflowStart`.
  - It stores `plan: plan.clone()` in a `RunView` (`support/mod.rs:430-441`) and never opens the file.
  - A test must therefore assert on the file Dispatch copies (`review/<n>/joined/plan.md`) and on the `plan` path in the recorded `WorkflowStart`, which are Dispatch-side facts.
- The cause is `review_subject` (`scheduler.rs:10534-10548`): it filters `AttemptState::Complete`, so the Failed `review/1` is skipped and `plan/1` is copied (`scheduler.rs:7465-7472`). `rerun_carries` (`scheduler.rs:9382-9390`) returns `""` for a workflow stage, so the rerun question never names a copy.
- Existing tests that constrain a fix:
  - `dispatch/src/ticket.rs:1827-1875` asserts a Failed or Cancelled review copy is not shown as the plan (1856-1864).
  - `ticket.rs:1797` asserts a Failed plan attempt is skipped as an input.
  - `dispatch/tests/first_slice.rs:3257-3287` and `3817-3831` cover the failed-review rerun without checking which plan was copied.
  - `first_slice.rs:3442-3500` covers revise without a rerun.
  - No test combines revise, a failed review and a rerun.
- `show`'s `files:` block (`dispatch/src/serve.rs:538-600`, `main.rs:980-995`) takes `review` from the latest workflow attempt that has artifacts (`serve.rs:564-568`).
  - After the rerun that is `review/2`, so `plan_rounds` lists nothing for the owner's round in `review/1`.
  - `print_answered` (`main.rs:1176-1198`) still prints the `revise` note while the decision stays on the ticket, so the note is visible there.
- Rendering of `plan_reviewing`/`plan_round` in `client/Sources` was not read. It is a presentation question for the planner, not a fact the plan turns on.

## Unresolved

- Whether a half-written `review/1/joined/plan.md` is possible after a killed run (the file is edited in place by the app).
  - Tried: reading `start_workflow`/`revise` in Dispatch. Dispatch only copies and reads it. The app's edit path is in `src/core/workflow.rs`, which I did not read in full.
  - Not settled because the answer needs the app's write behaviour or a killed run, which is outside the smallest experiment the plan turns on.
  - Build to: copy `review/1/joined/plan.md` as it is, and write the new attempt's copy with the same copy call. This is safe either way and needs no new atomic write: a partial file is the owner's call to inspect, and the rerun question names which copy is used.
  - If favourable, nothing more is needed. If unfavourable, nothing more is needed either, because the owner can see the named copy.
- Whether the planning session `plan/1` names is still resumable at rerun time (needed for `WorkflowStart.source`).
  - Tried: reading `start_workflow` and the fake. The real check lives in the app and needs a live session.
  - Build to: keep passing `plan/1`'s session as `source`, as the first attempt does today. If it is gone, the existing `Reply::Failed` branch (`scheduler.rs:7522` onward) already fails the attempt with a reason and asks `rerun`/`park`. The plan must add no fallback to another session.
- Whether `Revision` entries on `review/1` can be re-sent to a new app run.
  - The `WorkflowObject { run, round, text }` body targets an existing run (`revise`, `scheduler.rs:7565-7650`). Whether the app accepts an objection on a fresh run at round 1 was not checked.
  - Build to: the plan must not depend on resending the owner's note. Starting the rerun from the revised copy carries the revision itself. The note stays available as the answered `revise` decision.

## For the owner

Questions of fact only the owner can observe. Neither answer changes what the plan builds, and none blocks it.

- Does `review/1/joined/plan.md` on the reporter's machine still hold the revision (diff against `plan/1/joined/plan.md` and `review/2/joined/plan.md`), and does `review/1/joined/plan.feedback-2.md` exist with the owner's note?
  - If yes: the fix can read both from the failed attempt, and the issue's repro is confirmed on disk.
  - If no: the fix still starts the rerun from the newest copy Dispatch holds (the failed attempt's copy or `plan/1`) and names it in the question and in `show`.
  - The plan builds the same either way.
- Did the Switchboard app's project record lose the run (a removed run, a store reset, or a changed data directory) between the review's start and the `rerun` answer?
  - If yes: `no such run` is explained, and the plan may add a test that models it with `runs.clear()` (`support/mod.rs:690`).
  - If no (runner restart only): the cause is outside this tree. The plan still builds the same fix, because a failed attempt of any reason has to hand its copy to the rerun.
- Does attempt 2's plan carry the content the issue describes (every question open, an owner answer contradicted, two of three round-1 points already fixed in the revised copy)?
  - If yes: the plan has a worked example for its test fixtures, with the client's text replaced.
  - If no: the plan builds the same. The content check is the owner's own ticket and does not bear on the mechanism.
