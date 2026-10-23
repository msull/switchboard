You are scoring one implementation plan for a GitHub issue, blind: you do not know who or what wrote it, and you score this plan on its own.
The repository at the issue's base commit is checked out at /Users/sully/code_repos/plan-trial/sb-187-base (read-only; change nothing there). Every file path in the plan is relative to that tree.

Score by the rubric, column by column. Length is not thoroughness. For grounding, take the first five distinct file or line references the plan makes, in order of appearance, open each in the tree, and say what you found. For departures and decisions, use the reference as the yardstick; where you disagree with the reference, say so in the evidence rather than silently scoring your own view.

Your final message must be exactly this shape and nothing else:

```
| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| v5lean-sonnet-c | <0-3> | <0-3> | <0-3> | <0-3> | <yes|note|no> |
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

# The plan (v5lean-sonnet-c)

# Plan: issue #187, a review rerun starts from the newest plan the stage holds

## Where this departs from the issue

- The issue says "kill the review run … a runner restart mid-run does it". The tree does not bear that out as the cause. The plan does not depend on why the run vanished (facts, Unresolved: "Why the app answered `no such run`"); it reads the failed attempt's record, which `fail_attempt_with` leaves intact (`scheduler.rs:7195-7230`, facts Settled).
- "The `revise` note … should survive a runner failure the same way an answered decision does." It already does; what is lost is the pointer to the revised copy.
- The issue says `show` "names only the attempt-2 copy". Before the rerun answer it is worse: `shown_where` (`ticket.rs:1476-1491`) skips the failed review, so `show` names `plan/1/joined/plan.md` and the revised copy is not listed.

## The actions and effects

No new `AppAction` or `Effect`. Three changes in `dispatch/`:

1. **`review_subject` (`scheduler.rs:10534-10548`) picks the copy and the session separately.**
   - The path is the file of the newest attempt that wrote `subject` and is either `Complete`, or a `Workflow` attempt in `Failed` whose file exists (`Path::is_file`; if it does not, skip that attempt, so a deleted copy degrades to today's behaviour instead of making `fs::copy` fail).
   - `Failed` only; a `Cancelled` copy may be one Dispatch discarded (`ticket.rs:228-232`).
   - A failed review is accepted whether or not it has `revisions`. Risk: a review edits its copy in place until `finalize` (`ticket.rs:1446-1450`), so one that failed mid-round may leave a half-edited copy. The alternative, counting only revised attempts, drops the owner's `revise` on every chain (review/2 fails with no `revisions` of its own, and review/3 falls back to `plan/1`).
   - The session stays what it is today: the newest `Complete` attempt that wrote `subject` and has a session. It must not come from the failed review, whose `session` is the planner clone's face (`scheduler.rs:7786-7788`).
   - Return `(PathBuf, String)` as today; the only change is how the path is chosen. The one caller is `start_workflow` (`scheduler.rs:7465`), which keeps copying into the new attempt's directory.
   - `Ticket::input_where` (`ticket.rs:1452-1463`) is unchanged: it feeds downstream stages, which read only a completed attempt's output (test at `ticket.rs:1797-1824`).
2. **The rerun question names the copy.** `rerun_carries` (`scheduler.rs:9382-9390`) returns `&'static str` and gives a `Workflow` attempt nothing. Change it to return `String` and, for a `Workflow` attempt, append ` The next attempt reviews <path>`, where the path is the one item 1 picks. Both callers (`scheduler.rs:7266`, `9373`) change type only.
3. **`Ticket::shown_where` (`ticket.rs:1476-1491`) also accepts a `Failed` `Workflow` attempt**, so `show`, the ticket page and `shown_lane_files` name the copy the rerun reviews. Put the predicate on `Attempt` once (`holds_review_copy`) and use it in both places. `lane_files_by` already narrows to `Complete` for the input case (`scheduler.rs:10917-10921`).

## The tests

- `dispatch/src/scheduler.rs` unit tests beside `review_subject`: (a) plan/1 `Complete` plus review/1 `Failed` with a copy on disk → path is review/1's, session is plan/1's; (b) review/1 failed with the file missing → falls back to plan/1's path; (c) a `Cancelled` review with a copy on disk is never chosen; (d) review/1 `Complete` newer than plan/1 → unchanged from today.
- `dispatch/tests/first_slice.rs`, beside `a_run_switchboard_no_longer_has_fails_the_attempt` (line 3809): (1) drive plan, review, answer `finalize` with `revise --note`, clear `env.sb().runs` so the attempt fails, answer `rerun`. Assert the `WorkflowStart` for attempt 2 carries a `plan` under `review-plan/2/` whose bytes equal the revised `review-plan/1` file, that `source` is the plan attempt's session, and that the `rerun` question text names the review/1 copy. (2) The chained failure: a second failure, and attempt 3 starts from attempt 2's copy.
- `dispatch/src/ticket.rs` tests: `shown("plan")` returns a failed workflow attempt's copy; `input("plan")` still returns the completed attempt's (extend the test near line 1840).

## Docs, one commit

- `docs/dispatch.md`: the rerun rules near 318-350 get one sentence on a review rerun's input; the `revise --note` row at 2975 gets "a failed review attempt's rerun starts from its copy".
- `docs/design.md`: the Dispatch known-gaps list (line 3098) does not list this gap, so add one status sentence to the rerun paragraph at line 1702: a review rerun starts from the newest reviewed copy.
- `CHANGELOG.md`: under Unreleased, "Dispatch: a plan review rerun after a failed attempt starts from the newest reviewed copy, so an owner's `revise` survives, and the rerun question and `show` name the copy."

## Commit message

Dispatch: a plan review rerun starts from the newest reviewed copy, not the plan stage's output, and the rerun question and `show` name it

## Contract additions

- `Attempt::holds_review_copy` (dispatch crate, internal helper).
- No record, signal, `AppAction` or `Effect` change; no schema bump.

---

# The facts file the planner was given

The plan may cite this file as facts.md or facts:<line>; such a citation is checked against this text, not the tree. It was written by an earlier stage of the same process, not by the owner, so it is not the reference and may itself be wrong.

     1	# Facts for issue #187
     2	
     3	Nothing was run or built. Tree claims are from reading this worktree (HEAD 7fe8949).
     4	
     5	## Settled
     6	
     7	Tree questions from the investigation:
     8	
     9	- The app's `WorkflowStart` takes any absolute `plan` path; it does not need a file the attempt copied itself. The only check on the path is `is_absolute` (`src/core/workflow.rs:289-325`, wire decode `src/core/control.rs:1384-1394`). The planner is cloned from `source`, which must be a Claude Code session with a transcript. So the plan may pass review/1's revised copy as `plan` while `source` stays the plan attempt's session.
    10	- `clean_up` removes only the recorded round files (`feedback` and `response` of each round), never the reviewed copy (`src/core/workflow.rs:859-870`, `src/core/model.rs:394-399`). A cleaned-up review/1 still has its `plan.md`.
    11	- `remove_workflow` drops the run from `workspaces[].workflows`, and only when the run is not waiting (`src/core/workflow.rs:1009-1025`). It touches no Dispatch file. Dispatch sends no `workflow.remove` (investigation §4, grep of `dispatch/`).
    12	- A runner restart does not edit workflow attempts. The only workflow mention in `dispatch/src/restart.rs` is the match arm at line 883, which sets `failed_at_checks` to false. Nothing there touches `artifacts` or `revisions`. `recover.rs` has no reference to `Workflow`, `artifacts` or `revisions` (grep).
    13	- `fail_attempt_with` changes `state` and `ended_ms` and leaves `artifacts` untouched (`dispatch/src/scheduler.rs:7195-7230`, per the investigation). A failed review attempt still names its revised copy as `plan`, and `revisions` is still set.
    14	- The cause is `review_subject` (`dispatch/src/scheduler.rs:10534-10548`). It keeps only `AttemptState::Complete` attempts, so the failed review/1 is skipped and the plan stage's output is used. `Ticket::input_where` has the same filter (`dispatch/src/ticket.rs:1452-1463`).
    15	- `show` and the rerun question behave as the investigation's §2 and §1b say: they name no copy and ignore review/1's revision (`dispatch/src/serve.rs:538-594`, `dispatch/src/main.rs:981-998`, `dispatch/src/scheduler.rs:9335-9390`).
    16	- `Revision` is `#[serde]` data on `Attempt` (`dispatch/src/ticket.rs:346-350`). A test pushes one at `ticket.rs:1293`.
    17	
    18	Outside the tree:
    19	
    20	- Claude Code deletes session transcripts older than `cleanupPeriodDays` (default 30), and a deleted transcript cannot be resumed. Covers Claude Code 2.1.x. Evidence: `spikes/01-claude-resume/README.md:7` and `:278-285` (read from the binary's schema), and https://claudeissues.com/issue/59248-silent-retention-cleanup-deletes-session-transcripts-with-no-warning-opt-in-or-r (a report that agrees). This bears on the plan only if a rerun is days old. The rerun clones from the plan attempt's session, the same one it clones from today.
    21	  - Query: `Claude Code cleanupPeriodDays session transcripts deleted --resume session files retention`
    22	
    23	## Unresolved
    24	
    25	- **Codex `resume <id> "<prompt>"` when the rollout is missing** (`docs/design.md:1173-1174` lists it as unverified).
    26	  - Query: `OpenAI Codex CLI "codex resume" session id prompt rollout file missing error`.
    27	  - Only third-party blogs came back, and none documents that case.
    28	  - Line to build to: the fix does not need it. A review rerun starts a fresh run through `WorkflowStart`, with a new reviewer, and resumes nothing. Add nothing that resumes a Codex reviewer. If it is later verified, nothing in this plan changes.
    29	- **Why the app answered `no such run`** for the lost run after a runner restart. The tree shows the run was absent from `workspaces[].workflows` at the query. It does not show why. It was not reproduced, and the reproduction needs a live app and the owner's data.
    30	  - Line to build to: the plan must not depend on the cause. Whatever removed the run, a Failed review attempt keeps its `artifacts` and `revisions` on the Dispatch side, so the plan must pick the rerun's input from the ticket record alone.
    31	
    32	## For the owner
    33	
    34	- Did anything send `workflow.remove` or `Clean up` on the run between the revise and the failure? Whether or not it did, the fix is the same: the rerun reads the failed attempt's record. If the answer is yes, the fix is also unchanged. `clean_up` leaves the reviewed copy, and the run's removal does not touch Dispatch's files.
    35	- On the owner's ticket, does `review/1/joined/plan.md` hold the revision, and is `review/2/joined/plan.md` byte-identical to `plan/1/joined/plan.md`? If yes, this matches the code path above. If no, the cause lies elsewhere and the plan stays the same for the Failed-attempt case. The owner then reports the diff on the issue.
    36	- The issue's other claims about the run's contents are about the owner's ticket and cannot be checked here: the 09:28 write time, two of attempt 2's three round-1 points already fixed in the revised copy, and the finalize plan still listing every question as open. The plan does not depend on them.
