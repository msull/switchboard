You are scoring one implementation plan for a GitHub issue, blind: you do not know who or what wrote it, and you score this plan on its own.
The repository at the issue's base commit is checked out at /Users/sully/code_repos/plan-trial/sb-187-base (read-only; change nothing there). Every file path in the plan is relative to that tree.

Score by the rubric, column by column. Length is not thoroughness. For grounding, take the first five distinct file or line references the plan makes, in order of appearance, open each in the tree, and say what you found. For departures and decisions, use the reference as the yardstick; where you disagree with the reference, say so in the evidence rather than silently scoring your own view.

Your final message must be exactly this shape and nothing else:

```
| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| v5lean-sonnet-a | <0-3> | <0-3> | <0-3> | <0-3> | <yes|note|no> |
```

## Grounding
- <ref 1 as the plan wrote it>: found / wrong (<what is there>)
- ... five rows
## Departures caught
- <each departure in the reference>: caught / missed / caught with a wrong fix
- <any departure the plan found that the reference lacks>
## Decisions to overturn
- <each call you would reverse, with the reference decision it conflicts with>; or none
## Over-specification
- <what an implementer would not need, and roughly what fraction of the plan it is>
## Hand-off
- <the one line of guidance if note; why not if no>
## Notes
- best: <one line>
- worst: <one line>

---

# The issue

# #187: Dispatch: a review rerun after a failed attempt starts from the plan stage's original output, discarding the owner's revise

## What happened

A plan ticket on a client project reached the review stage's finalize decision and the owner answered `revise` with a note of lead decisions. The reviser wrote the revised plan to `review/1/joined/plan.md` (09:28). The review run then failed with `review run gone: no such run`, and the rerun decision was answered `rerun`.

Attempt 2 started one minute later from `plan/1/joined/plan.md`, the plan stage's original output, not from the revised copy in attempt 1. Its rounds converged and the finalize decision came back for a plan that still lists every question as open and contradicts one of the owner's answers. Two of attempt 2's three round-1 points were things the revised copy had already fixed.

Nothing warned that the revision had been discarded. `show` lists `review/1 joined failed` and `review/2 joined running`, and the files line names only the attempt-2 copy as the plan.

## Expected

A rerun after a failed review attempt should start from the newest plan the stage holds, which after a `revise` is the previous attempt's revised copy, or at least say in the rerun question and in `show` which copy it will review. The `revise` note is owner input and should survive a runner failure the same way an answered decision does.

## Where to look

- the review stage's rerun path: what it takes as the plan input for a new attempt;
- `show`'s `files:` block: when more than one attempt has a `plan.md`, say which is the reviewed copy and which is a prior revision.

## Repro

1. Take an issue through plan and review; at finalize answer `revise --note ...`.
2. Kill the review run so the attempt fails with `review run gone` (a runner restart mid-run does it).
3. Answer `rerun`.
4. Diff `review/1/joined/plan.md` against `review/2/joined/plan.md`: the revision is gone.


---

# The rubric

# Scoring a plan

Score blind: the twelve plans of an issue as A to L, with the key closed
until every row is filled. Length reads as thoroughness, so score each
column on its own and never the whole impression.

Each column is 0 to 3 except the last.

- **Grounding.** Open five file or line references the plan makes, in a
  worktree at the issue's base commit (`results/<short>-<issue>/base.txt`).
  3: all real and relevant. 2: one wrong. 1: several wrong. 0: the plan
  cites nothing checkable, or mostly misses.
- **Departures caught.** Where the issue's claims do not match the code,
  did the plan say so and say what it does instead? 3: every departure,
  including ones you had not noticed. 0: it planned the issue as written
  where the code says otherwise.
- **Decisions to overturn.** Count the calls you would reverse before
  implementing. 3: none. 2: one. 1: two. 0: three or more.
- **Over-specification.** How much of the plan an implementer would not
  need: restated code, sections with nothing to say, steps the test
  suite would force anyway. 3: nothing to cut. 0: half or more.
- **Hand-off.** Would you give this plan to an implementer as it is?
  `yes`, `note` (with one line of guidance), `no`.
- **Notes.** One line: the plan's one best and one worst thing.

After the key is opened, put beside each arm's scores what the run
measured: plan review rounds and whether the cap was hit (from
`report.json`), wall time, tokens by model, and for a single session the
`total_cost_usd` and `duration_ms` in `run.json`. Two repetitions per
arm: note where they disagree, since that is the consistency result.

Write the result per issue as the table and a paragraph of judgment, not
an average over two issues.

---

# The reference (owner-checked)

# Reference: Switchboard #187 at 7fe8949

Written before any plan was read. Line numbers are in `dispatch/src/`.

## What the code does

- There is no rerun-specific path. Every review attempt, first or rerun,
  launches through one function (`scheduler.rs:7440`), which copies the
  plan it reviews from `review_subject` (`scheduler.rs:10534`). That
  picks the newest attempt in state `Complete` holding a `plan`
  artifact. A failed review attempt is `Failed`, so it is skipped and the
  plan stage's output wins. That one filter is the bug.
- A `revise` answer (`revise`, `scheduler.rs:7565`) writes the owner's
  note as `plan.feedback-<round>.md` beside the reviewed copy in the
  review attempt's directory, pushes a `Revision { round, by, at_ms }`
  onto the attempt (`ticket.rs:510`), and sends `WorkflowObject`. The
  app's workflow (`src/core/workflow.rs:969` object_workflow, then the
  `AwaitingResponse` arm at `:655-680`) prompts the planner clone, which
  edits the copy in place. So the revised plan exists
  only as `review/<n>/<ctx>/plan.md`, and the ticket record already knows
  a revision happened. Nothing new has to be detected.
- The new attempt gets a fresh workflow run from the plan stage's
  planner session (`source_session`) and the copy; no rounds or points
  carry. `carried_from` (`ticket.rs:317`) carries settled and open points
  for code reviews only, and `rerun_carries` (`scheduler.rs:9382`) adds
  the "carries this one's points" sentence only for `AttemptKind::Review`.
  A plan review attempt is `AttemptKind::Workflow`, so its rerun question
  (`scheduler.rs:7240-7265`) says nothing about which copy it reviews.
- `show`'s `files:` block (`main.rs:981`) comes from `ticket_paths`
  (`serve.rs:539`) through `shown_where` (`ticket.rs`), which lists
  Complete attempts or an open Workflow attempt. A failed attempt's copy is
  never listed, and attempt 2's copy is the "plan" as soon as it is open.
  The `plan round N (owner)` lines list rounds and owner revisions.

## Departures from the issue's framing

1. "The review stage's rerun path" does not exist as such; the fix is in
   the subject lookup shared by every attempt. A plan that adds a
   rerun-only branch is building beside the real site.
2. The plan review does not carry anything between attempts today; only
   the code review does. A plan that says "the rerun already carries the
   points, just not the copy" is wrong.
3. The revision is not lost on disk and not unknown to the record:
   `revisions` on the failed attempt and the files in its directory are
   both there. The plan should use them, not add parallel state.

## Decisions, with the answer I would take

1. **Where the rerun's copy comes from.** Copy from the replaced attempt
   of the same stage and context when its copy is on disk, whether or not
   it was revised (unrevised it equals the plan output). Fall back to the
   plan stage's output. A plan that restricts this to "only if revised"
   is fine; one that prefers the plan stage's output unless some new flag
   is set is backwards.
2. **Provenance on the record.** Reuse `carried_from` on the new attempt
   (widen its doc from code review to any review) rather than a new
   field. A new field is acceptable, two is not.
3. **The rerun question names the copy.** Add a Workflow arm to
   `rerun_carries` that says the next attempt reviews the previous
   attempt's copy and, when there was a revision, that it holds the
   owner's round-N revision. Required by the issue's "at least".
4. **`show` lists both.** When an earlier review attempt's copy differs
   from the one under review, list it as a prior revision beside the
   reviewed copy. One extra `files:` line; not a redesign of `show`.
5. **The owner's note.** The revised copy carries its content. Copying
   the owner's feedback file into the new attempt's round numbering is
   optional; a plan must not route it through `t.rework`, since
   `WorkflowStart` takes no note.
6. **No app-side change.** The workflow in the app takes a plan path and
   a session; it needs nothing.
7. **Tests.** A scheduler test: review attempt with a revision fails,
   `rerun` answered, the new attempt's copy is the revised text, the
   rerun question and `show` name it. `docs/dispatch.md` (rerun around
   lines 318-350; the `revise` row of the decisions table around line 2975) and a
   `CHANGELOG.md` line.

## Anchors a grounded plan should cite

`scheduler.rs:10534` review_subject · `:7440-7520` review launch and
`WorkflowStart` · `:7565` revise · `:7240-7265` the rerun question ·
`:9382` rerun_carries · `:8988` rerun_note · `ticket.rs:317` carried_from
· `ticket.rs:510` Revision · `ticket.rs` shown_where / input_where ·
`serve.rs:539` ticket_paths · `main.rs:981` files block ·
`src/core/workflow.rs:969` object_workflow.

---

# The plan (v5lean-sonnet-a)

# Plan: #187, a plan-review rerun starts from the newest copy the stage holds

Checked against the worktree at HEAD `7fe8949`. "Facts" is `facts.md` for this issue.

## Where this departs from the issue

- **"The `revise` note … should survive a runner failure the same way an answered decision does."** It already does, so the plan changes nothing for the note. `revise` writes `plan.feedback-<n>.md` beside the reviewed copy (`dispatch/src/scheduler.rs:7616-7618`) and pushes a `Revision` on the attempt (7621). What is lost is the *revised copy as the rerun's input*, not the note.
- **"Nothing warned … `show` … names only the attempt-2 copy."** True of the rerun question (`rerun_carries` returns `""` for a workflow stage, `scheduler.rs:9382-9390`). The plan fixes the question; the `files:` block of `show` is left alone.
- **Repro step 2, "a runner restart mid-run" gives `review run gone`.** The tree does not bear this out (`remove_workflow`, `src/core/workflow.rs:1009-1028`, is the only drop; Facts, Settled). The plan fixes any failed review attempt, and the test models the failure with `runs.clear()` (`dispatch/tests/support/mod.rs:690`).

## The record

No change, no bump. `Attempt.artifacts["plan"]` of the failed `review/1` already holds the revised copy's path, and `Attempt.revisions` the owner's round.

## The actions and effects

No new action or effect. Three edits in `dispatch/src/scheduler.rs`:

1. **One helper `rerun_copy(t, stage, ctx, subject) -> Option<PathBuf>`** next to `review_subject` (10534). It walks `t.attempts` backwards (the vector is chronological, which `review_subject`'s `.rev()` already relies on; `n` is per stage and cannot order across stages). The first attempt that matches decides: a `Complete` attempt of any stage with the subject artifact, as `review_subject` filters (10534-10548), returns `None` and the rerun copies from it as today; a `Failed` workflow attempt of the same stage and context with the subject artifact, whose file exists, returns that path. Everything else, `Cancelled` included, is skipped: a parked or discarded review's copy is not the plan going forward (`ticket.rs:1856-1864`). A failed `review/1` followed by a re-run `plan/2` therefore loses to `plan/2`.
2. **`start_workflow` (7465-7472).** Keep `review_subject` for the session: `(source_path, source_session)`. Then `let source_path = rerun_copy(t, stage, ctx, subject).unwrap_or(source_path)` and copy that. The session stays `plan/1`'s, because `poll_workflow` overwrites `Attempt.session` of a workflow attempt with the run's face (7784-7788; Facts, Settled), and `WorkflowStart.source` must be the planning session.
3. **`rerun_carries` (9382-9390).** For a `Workflow` attempt that is `Failed` and has a copy `rerun_copy` accepts, return ` The next attempt reviews this attempt's copy, with its revisions: <path>.` Otherwise keep `""`.

Defaults on the Unresolved lines are what the code does today: the copy is taken as it is with `std::fs::copy`, and `plan/1`'s session is passed as `source` (a refusal fails the attempt through the existing `Reply::Failed` path, 7522 onward). Re-sending the owner's objection to the new run is not built.

## The tests

- `dispatch/tests/first_slice.rs`, one new test next to `3257`: `at_finalize`, `revise` with a note (as `3442`), overwrite `review/1`'s copy with marked text to stand in for the app's in-place edit, `sb().runs.clear()`, `step` to the `review run gone` failure, assert the `rerun` question contains `review/1/joined/plan.md`, answer `rerun`. Assert `review/2/joined/plan.md` equals the marked text (the fake never reads the file, Facts), the recorded `WorkflowStart.plan` is that path, and `source` is `plan/1`'s session.
- Same file: a *Cancelled* review (parked, then restarted) still copies from `plan/1`.
- Unit test beside `review_subject`: a failed `review/1` then a complete `plan/2` gives `None`; a complete `plan/2` then a failed `review/2` gives the failed copy; a Cancelled attempt gives `None`.

## Docs and release

- `CHANGELOG.md`, under Unreleased: "A Dispatch plan review that is run again after a failed attempt starts from the failed attempt's copy, with the owner's revisions, and the rerun question says which copy."
- `docs/design.md`: one paragraph in the Dispatch plan-review text near line 2213 — a rerun after a failed attempt copies the failed attempt's copy, `Cancelled` copies are not used, and the cloned planner has not seen the owner's objection (the file it reviews has) — plus a sentence in that paragraph for the status section.

## Commit

Message: `Dispatch reruns a failed plan review from the failed attempt's copy and says so`

Contract addition: `pub(crate) fn rerun_copy(t: &Ticket, stage: &str, ctx: &str, subject: &str) -> Option<PathBuf>` in `dispatch/src/scheduler.rs`.

---

# The facts file the planner was given

The plan may cite this file as facts.md or facts:<line>; such a citation is checked against this text, not the tree. It was written by an earlier stage of the same process, not by the owner, so it is not the reference and may itself be wrong.

     1	# Facts for #187 (tree HEAD 7fe8949)
     2	
     3	The issue and the investigation claim nothing about a browser, platform, crate or protocol outside this tree. Every open question is about this tree, a run, or the owner's machine, so I ran no web searches and wrote no spike. No spike is needed: no question the plan turns on is left to an outside source.
     4	
     5	## Settled
     6	
     7	Evidence is `file:line` in this worktree.
     8	
     9	- Nothing in `dispatch/src/` deletes a plan-review attempt directory or its `plan.md` or `plan.feedback-<n>.md` files.
    10	  - The `remove_dir_all` at `dispatch/src/scheduler.rs:10081` removes only `Evidence.dir`, in `drop_evidence`.
    11	  - The `remove_file` at `scheduler.rs:10154` is in `prepare_writes` ("A gate-only command's artifacts made ready", 10136). It is for gate commands, not workflow attempts.
    12	  - The `remove_file` at `scheduler.rs:7642` removes only the just-written owner round file, and only when the app refuses the revise.
    13	  - `retire_replaced` (3578-3605) deletes no directory.
    14	  - So `review/1/joined/plan.md` and its feedback file survive the failure and the rerun. A fix can read them.
    15	- The app can drop a `WorkflowRun` from the project record only through `remove_workflow` (`src/core/workflow.rs:1009-1028`). It refuses while the run `waiting()`. It is also lost on a store reset.
    16	  - `no such run` therefore means the run was removed by a user or the store was reset, not that the app lost it on a plain restart. The tree does not say which happened for the reporter (see For the owner).
    17	  - Evidence: `workflow.rs:1023` is the only `workflows.retain` on records. `core/control.rs:433` and `workflow.rs:54` retain definitions in settings, not runs.
    18	- A `rerun` answer's `--note` is accepted and then dropped for a plan review.
    19	  - `needs_note` (`dispatch/src/lib.rs:101-103`) requires a note only for `finalize`/`revise`, so `decide` for `rerun` takes one. `scheduler.rs:8150` enforces the requirement.
    20	  - `rerun_note` (`scheduler.rs:8988-8997`) returns `None` unless the stage is `Agent` or `Review`. A plan review is `StageKind::Workflow` (`pipeline.rs:715-725`).
    21	- `RunView` (`control/src/lib.rs:754-767`) carries `id, project, definition, state, round, cap, source, plan, reviewer, planner, op`.
    22	  - It carries no round-snapshot path. Dispatch learns the snapshot directory (`snapshot_dir`, `src/core/workflow.rs:41-48`, `<data dir>/workflows/<run>/round-<n>/`) only by reading the app's data directory itself.
    23	  - `RunView.plan` is the path of the copy Dispatch gave the app at start. In the repro that is `review/1/joined/plan.md`, which Dispatch already holds as `Attempt.artifacts["plan"]`. The snapshot is not needed to find the revised copy.
    24	- `WorkflowStart { source }` must name a live, resumable planning Claude session.
    25	  - The fake app refuses an unknown source with `no such session` and a non-resumable one with `cannot be the planner` (`dispatch/tests/support/mod.rs:397-405`). `design.md` is read as saying the same for the real app.
    26	  - `review_subject` (`scheduler.rs:10534-10548`) returns `a.session` of the chosen attempt. For a workflow attempt `poll_workflow` overwrites `Attempt.session` with the run's `reviewer`/`planner` face (`scheduler.rs:7784-7788`). Picking `review/1` as the subject there would pass a reviewer or planner id as `source`.
    27	  - The planning session to name is `plan/1`'s session. `review/1.run_sessions` (filled by `note_run_sessions`, `scheduler.rs:7710-7735`, keys `reviewer`/`planner`) holds the run's cloned `planner` session. The fake creates that session as "planner planner" (`support/mod.rs:416-423`), and a rerun may use it only if it is still resumable.
    28	- The fake app does not read the plan file on `WorkflowStart`.
    29	  - It stores `plan: plan.clone()` in a `RunView` (`support/mod.rs:430-441`) and never opens the file.
    30	  - A test must therefore assert on the file Dispatch copies (`review/<n>/joined/plan.md`) and on the `plan` path in the recorded `WorkflowStart`, which are Dispatch-side facts.
    31	- The cause is `review_subject` (`scheduler.rs:10534-10548`): it filters `AttemptState::Complete`, so the Failed `review/1` is skipped and `plan/1` is copied (`scheduler.rs:7465-7472`). `rerun_carries` (`scheduler.rs:9382-9390`) returns `""` for a workflow stage, so the rerun question never names a copy.
    32	- Existing tests that constrain a fix:
    33	  - `dispatch/src/ticket.rs:1827-1875` asserts a Failed or Cancelled review copy is not shown as the plan (1856-1864).
    34	  - `ticket.rs:1797` asserts a Failed plan attempt is skipped as an input.
    35	  - `dispatch/tests/first_slice.rs:3257-3287` and `3817-3831` cover the failed-review rerun without checking which plan was copied.
    36	  - `first_slice.rs:3442-3500` covers revise without a rerun.
    37	  - No test combines revise, a failed review and a rerun.
    38	- `show`'s `files:` block (`dispatch/src/serve.rs:538-600`, `main.rs:980-995`) takes `review` from the latest workflow attempt that has artifacts (`serve.rs:564-568`).
    39	  - After the rerun that is `review/2`, so `plan_rounds` lists nothing for the owner's round in `review/1`.
    40	  - `print_answered` (`main.rs:1176-1198`) still prints the `revise` note while the decision stays on the ticket, so the note is visible there.
    41	- Rendering of `plan_reviewing`/`plan_round` in `client/Sources` was not read. It is a presentation question for the planner, not a fact the plan turns on.
    42	
    43	## Unresolved
    44	
    45	- Whether a half-written `review/1/joined/plan.md` is possible after a killed run (the file is edited in place by the app).
    46	  - Tried: reading `start_workflow`/`revise` in Dispatch. Dispatch only copies and reads it. The app's edit path is in `src/core/workflow.rs`, which I did not read in full.
    47	  - Not settled because the answer needs the app's write behaviour or a killed run, which is outside the smallest experiment the plan turns on.
    48	  - Build to: copy `review/1/joined/plan.md` as it is, and write the new attempt's copy with the same copy call. This is safe either way and needs no new atomic write: a partial file is the owner's call to inspect, and the rerun question names which copy is used.
    49	  - If favourable, nothing more is needed. If unfavourable, nothing more is needed either, because the owner can see the named copy.
    50	- Whether the planning session `plan/1` names is still resumable at rerun time (needed for `WorkflowStart.source`).
    51	  - Tried: reading `start_workflow` and the fake. The real check lives in the app and needs a live session.
    52	  - Build to: keep passing `plan/1`'s session as `source`, as the first attempt does today. If it is gone, the existing `Reply::Failed` branch (`scheduler.rs:7522` onward) already fails the attempt with a reason and asks `rerun`/`park`. The plan must add no fallback to another session.
    53	- Whether `Revision` entries on `review/1` can be re-sent to a new app run.
    54	  - The `WorkflowObject { run, round, text }` body targets an existing run (`revise`, `scheduler.rs:7565-7650`). Whether the app accepts an objection on a fresh run at round 1 was not checked.
    55	  - Build to: the plan must not depend on resending the owner's note. Starting the rerun from the revised copy carries the revision itself. The note stays available as the answered `revise` decision.
    56	
    57	## For the owner
    58	
    59	Questions of fact only the owner can observe. Neither answer changes what the plan builds, and none blocks it.
    60	
    61	- Does `review/1/joined/plan.md` on the reporter's machine still hold the revision (diff against `plan/1/joined/plan.md` and `review/2/joined/plan.md`), and does `review/1/joined/plan.feedback-2.md` exist with the owner's note?
    62	  - If yes: the fix can read both from the failed attempt, and the issue's repro is confirmed on disk.
    63	  - If no: the fix still starts the rerun from the newest copy Dispatch holds (the failed attempt's copy or `plan/1`) and names it in the question and in `show`.
    64	  - The plan builds the same either way.
    65	- Did the Switchboard app's project record lose the run (a removed run, a store reset, or a changed data directory) between the review's start and the `rerun` answer?
    66	  - If yes: `no such run` is explained, and the plan may add a test that models it with `runs.clear()` (`support/mod.rs:690`).
    67	  - If no (runner restart only): the cause is outside this tree. The plan still builds the same fix, because a failed attempt of any reason has to hand its copy to the rerun.
    68	- Does attempt 2's plan carry the content the issue describes (every question open, an owner answer contradicted, two of three round-1 points already fixed in the revised copy)?
    69	  - If yes: the plan has a worked example for its test fixtures, with the client's text replaced.
    70	  - If no: the plan builds the same. The content check is the owner's own ticket and does not bear on the mechanism.
