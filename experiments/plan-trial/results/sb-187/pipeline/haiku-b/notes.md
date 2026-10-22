# Issue #187: a review rerun after a failed attempt starts from the plan stage's original output

Worktree: `/Users/sbgragent/.dispatch/worktrees/c6aa808d` (branch `dispatch/187-dispatch-a-review-rerun-after-a-failed-a`, at 7fe8949 Release 0.4.1). All paths and line numbers below are in this worktree. No code was changed and no tests were run.

## Short answer

The rerun copies the plan from the newest **Complete** attempt of the plan stage. A failed review attempt is never Complete, so its copy, which holds the owner's `revise` edit, is skipped. Attempt 2 therefore copies `plan/1/plan.md`, the original. Nothing in the rerun question or in `show` says that this happened. The owner's note is still on disk, but no code path reads it for attempt 2.

## The path, step by step

### 1. The owner's `revise` (scheduler.rs:7565–7630, `revise`)

- Reads the review run's state and requires `Converged` or `AtCap` (about line 7600).
- Computes `round = view.round + 1` and writes the owner's note to `crate::report::round_file(&subject, round)` (line 7616). That is `plan.feedback-<n>.md` beside the review's copy (report.rs:156).
- Pushes `Revision { round, by, at_ms }` onto attempt 1's `revisions` (lines 7620–7625) and sends `WorkflowObject` to Switchboard.
- The reviser then writes the revised plan into the review's copy in place: `review/1/joined/plan.md`.

### 2. The run dies (scheduler.rs:7741 `poll_workflow`, 7761)

- `NO_SUCH_RUN` ("no such run", const at 8445) turns into `fail_attempt(... "review run gone: {reason}")`. The reason string is at line 7761.
- `fail_attempt_with` (about 7195–7240) sets `state = Failed`, `ended_ms` and the rerun options. It does **not** clear `artifacts`. The attempt still names `plan` → `review/1/joined/plan.md`. Its `revisions` entry is also kept.

### 3. The owner answers `rerun`

- `ask_rerun` (scheduler.rs:7308) asks through `rerun_ask` (scheduler.rs:9335). The question is `"<stage> (<ctx>) attempt <n> failed: <reason>. Run it again?"` plus `rerun_carries(...)` (line 9382) and `rewrite_choices`.
- `rerun_carries` returns text only for `Review` attempts (the code review carry, 9385–9386), and `""` for a `Workflow` attempt (line 9387). So the plan-review rerun question says nothing about which copy is reviewed or whether the revision carries.
- Once acted, the next `workflow_stage` pass (scheduler.rs:7379) calls `start_workflow` (scheduler.rs:7436).

### 4. The new attempt copies the wrong file (scheduler.rs:7436–7520, `start_workflow`)

- `review_subject(t, "plan")` (scheduler.rs:10534–10548) takes `.filter(|a| a.state == AttemptState::Complete)` over the attempts, newest first. Attempt 1 is `Failed`, so it is skipped. It returns `plan/1/plan.md` and the session of plan attempt 1.
- Line 7470–7471: `copy = review/2/joined/plan.md`, copied from `plan/1/plan.md`.
- Line 7511: `WorkflowStart { source: source_session, plan: copy, ... }`. `source_session` is the **plan** stage's session, so the new planner clones the plan stage's conversation. The revising conversation from attempt 1 is not passed.
- Attempt 2's `artifacts` is `{plan: review/2/joined/plan.md}`. Nothing carries attempt 1's `revisions` or its round file.

Consequence: round 1 of attempt 2 reviews the original plan. This matches the report's "two of attempt 2's three round-1 points were things the revised copy had already fixed".

### 5. The selection helpers that cause it

- `Ticket::input_where` (ticket.rs:1452–1465) and `Ticket::input` (ticket.rs:1437): only `Complete` attempts. Used for agent prompts. Design: docs/design.md ~2160–2170 says an agent's prompt is "handed only complete files".
- `Ticket::shown_where` (ticket.rs:1476–1493): `Complete`, or a `Workflow` attempt that is open. A failed or cancelled review copy is not shown either.
- The test that pins this: ticket.rs:1858–1866. The loop over `Cancelled` and `Failed` asserts `shown` falls back to `plan/1`, with the comment "A parked review's copy is not the plan going forward." So the current behaviour for **failed** attempts is an explicit test expectation, not an accident. Any fix must change that test on purpose.

## What `show` prints (the `files:` block)

- `main.rs:981–982`: `say!("files:")`, then `print_lane_files("plan", &p.plan_files)`.
- `serve.rs:538–593` (`ticket_paths`):
  - `plan_files` (serve.rs:539) comes from `shown_lane_files` (scheduler.rs:10900) → `lane_files_by` (scheduler.rs:10908) → `t.shown_where`. For this ticket that is the open attempt 2 copy, with `reviewing: true` and `round: None`.
  - Attempt 1's copy is not listed at all, so `show` never says it is a prior revision.
  - `review` (serve.rs:564–568) is the newest workflow attempt with artifacts, which is attempt 2. `plan_rounds` (serve.rs:572) is computed only for that attempt. Attempt 1's owner round (`plan.feedback-1.md`, `by` set) is therefore not printed. The `plan round N (owner)` lines (main.rs:991–993) cannot show it.
- Result: `show` lists `review/1 joined failed` and `review/2 joined running` in attempts, and the `files:` plan line names only review/2. That matches the issue's report.

## On disk and on rehydration (before the pane)

Record:
- `<root>/tickets/<id>.json` (store.rs:134–135). Attempts carry `stage`, `n`, `context`, `state`, `artifacts`, `session`, `revisions` (`ticket.rs:350`), `forgotten`, `run`.
- Attempt directory: `<ticket dir>/<stage>/<n>/<ctx>/` (scheduler.rs:6303–6317, `attempt_dir`). The review copy is `<attempt dir>/plan.md`. Round and response files sit beside it (report.rs:156 `round_file`, report.rs:202 `response_file`).
- Switchboard's run snapshots live under `<app data>/workflows/<run>/round-<n>/` (src/core/workflow.rs:43). I did not check whether they survive a restart or whether `no such run` clears them.

Rehydration on rerun: the record is read, the latest `Complete` plan is chosen, and the review copy is copied fresh from it (steps 4 above). The on-disk `review/1/joined/plan.md` and `plan.feedback-1.md` are not read again by anything.

Pane (what the owner sees, not verified in Switchboard):
- The review card shows the planner "face" (scheduler.rs:7741 `poll_workflow` sets `session` to the run's planner, or the reviewer if none).
- For attempt 2, the planner conversation is a clone of the plan stage's session (scheduler.rs:7511 `source`). The conversation in which the revision was written is attempt 1's session and is not carried. The reviewer reads the original plan copy.
- I did not read the Switchboard-side clone code (`src/` workflow handling) to confirm how `WorkflowStart { source }` clones. That is the one link in the chain I inferred rather than read.

## Where the design stands

- docs/design.md ~1657–1665 and ~1790–1800: a failed plan review gets the ordinary rerun question. A rerun "kills the old run's agents and starts a fresh run". The design does not say which copy the fresh run reviews.
- docs/design.md ~1790–1800: "Review rounds carry forward" applies to **code review** attempts only (`carried_from`). Plan reviews have no carry. `rerun_carries` (scheduler.rs:9385) matches that.
- docs/design.md ~2160–2175: the plan section says agents get only complete files. That is the rule `input_where` enforces, but the design does not say whether a review's own copy should count as "the plan the stage holds" after a failure.

So the design leaves this open. The issue's expectation (carry the newest plan) is a behaviour change, not a bug in a documented rule.

## Options (none implemented)

A. **Reviewed copy after a failed plan review** (narrowest, addresses the report directly). In `review_subject` (scheduler.rs:10534), also accept a `Failed` or `Cancelled` `Workflow` attempt for the same subject when it has an owner revision (`revisions` non-empty) whose `response_file` exists (report.rs:202), i.e. the reviser finished. Otherwise keep `plan/1`. Also take the planner session from that attempt rather than plan/1, if its `session` survives failure (not checked). Update the test at ticket.rs:1858–1866, which pins the failed case. Risk: a run that died mid-edit may leave a partly written copy. I did not check whether the reviser's write is atomic. Requiring a response file limits this to settled revisions.

B. **Say which copy will be reviewed.** Extend `rerun_carries` (scheduler.rs:9382–9390) with a `Workflow` clause naming the copy, for example "The next attempt reviews the plan at <path>; this attempt's revised copy is not carried." This is a small, safe change to the question text. Ask whether a test pins the question wording (not checked).

C. **`show` labels and plan rounds.** In `ticket_paths` (serve.rs:538–593), list each workflow attempt's copy, labelling the one reviewed and the earlier ones as prior revisions, and print `plan_rounds` for every workflow attempt (serve.rs:564–572), not only the newest. The `LaneFile` view type may need a label field. It is in the shared control crate and was not read, so the change is additive but its size is unverified.

D. **Owner note survival.** The note is already on disk (`plan.feedback-1.md`) and in `revisions`. Under A the revised copy carries it, since the reviser folded it into the plan. Passing the old round file to a fresh Switchboard run as a first round would need a new Switchboard operation: `workflow.object` needs a converged run, and a fresh run has none. I would not do that.

Recommendation: A together with B, plus the test change in A. C is a separate `show` improvement. A is a behaviour change, so it needs the owner's decision on one point:

- **Decision for the owner:** should a failed plan review carry only settled owner revisions (narrow, as in A), or every in-place edit the planner made before the failure (broader, and the same loss happens without an owner `revise`)? The design is silent. I recommend the narrow version first.

## Not verified

- Switchboard's `WorkflowStart { source }` clone behaviour and the pane contents.
- Whether `session` survives `fail_attempt`.
- Whether the reviser's write to the copy is atomic.
- Whether Switchboard run snapshots (`workflows/<run>/round-<n>/`) survive a restart.
- Whether any existing test pins the rerun question wording.
- The `LaneFile` view type and its size.
