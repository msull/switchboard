# Facts: issue #187

The issue and the investigation claim nothing about a browser, platform, crate, protocol or tool outside this tree. Every open question is about the tree, a run, or the owner, so no web search or spike applied (queries run: none). No spike was written; `git status` is clean. Line numbers are from this worktree.

## Settled

**The source of the second attempt's plan**
- The cause is `review_subject`. It walks `t.attempts` newest first and keeps only `AttemptState::Complete` attempts that hold the subject. A failed `review/1` is skipped, so `plan/1` wins. (`dispatch/src/scheduler.rs:10534-10548`)
- `start_workflow` calls `review_subject`, then `std::fs::copy`s the source into the new attempt directory. It records only `{subject: copy}` in the new attempt's artifacts. (`scheduler.rs:7465-7470`, `7482-7490`)
- `new_attempt` has no source or input field, and `Attempt` has no field naming where its plan came from. (`scheduler.rs:10277-10307`)
- `review_subject` has exactly one caller. (`scheduler.rs:7465`; grep of `dispatch/src`)

**The planner clone and the source session**
- `WorkflowStart` carries `source` (a session id from the same attempt as the plan) and `plan` (the copy path). (`scheduler.rs:7497-7506`)
- The app clones the source's whole transcript with `Effect::CloneAllTranscript`, not up to a chosen message. It sends the first reviewer prompt with the plan path. The planner is prompted with the same `plan` path on each round. (`src/core/workflow.rs:289-372`, `respond_prompt` at `125-132`)
- A plan newer than the transcript therefore changes nothing in how the clone starts. It reads whatever file is at `plan`. The clone, though, is always of the plan stage's session, which never saw the `revise` exchange. A rerun from the revised copy gives the planner a transcript that does not mention the revisions. The file is the only carrier.
- The app takes `source` and `plan` as given. It has no per-ticket rule about a second run. (`src/core/control.rs:441-455`)

**What `show` and the rerun question say today**
- The `rerun` question is built in `rerun_ask`. For a Workflow attempt, `rerun_carries` returns `""`, so it names no plan and no source. (`scheduler.rs:9335-9390`; the `fail_attempt_with` path at `7263-7266` uses the same function)
- `show` takes its plan list from `Ticket::shown_where`. It keeps `Complete` attempts and open Workflow attempts only, so a Failed `review/1` is never listed. (`dispatch/src/ticket.rs:1476-1494`, `scheduler.rs:10899-10935`)
- `print_lane_files` marks only an open review's copy as "(reviewed copy, round N, review open)". Nothing relates it to the attempt before it. (`dispatch/src/main.rs` ~895-908, per the investigation; not re-read)

**Other readers of the plan**
- Readers of "the newest plan":
  - `Ticket::input` and `input_where` serve later agent stages through `lane_input` and `lane_plan`. They require `Complete`. (`ticket.rs:1437-1462`, `scheduler.rs:10838-10858`)
  - `shown` is used by `ticket_paths` in `dispatch/src/serve.rs:570`. `input("notes")` is used at `serve.rs:587`.
- The `shown` test deliberately excludes Failed and Cancelled review copies: "A parked review's copy is not the plan going forward". Any change that makes a failed copy the plan going forward must change that test. (`ticket.rs:1850-1866`)

**What survives**
- Nothing in the tree removes a stage attempt directory on a rerun. `retire_replaced` only kills processes, marks the decision acted, and may store a note in `t.rework`. (`scheduler.rs:3578-3604`)
- `prepare_writes` deletes an artifact file before an attempt launches. It applies to the attempt's own artifacts only, and the rerun attempt has a different directory, so `review/1/joined/plan.md` is not touched. (`scheduler.rs:10140-10160`)
- The `remove_dir_all` calls in `scheduler.rs` are on `ev.dir`, the evidence directory. I found none for an attempt directory in `scheduler.rs`.
- Not read: the ticket close path. Whether closing deletes `<ticket_dir>` is not settled here; it does not matter before the rerun.
- The `revise` note survives a runner failure today in three places:
  - the `Answered{note}` decision;
  - the file `review/1/joined/plan.feedback-<n>.md`;
  - `Attempt.revisions`, which holds round, by and time, not the text. (`ticket.rs:503-515`)
- The revised plan survives only as the file at `review/1/joined/plan.md`. The attempt record holds only the path.

**A `--note` on a Workflow `rerun`**
- It has no consumer. `rerun_note` returns `None` unless the stage kind is `Agent` or `Review`, so nothing goes onto `t.rework`. `start_workflow` does not read `t.rework` either. (`scheduler.rs:8988-8999`, `3524-3528`; `start_workflow` body `7436-7510`; `scheduler.rs:10601` excludes Workflow stages from the sent-back rework path)

**Adding a field to an Attempt**
- Dispatch versions its records with `RECORD_VERSION`, currently 28. `migrate` steps one version at a time. Added fields arrive from `#[serde(default)]`, with a comment per step and a bump. A build refuses a record newer than its own. (`dispatch/src/store.rs:26`, `440-475`)
- Earlier steps, for example 3 to 4 and 4 to 5, add `Attempt` fields the same way. That is the pattern to follow for a source or "reviewed copy of" field.

**Tests to extend**
- `a_run_switchboard_no_longer_has_fails_the_attempt` at `dispatch/tests/first_slice.rs:3809` drives the failure with `env.sb().runs.clear(); env.step();`. The `at_finalize` and `revise` helpers are in the same file, so the repro is a finalize, `revise`, clear runs, step, answer `rerun`, step, and compare the `plan` the fake `WorkflowStart` received.

## Unresolved

**Is the failed copy complete when the run is gone mid-round?**
- Tried: read `start_workflow` and `respond_prompt`. The planner is told the copy path and edits it in place. Queries: none (not an outside-world question).
- Why not settled: whether a file is whole when the run vanishes mid-write needs a live or fake run with a kill at the right moment. The fake Switchboard does not write files.
- Plan builds to: the issue reports the revised file at `review/1/joined/plan.md` written at 09:28, before the failure, so a revised copy exists in that case.
  - If favourable (the copy is whole): the plan may seed the rerun from it.
  - If unfavourable (only a half-written copy exists, for example the failure came during a reviser round): the plan must still decide, from the attempt's own record, which copy a rerun takes. The record has `revisions` and the round files beside the copy to say whether a `revise` was answered and answered by whom. Do not rely on file contents alone.

**Exact lines `show` and the `rerun` question print for the repro**
- Tried: read the code (above). No run.
- Plan builds to: `rerun_carries` returns `""` for a Workflow attempt; `show` prints one `plan:` line for `review/2` only. The plan should add the new text and a first_slice test that asserts on the question text and the `show` output; it need not wait on seeing today's exact output.

## For the owner

**Did anything besides the revised plan file and the `revise` note matter on your ticket?**
- Question: after `revise`, did the planner clone's session state (what it accepted and refuted in earlier rounds) matter to you, or only the plan text?
- Tree answer: a new run always clones the plan stage's whole transcript. Neither `review/1`'s clone nor its round responses carry over. Only the files do: the revised `plan.md`, and the `feedback-<n>.md` and `response-<n>.md` beside it.
- If only the plan text mattered: seeding the rerun from the newest revised copy fixes it.
- If the session state mattered too: that cannot be fixed by seeding a file, because `CloneAllTranscript` clones the plan stage's transcript, not `review/1`'s planner. The plan must then say in the rerun question that the planner starts without the earlier rounds.
- Works in both cases: seed the rerun from the revised copy, and say in the rerun question and in `show` which copy it reviews and that the planner's earlier rounds are not carried.
