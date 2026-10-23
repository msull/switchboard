# Issue #187 — investigation notes

Worktree: `/Users/sully/code_repos/plan-trial/sb-187-stage-investigate-v5lean-haiku-b`
(branch `stage/sb-187-investigate-v5lean-haiku-b`, clean at start). Nothing was
edited, built, or run. All line numbers are as they are in this worktree.

## What the code does, in one paragraph

A plan review is a `StageKind::Workflow` stage (`dispatch/src/pipeline.rs:684-724`).
Each of its attempts is an `Attempt` with `kind == Workflow` whose `artifacts`
map holds the reviewed copy under the subject name (`plan`). The copy lives in
the attempt's directory, `<attempt dir>/plan.md` (e.g. `review/1/joined/plan.md`),
and is a fresh `std::fs::copy` of the newest *complete* plan. A rerun of a failed
attempt asks `review_subject` for its input, and `review_subject` only looks at
`Complete` attempts. So a failed attempt's copy, even one the owner revised, is
never the input of the next attempt. The next attempt copies `plan/1/...` again.

## Chain of events, mapped to code

1. **Finalize → `revise`.** The `finalize` decision is answered `revise` with a
   note. `workflow_stage`/`review` reaches `("finalize", "revise")` at
   `dispatch/src/scheduler.rs:3495-3497`, which calls `revise()`.
   - `revise()` is at `dispatch/src/scheduler.rs:7565-7636`.
   - It writes the owner's note to the round feedback file *beside the reviewed
     copy*: `round_file(&subject, round)` at `:7616-7618`. For the repro that is
     `review/1/joined/plan.feedback-<n>.md`, with `n = run.round + 1`.
   - It pushes a `Revision { round, by, at_ms }` onto the **failed-to-be attempt**
     `review/1` at `:7620-7624`. The note text is *not* stored on the ticket
     record; `Revision` is `dispatch/src/ticket.rs:510-516` and holds only
     `round`, `by`, `at_ms`.
   - It sends `Body::WorkflowObject { run, round, text }` (`:7627-7634`), which the
     Switchboard app turns into `ObjectWorkflow` (`src/core/control.rs:1399-1406`,
     `src/core/workflow.rs:969-1000`). The app side keeps the text in the run's
     `user_feedback`. A refused send pops the revision in `apply_reply`
     (`dispatch/src/scheduler.rs:11238`, pop at `:11313`) and `revise()` removes the
     feedback file (`:7636-7642` region).
   - The reviser (the run's planner clone) edits the copy in place at
     `review/1/joined/plan.md`. The prompt tells it to edit "that file and only
     that file" (`docs/dispatch.md:1150`, `respond`).

2. **Runner failure → `review run gone`.** `poll_workflow` (`dispatch/src/scheduler.rs:7741`)
   asks the app for the run; `Reply::Failed { reason: NO_SUCH_RUN }`
   (`NO_SUCH_RUN` at `:8445`) calls `fail_attempt` with
   `"review run gone: no such run"` at `:7757-7763`. The attempt goes to
   `AttemptState::Failed`, with `ended_ms` set (`fail_attempt_with`, ~`:7198-7240`).
   The app side answers `no such run` from `core.run_view(WorkflowId)` returning
   `None` (`src/app.rs:3915-3919`, `src/core/control.rs:865`).

3. **Rerun question.** `workflow_stage` (`dispatch/src/scheduler.rs:7378-7424`)
   sees the latest attempt is not open and not complete. It calls `ask_rerun`
   (`:7308-7332`), with question text from `rerun_ask` (`:9335-9379`).
   - For a Workflow attempt `rerun_carries` returns `""` (`:9382-9392`). It only
     adds a "next attempt carries…" sentence for `AttemptKind::Review`. So the
     question says nothing about which copy will be reviewed or about the
     revision.
   - `rerun_note` (`:8988-9010`) returns `None` for any stage that is not
     `Agent | Review` (`:8996-8999`), so a `rerun --note` answer on a plan review
     is not carried into `t.rework`.
   - `sent_back` (`:10601`) states "A workflow stage takes no note."

4. **Rerun answered → new attempt copies the wrong file.** The `("rerun", "rerun")`
   arm (`:3524-3527`) runs `retire_replaced` (`:3566-3601`), which marks the
   decision acted. Then `workflow_stage` sees `may_rerun` (`:7402`, defined
   `:10665-10669`) and calls `start_workflow` with `next_n` (`:7403-7412`).
   - `start_workflow` (`:7436`) calls `review_subject(t, "plan")` at `:7465`.
   - `review_subject` (`dispatch/src/scheduler.rs:10534-10548`):
     ```
     t.attempts.iter().rev()
         .filter(|a| a.state == AttemptState::Complete)   // <- skips review/1 (Failed)
         .find_map(|a| a.artifacts.get(subject)...)
     ```
     It returns `(path, session)` of the newest complete attempt that wrote
     `plan`, which is `plan/1/joined/plan.md` and plan/1's session.
   - The copy is `dir.join("plan.md")` (`:7458-7461`), `fs::copy` from
     `source_path` at `:7471`.
   - `Body::WorkflowStart { source: source_session, plan: copy, ... }` is sent at
     `:7510`. The app's `start_workflow` (`src/core/workflow.rs:289`) clones
     **the source session** (plan/1's planner) into a new planner, plus a new
     reviewer. It does not clone review/1's planner, which is the session that
     received the owner's note.

5. **Readers that also skip the revised copy.** The same `Complete`-only rule
   appears in the prompt path and in the display path:
   - `Ticket::input_where` (`dispatch/src/ticket.rs:1452-1463`) — `Complete` only.
     Used by `lane_input` (`dispatch/src/scheduler.rs:10849`) and so by the
     next stage's `plan` (`lane_plan` `:10859` area).
   - `Ticket::shown_where` (`dispatch/src/ticket.rs:1476-1491`) — `Complete`, or a
     Workflow attempt that is open. Failed or Cancelled workflow attempts are
     excluded. Used by `ticket_paths` (`dispatch/src/serve.rs:570`) and by
     `lane_files_by` with `shown=true` (`dispatch/src/scheduler.rs:10899-10925`).

## What the on-disk record looks like

Ticket record (JSON, Switchboard data directory; `docs/dispatch.md:393-396`,
schema version 19 adds `attempts[].revisions`):

- `attempts[]` for the plan review, in order:
  - `{stage: "<plan review stage>", n: 1, context: "joined", kind: "workflow",
    state: {failed: {reason: "review run gone: no such run"}}, artifacts:
    {plan: ".../review/1/joined/plan.md"}, revisions: [{round: N, by: "you"|"supervisor", at_ms}], run: <run id>, session: <planner id>, ended_ms}`
  - `{stage, n: 2, ...}` appended by the rerun, `state: starting|running`,
    `artifacts: {plan: ".../review/2/joined/plan.md"}`.
  - `{stage: "plan", n: 1, state: complete, artifacts: {plan: ".../plan/1/joined/plan.md"}, session: <original planner>}`.
- `decisions[]`: the `finalize` decision (answered `revise`, its note on the
  decision), the `rerun` question for attempt 1 (answered `rerun`, acted), and
  the rerun's own question if any.
- Files on disk (attempt dirs under the ticket's run area; exact root is
  `attempt_dir`, `dispatch/src/scheduler.rs:6303`):
  - `review/1/joined/plan.md` — the revised plan (the revision is here).
  - `review/1/joined/plan.feedback-<N>.md` — **the owner's note**, written by
    `revise()` (`:7616-7618`). Its existence is the only durable copy of the note
    text in the dispatch side (the app keeps it in the run's `user_feedback`,
    which died with the run).
  - `review/2/joined/plan.md` — fresh copy of `plan/1/joined/plan.md`.
  - `plan/1/joined/plan.md` — the original plan.
- The app-side `WorkflowRun` (`src/core/…`, design `docs/design.md:1015-1022`)
  holds rounds, `user_feedback`, and state; `NO_SUCH_RUN` says that record is
  gone or not found.

## What the pane (and `show`) look like

`show` (`dispatch/src/main.rs:~930-993`, `files:` block at `:981-993`):
- `print_lane_files("plan", p.plan_files)` (`:896-907`). `plan_files` comes from
  `shown_lane_files(t, p, "plan")` (`dispatch/src/serve.rs:539-546`). The
  `reviewing` flag is `kind == Workflow && is_open()`, so only the open
  attempt-2 copy gets "(reviewed copy, review open)". There is no round
  number because `review_round` (`serve.rs:529-535`) reads round files beside
  the *latest* workflow attempt's copy, which has none yet.
- `plan round N (owner)` lines come from `p.plan_rounds` (`serve.rs:508-525`,
  built at `:572`). That is taken from the *latest* workflow attempt with
  artifacts (`serve.rs:563-568`, `review` = review/2 once it exists), so
  `review/1/…/plan.feedback-N.md` and its `(owner)` marker are not listed.
  Before attempt 2 starts, `review` = review/1 and the owner round *is* listed.
- `show`'s attempts list shows `review/1 joined failed` and `review/2 joined
  running`; the `files:` plan line names `review/2/…/plan.md` only.

App Plan tab (what the user sees on the ticket page):
- `plan_label` (`src/ui/ticket.rs:425-432`) turns `PathsView` into the label:
  `Reviewed copy, round N: review open` / `Reviewed copy: review open` /
  `Reviewed copy` / `From stage <stage>`. The `PathsView` comes from the same
  `ticket_paths` (`dispatch/src/serve.rs:75`, `:1396`…).
- With attempt 2 open the label reads "Reviewed copy: review open" over
  plan/1's text. Between failure and rerun, `shown` is plan/1 Complete, so the
  label reads "From stage plan" and the body is the original plan.
- Round folds under the plan (`src/ui/ticket.rs` `plan()` ~`:434+`) follow
  `paths.plan_rounds`, so the owner's round disappears from the pane once
  attempt 2 is created.

Consistency of the two surfaces: the pane and `show` both use `ticket_paths`
(`serve.rs`), so a fix in `shown`/`ticket_paths` changes both. The rerun
question text is built separately in `rerun_ask` (`scheduler.rs:9335`), so it
would need its own change.

## Related paths that hit the same rule (verified in code)

- **Park then resume.** `park` cancels every open attempt via `cancel_attempt`
  (`dispatch/src/scheduler.rs:2656-2657`, `:2816`). A plan review run open
  at park becomes `Cancelled { reason }`. `resume` then asks `rerun` for it
  (`rerun_ask` has a `Cancelled` arm, `:9363-9368`), and the rerun copies plan/1
  by the same `review_subject` rule. So the same loss happens on park/resume,
  not only on a runner failure.
- **Close.** `cancel_open_attempts` (`:2893`) is used on close (`:2072`); the
  tree is removed, so nothing reruns there.
- **Restart discard.** `restart.rs:840-880` marks `Complete` attempts in the
  restarted range `Cancelled`. A discarded plan review then also drops out of
  `review_subject`. This is probably intended (a restart from plan re-runs
  plan), but it is the same predicate.
- **Owner's `rerun --note` on a plan review** is dropped from `t.rework`
  (`rerun_note` `:8996-8999`), but it remains on the decision record.

## Facts from design and lessons docs

- `docs/design.md:1000-1030` — plan review workflow: the run is a durable
  record with rounds, state, and `user_feedback` in the app store.
- `docs/design.md:1072-1082` — round files live beside the plan (`<plan stem>.feedback-<n>.md`),
  copied into the data dir at end of each round. This is why the owner's note
  survives on disk even when the run is gone.
- `docs/design.md:1186-1200` — `workflow.object {run, round, text}` is refused
  unless the run is `Converged`/`AtCap`, and `round` must be count+1 so the
  feedback file and the opened round match.
- `docs/design.md:1072-1082` also says each completed round is snapshotted into the data
  dir. I did not check whether a round with an owner's `revise` (or a round
  that never completed) is snapshotted, so the revised copy's durability is
  not settled here; see the open questions.
- `docs/design.md:2158-2172` — the Plan tab contract: "an open review's copy
  labelled with its round, else the newest complete plan labelled with the
  stage that wrote it; an agent's prompt is still handed only complete files
  (`Ticket::shown` against `Ticket::input`)".
- `docs/dispatch.md:393-396` — `revisions` schema v19.
- `docs/lessons.md` is about the Prompt Box predecessor and has nothing on
  this path. Not used.
- `CLAUDE.md` rules that apply to a fix: no tool attribution in commits/PRs;
  record changes need a `SCHEMA_VERSION`/`migrate` step if a serialized type
  changes (`dispatch/src/store.rs:555` shows the version history); a fix in
  `dispatch/` must keep `cargo clippy` clean on Linux when it touches `cfg`.

## Repro reading

Steps 1–4 of the issue match the code path above exactly. Step 4's diff
(`review/1/joined/plan.md` vs `review/2/joined/plan.md`) should show the
revision missing, because `review/2` is a fresh copy of `plan/1`. I did not run
it.

---

## Open questions

- [tree] Does any code remove or overwrite a failed or cancelled workflow attempt's directory (`review/1/joined/`), on rerun, park, resume or restart cleanup? A fix that reads that copy depends on it surviving. Look for `remove_dir`, `remove_file` and attempt-dir cleanup in `dispatch/src/` around `attempt_dir` (`scheduler.rs:6303`) and `restart.rs`.
- [tree] Is a revised reviewed copy (and its feedback file) snapshotted into the data dir the way `docs/design.md:1072-1082` says completed rounds are? Search for the snapshot step (`RoundSnapshotted`, `round-<n>`) and whether it runs before a run is lost.
- [tree] Which other readers of `input_where` / `shown_where` / `review_subject` for the `plan` name would change with a rule that counts a failed review's copy: `lane_input`/`lane_plan`/`lane_plans` prompts (`scheduler.rs:10849`, `:10859` area), `lane_files_by` (`:10899-10925`), `ticket_paths` (`serve.rs:539-575`), and `report.rs` `plan_review` (`:355-380`). The plan must list them all.
- [tree] Is `Attempt.revisions` non-empty on a failed attempt a reliable "owner revised this copy" marker? `apply_reply` pops it on refusal (`scheduler.rs:11238`, pop `:11313`). Confirm the pop covers every refusal path (including `NO_ANSWER`, which keeps it), and whether a revision can exist for a round the run never received.
- [tree] What does `show --json` (`dispatch/src/main.rs` show path, `serve.rs` `LaneFile`) expose today, and would a new field on `LaneFile`/`PathsView` need a wire change in `dispatch-control` (additive per CLAUDE.md)?
- [tree] Which attempt-level state (`failed_at_checks`, `failed_at_rewrite`, `rework`) does a plan review rerun already read? Does any of it need the rerun's question text to differ for `Workflow`, given `rerun_carries` only special-cases `AttemptKind::Review` (`scheduler.rs:9382-9392`)?
- [run] Why does the app answer `no such run` on this run: is the run removed from the store, not reloaded at app start, or pruned? Check `run_view` (`src/core/control.rs:865`) and the store load of `workflows` (`src/adapters/store.rs` around `:1082`). The answer decides whether a rerun is the only recovery or whether the run could be re-attached.
- [run] Does `review/1/joined/plan.md` hold the complete revised text after a kill mid-revision (the reviser may have been writing when the run died)? Reproduce with the repro steps and diff against `plan/1`.
- [run] Does a rerun clone of plan/1's planner session (the `source` in `start_workflow`, `src/core/workflow.rs:289`) start without the owner's note and without the reviser's edits? The note went only to the reviser's run (`ObjectWorkflow`, `src/core/workflow.rs:969-1000`), so the expected answer is yes; confirm in a live run.
- [owner] Does the issue's diff hold: attempt 2's round-1 points include two that the revised copy already fixed, and attempt 2's finalize contradicts the owner's answer? Check against `review/1/joined/plan.md`, `review/2/joined/plan.md`, and the feedback file `review/1/joined/plan.feedback-<N>.md`.
- [owner] Does the owner's `revise` note in `review/1/joined/plan.feedback-<N>.md` read the way the issue quotes it, and is the revised plan the one the owner approved?
- [source] Claude Code session fork: does resuming a session with a fork flag (search: "claude code resume fork session") copy the transcript up to the fork point, and what happens when the source session was killed mid-turn? This bears on whether a rerun's planner clone can ever see the revision by transcript rather than by file.

## What I did not check

- I did not build, run the tests, or run a live repro. Everything above is read from source and docs.
- I did not read the app-side workflow clone code (`workflow_cloned`, `src/core/workflow.rs`) beyond `start_workflow`.
- I did not read the `poll`/`recover` paths in `dispatch/src/recover.rs` for a failed workflow attempt, or `restart.rs` beyond the discard loop.
- I did not check the repo's `tests/` for an existing test pinning a failed review's copy as ignored. `ticket.rs:1840-1900` and `scheduler.rs:12838-12870` pin only the complete and open cases.
