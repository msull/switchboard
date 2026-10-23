# Issue #187 — investigation notes

Tree: `/Users/sully/code_repos/plan-trial/sb-187-stage-investigate-v5lean-haiku-c`
(branch `stage/sb-187-investigate-v5lean-haiku-c`, HEAD `7fe8949`, clean).

Read first: `CLAUDE.md` (the Switchboard project file; its "Working in this repo"
and "Architecture" sections are the binding ones), `docs/design.md` (the
"Plan review workflow" section at 1000–1211, "Asking the owner" 2632, the Dispatch
ticket page at 2135–2175, the Dispatch runner section near 1784–1790), and
`docs/lessons.md` (a Prompt Box retrospective; nothing in it bears on this issue).
The Dispatch spec is `docs/dispatch.md`; its rerun and carry rules are at
1690–1712 (code reviews only) and 318–350.

The issue's code lives in the `dispatch` crate (`dispatch/src/`) and the
Switchboard app's workflow core (`src/core/workflow.rs`, `src/core/control.rs`,
`src/app.rs`). Nothing was run, built or reproduced.

---

## 1. The mechanism, in the order it happens

### 1a. Finalize and `revise`

- `finalize` is asked by `review_done` (`dispatch/src/scheduler.rs:7652–7703`).
  Its question names the reviewed copy from the review attempt's artifacts
  (7681–7696). Options are `finalize | revise | park` (7697).
- `revise` is `scheduler.rs:7565–7647`. It writes the owner's note as a round
  file beside the reviewed copy, `round_file(&subject, round)` (7616–7618;
  naming at `dispatch/src/report.rs:156–158` and `beside` at 208). The body is
  `OWNER_ROUND_HEADING` plus the note (`dispatch/src/lib.rs:96`,
  `scheduler.rs:7617`). It pushes a `Revision { round, by, at_ms }` onto the
  review attempt (7619–7626; struct at `dispatch/src/ticket.rs:510–516`, field
  at 346–350). Then it sends `workflow.object` (7627–7634). The app accepts only
  when the run is `Converged` or `AtCap` (`scheduler.rs:7608`, mirrored by the
  app in `src/core/workflow.rs`). The planner then revises the reviewed copy in
  place. The reviewed copy is the file the attempt's artifacts name.

### 1b. The review run fails

- `poll_workflow` (`scheduler.rs:7741–7830`) asks the app for the run. A
  `Reply::Failed` with `NO_SUCH_RUN` (`scheduler.rs:8445`, the constant is
  `"no such run"`) goes to `fail_attempt(..., "review run gone: no such run")`
  (7755–7764).
- The app side that produces `no such run` is `src/app.rs:3916–3919`: it calls
  `core.run_view(WorkflowId)` (`src/core/control.rs:865`), which is
  `self.workflow(id)` (`src/core/workflow.rs:222–224`), which searches
  `self.workspaces[*].workflows` (`src/core/workflow.rs:217–219`). So "no such
  run" means the run is absent from the in-memory core state. Note that
  `src/app.rs:1895` is a different `no such run` (an output log), not this one.
- `fail_attempt` → `fail_attempt_with` (`scheduler.rs:7148–7158`,
  `7195–7230`). It sets the attempt's state to `Failed { reason }` and keeps
  `artifacts` untouched (only `state`, `ended_ms`, `failed_at_*` change, 7208–7215).
  So after the failure the review attempt still names
  `review/1/joined/plan.md` as its `plan` artifact, and it still holds the
  revision.
- The rerun question is `ask_rerun` (`scheduler.rs:7308–7332`), whose text
  comes from `rerun_ask` (`scheduler.rs:9335–9377`). The question reads
  `"<stage> (<ctx>) attempt <n> failed: review run gone: no such run. Run it
  again?"`. Options are `rerun | park` (9348–9354). `rerun_carries` adds text
  only for `AttemptKind::Review` (9382–9390); a `Workflow` attempt gets none. So
  the question names no copy and says nothing about the revision.

### 1c. The rerun picks its input

- `workflow_stage` (`scheduler.rs:7379`) relaunches a failed attempt through
  `start_workflow` when `may_rerun` allows (`scheduler.rs:10665–10669`, which
  needs an acted `rerun` decision for that attempt).
- `start_workflow` (`scheduler.rs:7436–7530`) takes its source from
  `review_subject(t, &subject)` (7465). That function is at `scheduler.rs:10534–10548`.
  It walks attempts newest first, **keeping only `AttemptState::Complete`**
  (10539), and returns the first artifact named `plan`. Review/1 is `Failed`, so
  it is skipped. The answer is `plan/1/joined/plan.md`, and the session is the
  plan attempt's session (10546).
- It then copies that file into the new attempt's directory (`dir.join("plan.md")`,
  7469–7472), pushes the new attempt with `{plan: copy}` (7487–7495), and sends
  `WorkflowStart { source, plan: copy, ... }` (7510–7514). The app starts a fresh
  run from the unrevised copy. Nothing in this path reads review/1's artifacts,
  its `revisions`, or its round file.
- The same rule appears in `Ticket::input_where` (`dispatch/src/ticket.rs:1452–1463`,
  Complete filter at 1460). `Ticket::input` (1437) and `input_with_stage` (1444)
  wrap it. The ticket test at `ticket.rs:1797–1824` pins "the latest completed
  attempt that wrote them".
- The `rerun` answer's own note is not carried for a workflow stage. `rerun_note`
  (`scheduler.rs:8988–`) returns `None` unless the stage's kind is `Agent` or
  `Review` (`pipeline.rs:684–690`, the enum; `StageKind::Workflow` is
  what a `review` stage is, `pipeline.rs:715–724`). The `revise` note is not
  read by `rerun_note` either.

### 1d. The existing test that looks related but is not this case

`dispatch/src/ticket.rs:1866–1868` says "A plan stage rerun after a finished
review writes the newer file." That is the rule for a new `plan` attempt (a
plan rerun), not for a review rerun. The issue's case never reaches it.

---

## 2. What `show` says (`files:`)

- `show` prints the `files:` block at `dispatch/src/main.rs:981–998`, from
  `view.paths` built in `dispatch/src/serve.rs:538–594` (`ticket_paths`).
- `plan_files` (serve.rs:539–548) comes from `shown_lane_files(t, p, "plan")`
  (`scheduler.rs:10900–10906`, `lane_files_by` at 10908). It uses
  `Ticket::shown_where` (`ticket.rs:1476–1491`), which accepts `Complete`
  attempts and open `Workflow` attempts (1486–1488). So after the failure and
  before the rerun, `show` names `plan/1/joined/plan.md` as the plan (no review
  flag). The revised copy is not listed at all. After the rerun it names
  `review/2/joined/plan.md` as `(reviewed copy, review open)`
  (`print_lane_files`, `main.rs:896–909`).
- `plan round N` lines (`main.rs:991–994`) come from `plan_rounds(review)`,
  where `review` is the **last** `Workflow` attempt with artifacts
  (`serve.rs:568–569`), which is review/2. Review/1's `revise` round (its round
  file and its `Revision`) is never listed. Its `(owner)` tag does not appear.
- `round` (`main.rs:983–989`) is `last_round` or `plan_rounds.last()`, both from
  the last workflow attempt. Review/1's round is not reachable from `show`.
- The `revisions` field is read by `plan_rounds` (serve.rs:514–518) and
  `review_round` (526–533). Neither is used on the rerun path.
- Conclusion: `show` names only the attempt-2 copy as the plan, as the issue says.
  Once review/2 exists, nothing in `show` mentions review/1's revised copy.

---

## 3. The on-disk record and what rehydrates, before the pane

**On disk, after the repro (tree-verified by reading the code, not by running it):**

- Ticket record: `attempts[]` holds `review-plan/1 joined` with
  `state: failed, reason: "review run gone: no such run"`,
  `artifacts: { plan: <ticket dir>/review-plan/1/joined/plan.md }`,
  `revisions: [{round, by, at_ms}]`, and `run` set to the lost run's id.
  `attempts[]` also holds `plan/1 joined` (Complete, `plan/1/joined/plan.md`,
  session = the planner).
- Decisions: `finalize` answered `revise` with the note (and `by`); a `rerun`
  decision (`attempt: (review-plan, 1)`), pending until answered.
- Files: `review/1/joined/plan.md` (the revised copy, written by the planner);
  `review/1/joined/plan.feedback-<n>.md` (the owner's note under
  "# The owner's objection"). `<n>` is the round number from
  `view.round + 1` at the app's run.
- App side: the workspace record's `workflows[]` either holds the run or does
  not. The issue's "no such run" means it did not, at the moment of the query.

**Rehydration of the rerun:**

- `input_where` (`ticket.rs:1452–1463`) skips review/1 and returns `plan/1`.
- `start_workflow` (`scheduler.rs:7465–7472`) copies `plan/1/joined/plan.md` into
  `review/2/joined/plan.md` (a new file, same bytes as plan/1).
- The rerun's fresh run is started with `plan: review/2/...` and source = the plan
  session. The planner clone, the reviewer, and the app-side round state are all new.

**What the pane shows, after the rerun:**

- Ticket page, Plan tab (`docs/design.md:2161–2175` describes it;
  `serve.rs` `PlanRoundView` feeds it): the plan as `Ticket::shown`
  gives it, which is review/2's copy labelled "Reviewed copy, round N: review
  open". Plan rounds fold only review/2's rounds. Review/1's revision is absent.
- `show` (`main.rs:981–998`): `plan (joined): .../review/2/joined/plan.md`
  with `(reviewed copy, review open)`. No mention of review/1.
- The rerun question's text (`scheduler.rs:9368–9375`) is the same whichever
  copy is reviewed. It does not name a copy.

**Before the rerun** (between the failure and the answer): `show` names
`plan/1/joined/plan.md` as the plan. The revision is invisible, though the file
exists.

---

## 4. The issue's claims, and whether the tree settles them

The tree settles these (verified by reading the code, not by running it):

- The rerun copies `plan/1/joined/plan.md` into `review/2/joined/plan.md`
  (`scheduler.rs:10534–10548`, `7465–7472`). The issue's observed
  `review/2` is consistent with this.
- `show` names only review/2 after the rerun (`serve.rs:568–571`, `main.rs:981–998`).
- The revision is not carried by any code path: no reader of `Revision`, the
  round file, or `review/1`'s artifacts runs on the rerun path.
- The `rerun` question does not name the copy (`scheduler.rs:9335–9377`).
- The `revise` note is written to disk before the send, and popped with the file if
  the app refuses (`scheduler.rs:7615–7645`).

The tree does NOT settle these; they are claims about the world:

- The runner restart is the cause of `no such run`. In the tree, `no such run`
  comes from the Switchboard app's core state (`src/app.rs:3918`,
  `src/core/workflow.rs:217–224`), not from the Dispatch runner. Dispatch sends
  `workflow.remove` nowhere (grep of `dispatch/`, `dispatch-control/`, `src/ui`
  finds no sender). See Open questions.
- The reviser wrote the revision to `review/1/joined/plan.md` at 09:28.
- Two of attempt 2's three round-1 points were already fixed in the revised copy.
- The finalize decision for attempt 2 lists every question as open.

---

## 5. What was not checked

- `dispatch/src/restart.rs` and `dispatch/src/recover.rs`: whether a runner
  restart touches workflow attempts, their artifacts, or their rounds. Not read.
- `src/core/workflow.rs` `clean_up` (dispatched at 176; the body was not read):
  whether it deletes the reviewed copy or only the round files.
- `src/core/workflow.rs` `WorkflowStart` handling: whether it validates the
  `plan` path (for example, must it lie under the project or the attempt
  directory?) and how it clones the planner from `source`.
- `dispatch/tests/support/mod.rs:690`, the fake's `no such run`. Not read.
- Whether `Attempt.revisions` survives the store's round trip. Not read.
- No test was run, no build, no reproduction.

---

## Open questions

- [tree] Does the app's `WorkflowStart` accept a `plan` path outside the new attempt's directory (for example review/1's copy), or does it need a copy it makes itself? Read `src/core/workflow.rs` `WorkflowStart` handling and the port adapter.
- [tree] Does `clean_up` in `src/core/workflow.rs` (dispatched at line 176) delete the reviewed copy, or only the round files it recorded? If the copy, a cleaned-up review/1 could not be the rerun's input at all.
- [tree] Does `restart.rs` or `recover.rs` change a workflow attempt's `artifacts`, `state` or `revisions` on a runner restart? Not read.
- [run] Why does the app answer `no such run` for review/1's run after a runner restart? Is the run absent from `workspaces[].workflows` after reload (the store's read, `src/adapters/store.rs`), or was it removed by `remove_workflow` (`src/core/workflow.rs:1009–1025`)? Reproduce with the repro steps and read the store file before and after.
- [run] Does `src/core/workflow.rs`'s `remove_workflow` ever run in the repro? Dispatch never sends `workflow.remove`, so the only sender is an external client of the control port.
- [owner] Did anyone, the owner or a script, send `workflow.remove` or run `Clean up` on the run between 09:28 and the failure?
- [owner] Does the owner's ticket on disk match the issue's description: review/1's plan holds the revision, review/2's plan is byte-identical to plan/1's, and `show` lists only review/2? Diff the files and capture `show` before the rerun answer.
- [owner] Were the two round-1 points that the revision had already fixed really fixed in the revised copy? Compare the findings against the revised file.
- [source] If the rerun resumes the planner clone (`claude --resume <session id>`) instead of starting from the source session: does Claude Code keep a transcript after its session file is lost or expires? Search: "claude code --resume session transcript cleanup".
- [source] If the rerun resumes a reviewer through Codex (`codex resume <id> "<prompt>"`, which design.md:1173–1174 lists as unverified): does Codex accept a resume for a rollout that no longer exists? Search: "codex resume session id prompt rollout missing".
