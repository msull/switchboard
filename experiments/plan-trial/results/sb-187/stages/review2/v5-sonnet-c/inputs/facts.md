# Facts: issue #187 (HEAD 7fe8949, Release 0.4.1)

The issue and the investigation claim nothing about a browser, platform, crate, protocol or tool outside this tree, so no web search or spike was run (a spike would test only the tree's own code, which the reading below settles). Every open question marked [tree] is answered from the tree; the rest are below.

## Settled

Root cause (confirms the investigation):
- A rerun's plan comes from `review_subject`, which keeps only `Complete` attempts, so failed `review/1` is skipped and `plan/1`'s copy and session are used. `dispatch/src/scheduler.rs:10534-10548`; called only from `start_workflow` (`scheduler.rs:7465`; grep of `dispatch/src/*.rs` finds no other caller).
- `NO_SUCH_RUN` is the app's reply when `core.run_view` finds no run: `src/app.rs:3915-3918`; the runner turns it into `fail_attempt("review run gone: ..")`: `scheduler.rs:7755-7763`.

Q: does any path remove a failed workflow attempt's copy or `artifacts` entry?
- No. In `dispatch/src/scheduler.rs`, `ticket.rs` and `restart.rs`, grep for `artifacts.remove`, `artifacts.clear` and `artifacts =` finds nothing; the only `artifacts` writers are the attempt constructors at `scheduler.rs:4453`, `6248` and the `start_workflow` record at `7499-7506`. `forget` hits are `forget_secrets` only (`scheduler.rs:2093`, `2671`). So `review_subject` may rely on a failed attempt's `artifacts["plan"]` entry. Whether the file still exists on disk is not checked by anything; a fix should handle a missing file.

Q: what do restart, park and close do with a failed workflow attempt, and do they call `review_subject`/`start_workflow`?
- `review_subject` has one caller (`start_workflow`), and `start_workflow` is reached from `workflow_stage` (`scheduler.rs:7401-7412`, fresh attempt or `may_rerun`). `restart.rs:883` treats `AttemptKind::Workflow | GateOnly` as having no restart behaviour (`false`); it does not call either function. Park is the other option of the `rerun` question (`scheduler.rs:7270-7290`) and `rerun_ask` re-asks it (`9335`); neither starts a workflow.

Q: serde defaults on `LaneFile`/`PathsView`, and is a new marker additive?
- Both are `#[serde(default)]` structs deriving `Default`: `dispatch-control/src/lib.rs:584-586` (`LaneFile`) and `:541-542` (`PathsView`). A new `bool`/`Option` field on either deserializes from an older runner as false/none, which is additive per CLAUDE.md "Extending the contract". Existing fields to reuse: `LaneFile.reviewing`, `LaneFile.round`, `PathsView.plan_reviewed`, `plan_round`, `plan_rounds` (`lib.rs:553-573`, `595-598`). The app's Plan tab consuming them was not read, so "the tab ignores an unknown field" rests on serde's default of ignoring unknown fields on the app side, not on a read of the tab.

Q: RECORD_VERSION and migration:
- `dispatch/src/store.rs:26` `RECORD_VERSION = 28`; `migrate` at `store.rs:451` steps one version at a time, and earlier steps document "arrives from its serde default" for added fields (`store.rs:455-470`). A new `Attempt` field therefore needs version 29 plus a comment step; a fix that only changes `review_subject` needs neither.

Q: `Attempt.revisions` as a signal:
- It is pushed by `revise` on the attempt that holds the copy (`scheduler.rs:7620-7625`, per the investigation) and is kept on a failed attempt (investigation l.26). It says the copy was edited by a revise, but it does not say the planner finished writing it (see Unresolved). Comparing files is not needed to know a revision was requested; it is the only way to know the content differs.

Q: how do tests fake `NO_SUCH_RUN`?
- There is no existing test that drives `poll_workflow` into `review run gone`: grep of `NO_SUCH_RUN`/`run gone` in `dispatch/src` shows only the three production uses (`scheduler.rs:2840`, `7755`, `8445`/`7761`). A test must be built from the fake app reply used by the existing workflow tests (`scheduler.rs` near `12838`, `new_attempt(.., AttemptKind::Workflow, ..)`) answering `Reply::Failed { reason: NO_SUCH_RUN.into() }` to `Body::Workflow`. Which fake harness answers `Body::Workflow` was not read; the planner should read it before naming the test.

`show` / paths:
- `shown_where` lists only `Complete` or open-workflow attempts (`ticket.rs:1476`, investigation l.20), so after `review/2` starts, `review/1`'s copy is listed nowhere. `lane_plan`/`lane_input` also take only Complete attempts (`scheduler.rs:10843-10858`), and their test at `scheduler.rs:12838-12869` pins that a waiting `finalize` still hands later stages the outline, not the open copy.

## Unresolved

U1. Can a revised copy be half-written or stale when `no such run` comes back? Tried: read `src/core/workflow.rs` `start_workflow` (l.289-330) and `src/app.rs:3915`; queries to outside sources: none (it is the tree's own behaviour, and the write is by a Claude Code agent in the owner's session, not a documented vendor guarantee). Not settled because the planner's in-place write is an agent action, not code in the tree. Line to build to: if the revised file is complete, a rerun may start from it; if not, the plan must still start from it (it is the newest the owner approved to revise) and must say in the rerun question and in `show` which copy it reviews and that its `revisions` count is non-zero, so the owner can check it.

U2. Does `run_view` return None after an app restart (what makes `no such run`)? Tried: `src/core/control.rs:865` and `src/app.rs:3915`; whether `self.workflow(id)` is rebuilt from disk on launch was not read. Line to build to: assume a run can be gone at any time (the `Paused{failed:true}` path fails the attempt identically, investigation l.14); the fix must not depend on the cause.

U3. Repro (revise, kill run, rerun, diff). Not run: it needs a live app, a Claude Code planner session and a runner, and the reading of `review_subject` is unambiguous. Line to build to: the plan includes a scheduler test that seeds a failed `review/1` with `revisions` non-empty and asserts which path `start_workflow` sends in `WorkflowStart.plan`; the test, not the live repro, is the gate.

## For the owner

O1. Did the 09:28 revised copy on the real ticket look complete, and was the runner or the app restarted when `review run gone` appeared (ticket record and runner log)?
- Favourable (complete, restart): the plan may start attempt 2 from `review/1/joined/plan.md`.
- Unfavourable (partial or the run was pruned for another reason): the plan must still start from the newest attempt's copy and name it in the rerun question and in `show`; it works in both cases.
