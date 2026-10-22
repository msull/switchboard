# Facts: issue #187

Nothing in the issue or the investigation depends on a browser, platform, crate, protocol or tool outside this tree, so no web search was run and no spike was needed. Every open question marked [tree] is answered by reading; Switchboard is this repo. Paths are in this worktree.

## Settled

Failed workflow files are kept
- `fail_attempt_with` only sets `state: Failed`, `ended_ms`, the failure flags, saves the ticket, and asks or parks; it touches no file. `dispatch/src/scheduler.rs:7195-7260`.
- The only `remove_file` calls: `scheduler.rs:7642` (a feedback file Switchboard refused, in `revise`), `scheduler.rs:10154` (`prepare_writes` removes the *new* agent attempt's own artifact paths), and `scheduler.rs:10081` (`drop_evidence`, evidence dirs only). None touches a failed Workflow attempt's `plan.md` or `*.feedback-N.md`. So review/1's revised copy is still on disk at rerun time, unless something outside Dispatch removes it.

Why the rerun starts from plan/1
- `workflow_stage` calls `start_workflow` for the rerun once `may_rerun` is true (`scheduler.rs:7396-7425`). `resume_with` / `park_reruns` / `reruns_on_resume` (`scheduler.rs:8262`, `8344`, `9169`) only select cancelled attempts and write `rerun` answers; they launch nothing. Every plan-review start, first or rerun, goes through `start_workflow` (`scheduler.rs:7436`), so one change there covers `rerun` and `dispatch resume`.
- `start_workflow` takes its input from `review_subject`, which keeps only `Complete` attempts (`scheduler.rs:10534-10549`). It copies that path to the new attempt's `plan.md` (`scheduler.rs:7469-7472`).
- The same `Complete` filter is `Ticket::input_where` (`dispatch/src/ticket.rs:1452-1463`), used for what agents are handed (`scheduler.rs:10849`). Only `Ticket::shown_where` (`ticket.rs:1476-1492`) also counts an open Workflow attempt. A failed attempt is in neither.
- A fix in `review_subject` is enough for the rerun's input. Later stages read only `Complete` attempts, so they get the finalized copy of the attempt that completes; nothing else picks a plan for a failed-then-rerun review.

The planner's history is separate from the file
- `start_workflow` passes `source` = the session of the chosen attempt (`scheduler.rs:7515`; `review_subject` returns `a.session` of the same attempt). Switchboard's `start_workflow` requires a Claude Code session with a `ResumeHandle::ClaudeCode` transcript (`src/core/workflow.rs:300-312`) and queues `CloneAllTranscript` of that handle (`workflow.rs:374`). `clone_all` copies the **whole** source transcript under a fresh session id (`src/adapters/transcript.rs:105-125`; the clone mechanism is spike `07-clone-session`).
- So attempt 2's planner has the history of whichever session `source` is. With plan/1 chosen it has the plan session's history and none of attempt 1's rounds or the owner's `revise` note. The planner clone of review/1 is a `SessionRecord` with a Claude Code `resume` handle (`workflow.rs:~410-420`; `Attempt.session` for a workflow attempt is that planner, per the investigation). Whether review/1's `Attempt.session` is always set when the run failed early is not read; see Unresolved.

The owner's note survives on disk
- `revise` writes `<stem>.feedback-<round>.md` beside the reviewed copy and pushes a `Revision` on the attempt before sending `WorkflowObject` (`scheduler.rs:7565`; `report.rs:156`; `ticket.rs:350`). It is removed only if Switchboard refuses the round (`scheduler.rs:7642`). Nothing reads it for the next attempt.

Failure source
- `no such run` is Switchboard's reply when `core.run_view` finds no workflow with that id (`src/app.rs:3918`). Dispatch fails the attempt with `review run gone: no such run` (`scheduler.rs:7731-7765`, `NO_SUCH_RUN` at `scheduler.rs:8445`). Test: `dispatch/tests/first_slice.rs:3807`.

Schema
- `RECORD_VERSION` is 28 (`dispatch/src/store.rs:26`); `migrate` is `store.rs:452`. A rule that reads only existing fields (`Attempt.artifacts`, `state`, `revisions`, file existence) needs no bump. A new serialized field on `Attempt` or `Decision` would need 29 and a `migrate` step (CLAUDE.md "Extending the contract").

`show` and the app
- `show` names only the newest matching plan: `ticket_paths` builds `plan_files` from `shown_lane_files` -> `shown_where` (`dispatch/src/serve.rs:539`, `scheduler.rs:10900-10919`), which skips Failed attempts, so with review/1 Failed and review/2 open only review/2's copy appears. Matches the issue.
- `LaneFile { lane, stage, path, reviewing, round }` is `Serialize/Deserialize` with `#[serde(default)]` (`dispatch-control/src/lib.rs:584-598`), so an added field is additive and an older reader ignores it.
- The app's ticket page reads `details.paths.plan` and `plan_rounds` (`src/ui/ticket.rs:440-470`); the grep finds no use of `plan_files` under `src/`. A new `LaneFile` field therefore reaches only the CLI unless the app is changed.
- The rerun question text is `rerun_ask` (`scheduler.rs:9335`), taking `(&Ticket, &Attempt)`; `rerun_carries` (`scheduler.rs:9382`) says what the next attempt carries, for code reviews only, empty for a workflow attempt. A path can be named there with no new input. Code-review reruns already carry state (`carried_from`, `docs/dispatch.md:3058`).

## Unresolved

U1. A failed attempt that never got an owner `revise` (failed in round 1): is its copy newer than plan/1 and worth carrying?
- Tried: read `start_workflow` (copy of plan/1 is made at start, `scheduler.rs:7469`) and `docs/dispatch.md:2909` ("The copy in the review attempt changes; the plan attempt's file does not"). The copy is edited in place by the planner from round 1 on, so even a no-revision failed attempt may hold planner edits. Whether those edits are complete when the run is lost is a runtime fact. No queries (tree question); not run.
- Build to: the rule "newest attempt of this stage with a `plan` artifact whose file exists, and whose `revisions` is non-empty" is safe under both answers: it carries the owner's revise whenever there is one and falls back to plan/1 otherwise. If the planner's mid-round edits are known to be sound, the plan may drop the `revisions` condition. Never delete or overwrite review/1's file either way.

U2. Does a runner restart actually produce `no such run`, and is review/1's `plan.md` complete when it is noticed?
- Tried: read the reply site (`src/app.rs:3918`, run lookup by id; workflow runs are in the workspace record) and Dispatch's handler. Nothing in the tree shows a restart removing the run; running one needs the signed app. Not searched on the web: it is this repo's behaviour, not a vendor's.
- Build to: if a restart does produce it, the plan covers the issue's repro as written. If it does not (the run vanishes another way, e.g. the run record deleted), the same fix is needed: the decision depends only on the failed attempt's record and file, not on why it failed. The plan never waits on this.

U3. Is review/1's `Attempt.session` set for every failed workflow attempt?
- Tried: `review_subject` returns `a.session` and errors "was not written by a session" if `None` (`scheduler.rs:10546`); `poll_workflow` sets it from the planner (`scheduler.rs:~7790`, not re-read here).
- Build to: if it is always set once the planner exists, the plan may name review/1's planner as the new `source`. If it can be `None` (failure before the planner clone), the plan must fall back to the plan stage's session while still copying the revised file; a failure before any planner exists has no revision to carry anyway.

## For the owner

O1. Is `review/1/joined/plan.md` still on disk and unchanged in the real ticket directory?
- Favourable: the issue's repro is the fix's own regression case. Unfavourable: the fix is unchanged (it reads the record and file of any failed attempt); the plan must still handle the carried file missing by falling back to plan/1 and saying so in the rerun question. Works in both cases: fall back with a stated reason.

O2. What exact `show` and rerun-question text did the owner see, and on which build (before 0.4.1)?
- Favourable or not, the plan builds the rerun question and `show` text from the tree (`rerun_ask`, `print_lane_files`) and does not depend on the old wording.
