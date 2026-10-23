You are scoring one implementation plan for a GitHub issue, blind: you do not know who or what wrote it, and you score this plan on its own.
The repository at the issue's base commit is checked out at /Users/sully/code_repos/plan-trial/sb-187-base (read-only; change nothing there). Every file path in the plan is relative to that tree.

Score by the rubric, column by column. Length is not thoroughness. For grounding, take the first five distinct file or line references the plan makes, in order of appearance, open each in the tree, and say what you found. For departures and decisions, use the reference as the yardstick; where you disagree with the reference, say so in the evidence rather than silently scoring your own view.

Your final message must be exactly this shape and nothing else:

```
| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| v5-sonnet-a | <0-3> | <0-3> | <0-3> | <0-3> | <yes|note|no> |
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

# The plan (v5-sonnet-a)

# Plan: issue #187, a plan review rerun starts from the owner's revision

Line numbers are from the worktree. "Facts" is `facts.md`; S is its Settled
section, U its Unresolved, O its For the owner.

## Where this departs from the issue

- The issue says the `revise` note "should survive a runner failure the
  same way an answered decision does". It already does (S, "The app does not
  touch the copy"). The plan adds no storage for the note. It names the
  feedback file (`<subject>.feedback-<round>.md`) in the rerun question so the
  owner can find it.
- The issue's "newest plan the stage holds" has no counterpart in the
  code: `review_subject` (`scheduler.rs:10534`) takes the newest
  `Complete` attempt that wrote `plan`, and a `revise` edits the copy of
  an attempt that is never `Complete` until `finalize`. The plan adds the
  one case the issue is about (a failed attempt holding a finished owner
  revision), not a general rule.
- The investigation notes say a non-empty `Attempt.revisions` shows that
  the copy holds an owner revision. It does not (S, "Partial or finished
  revision"): `revise` pushes the `Revision` before sending the round and
  keeps it when the app refuses the round (`scheduler.rs:7607-7647`). The
  plan corrects the `revisions` doc comment (`dispatch/src/ticket.rs:348`) and
  does not change the behaviour. The evidence of a finished revision is the
  response file the planner writes.

## The record

One additive field. `Attempt.revised_from: Option<u32>` in
`dispatch/src/ticket.rs`, `#[serde(default)]`: for a plan review attempt, the
number of the earlier attempt of the same stage and context whose
owner-revised copy this attempt's copy was made from; none when it was made
from the plan stage's output. Written when the attempt is made in
`start_workflow` and never changed, as `carried_from` is, so `show` and a
restarted runner report what was done. `new_attempt` (`scheduler.rs:10277`)
sets it `None`.

`dispatch/src/store.rs`: `RECORD_VERSION` (`store.rs:26`) goes from the value
there at implementation time, plus one. Add the matching comment-only step in
`migrate` (`store.rs:452`), as the other steps that only add a defaulted
field do. `crate::model` (`SCHEMA_VERSION`, `src/core/model.rs:12`) is not
touched. `docs/dispatch.md` has its per-version list near line 393: add the
paragraph there.

## The signals

No new signal and nothing new from outside. The one new read is whether a
response file exists beside a failed attempt's copy
(`report::response_file`, `dispatch/src/report.rs:202`).

Choosing the plan: replace `review_subject` (one caller, `start_workflow`
at `scheduler.rs:7465`; S "Plan readers") with
`plan_to_review(t, stage, ctx, subject)`, returning an owned struct
`{ path, session, revised_from: Option<u32>, skipped: Option<(u32, u32)> }`.

Walk `t.attempts` newest first by `Vec` position:

1. A `Complete` attempt whose `artifacts` has `subject` ends the walk. Its
   `session` is the `source`, as today. Its path is the plan, unless step 2
   found a revised copy earlier in the walk.
2. A `Workflow` attempt of this `stage` and `ctx`, `Failed` or `Cancelled`,
   with `artifacts[subject]` a file on disk and a non-empty `revisions`, is a
   candidate. Take the highest `Revision.round` whose feedback file
   (`report::round_file`) still exists, since a refused round's file was
   removed. No such round: not a candidate. Its response file exists: the
   planner has answered that round, and this is the revised copy (first one
   wins). Otherwise no response file was found, so record it in `skipped` as
   `(attempt n, round)` and keep walking.
3. Anything else is passed over.

Why these choices:
- U2 (is a failed copy ever mid-edit) is unresolved and nothing here can
  answer it, so the default does not assume either way. A round with no
  response file is not used; the plan stage's output is, and the question
  says what was found. The revised copy is taken when the planner has
  answered the round. That is weaker than "finished": the respond prompt
  (`src/core/model.rs:279-284`) says to accept items by updating the plan but
  does not order the edit before the response, and the app counts a round
  responded when the response file appears (`src/core/workflow.rs:663-668`),
  so the copy taken may lack the planner's last edit. The implementer's first
  check is to read that template and code, confirm the ordering gap, and say
  so in the `dispatch.md` paragraph and the `design.md` text. Both branches
  give a working rerun.
- U1: pinned by test (c).
- O1: pinned by test (f).
- The copy carried is the failed attempt's whole file, not the owner's edit
  alone: the owner round is round N+1 on the same copy (`revise` uses
  `view.round + 1`, `scheduler.rs:7627`) and nothing separates it from the
  reviewer rounds. A later failure without a new `revise` still reaches
  review/1's copy (review/2 has no `revisions` and is passed over).

## The actions and effects

The same `WorkflowStart`, with `plan_to_review().path` copied
(`scheduler.rs:7466-7471`). Add the call's `revised_from` to the attempt it
pushes (`:7489-7497`) and log one `info` line naming the file chosen and, when
`skipped` is set, the revision with no response file that was passed over.

`rerun_carries` (`scheduler.rs:9382`) returns `String` instead of
`&'static str` and gains an arm for a `Workflow` attempt (today it returns
"" there). Both callers format it already (`:7266`, `:9373`). It calls
`plan_to_review` with the attempt's stage and context, taking the subject from
the attempt's `artifacts` only when that map has exactly one key (otherwise
""). It adds one sentence:
- revised: "The rerun reviews the copy from `<stage>/<n>` at `<path>`, which
  holds that attempt's edits up to round `<r>`, your `revise` round; your
  note for it is at `<feedback file>`."
- skipped: "`<stage>/<n>` has a revision (round `<r>`) but no response file
  was found for it, so the rerun reviews the plan stage's output at `<path>`;
  your note is at `<feedback file>`."
- otherwise: "The rerun reviews the plan stage's output at `<path>`."
- an `Err` from `plan_to_review` gives "", as now.

## The UI

CLI `show` only.

- `dispatch-control/src/lib.rs:586`: `LaneFile` gains `prior:
  Option<PriorCopy>`, with a new `PriorCopy { n: u32, path: PathBuf }`
  (`Default`, `#[serde(default)]` like its neighbours).
- `ticket_paths` (`dispatch/src/serve.rs`, the `plan_files` build near the
  `LaneFile { .. }` literal) fills `prior` from the attempt's
  `revised_from`: that attempt's `artifacts[subject]`.
- `print_lane_files` (`dispatch/src/main.rs:896`) prints, under the plan line
  and whether or not the review is still open: `(started from the copy in
  <stage>/<n>: <path>)`.

## The tests

Integration, `dispatch/tests/first_slice.rs`, next to
`a_failed_review_run_fails_the_attempt_and_a_rerun_starts_a_new_run` (`:3257`)
using `at_finalize`, `revise` (`:3417`) and `review_run_fails` (`:3249`):

1. Revised then failed then rerun: after `revise`, the test plays the
   planner (edit the copy, write `response_file(copy, 2)`), fails the run,
   answers `rerun`. Assert the attempt-2 copy has the revised text,
   `revised_from == Some(1)`, the `WorkflowStart` plan is the attempt-2 copy,
   its `source` is the plan stage's session, and the rerun question names
   review/1's copy and the feedback file. `show --json` has `prior`
   pointing at review/1's copy, and the `show` text has the `started from`
   line (this covers `print_lane_files`).
2. No response file: same without it. Attempt 2's copy equals `plan/1`'s
   output, `revised_from` is none, the question says no response file was
   found.
3. Refused round: the fake refuses `workflow.object`, so the feedback file
   is gone and the `Revision` stays; the rerun starts from the plan stage's
   output without the "no response file was found" sentence.

Unit, in `scheduler.rs` beside the plan-reader tests (about `:12449`):
`plan_to_review` on built tickets: (b) a failed review with a response file
before it, where a `Cancelled` attempt counts as a `Failed` one; (c) a
`Complete` plan attempt appended after the failed review wins (U1); (d)
review/2 failed with no revisions after a finished review/1: review/1's copy
is chosen again; (f) the copy file missing falls back (O1). `store.rs`: add a
previous-version fixture test like the neighbours: the record loads with
`revised_from` none and is written at the new version. `serve.rs` tests
(about `:1590-1710`): `ticket_paths` sets `prior`.

## Docs

- `docs/dispatch.md` near `:393`: the version paragraph.
- `docs/design.md`: the plan review rerun paragraph (`:1783-1790`, "a rerun
  kills the old run's agents and starts a fresh run"): add that the fresh
  run starts from the failed attempt's copy, with all its edits, when the
  planner has written a response to the owner's round, and that the copy is
  not proven final (U2). Under the Dispatch ticket page section (`:2135`) list
  the gap: the Plan tab does not show `prior`.
- `CHANGELOG.md` under Unreleased: "A plan review rerun after a failure
  starts from the failed attempt's copy when the planner answered the owner's
  `revise` round."

## One commit

Message: `Dispatch: a plan review rerun starts from the failed attempt's copy with the owner's revision (#187)`

Contract additions (copy into the commit message body):
- `Attempt.revised_from` (dispatch `RECORD_VERSION` bumped, value in
  `store.rs` at implementation time plus one)
- `PriorCopy`
- `LaneFile.prior`

## Follow-ups for the owner

The rerun's planner is still a clone of the plan stage's original session
(S, "`source` and the new planner"), so it has not seen the `revise` note;
fixing that is an app change and a spike. The owner's data directory settles
O1 and U2.

---

# The facts file the planner was given

The plan may cite this file as facts.md or facts:<line>; such a citation is checked against this text, not the tree. It was written by an earlier stage of the same process, not by the owner, so it is not the reference and may itself be wrong.

     1	# Facts: issue #187
     2	
     3	The issue and the investigation claim nothing about a browser, platform, crate, protocol or tool outside this tree. Every open question is about the tree, so no web search was run and no spike was written (git status is empty). Line numbers are from this worktree.
     4	
     5	## Settled
     6	
     7	Plan readers (open question 1):
     8	- `review_subject` has one caller, `start_workflow`. It reads `t.attempts` newest first by `Vec` position and keeps only `AttemptState::Complete`. `dispatch/src/scheduler.rs:7465`, `:10534-10547`.
     9	- Every other reader of the plan goes through `Ticket::input_where`, which is also Complete-only and skips `forgotten`. These are `lane_input` (`scheduler.rs:10843`), then `lane_plan`, `lane_plans` and `plan_clause`, and the prompt variables `vars_for` (`:10776`). `dispatch/src/ticket.rs:1452-1462`.
    10	- Report plan lists use `lane_files` (`dispatch/src/report.rs:300`). `serve.rs:587` reads only `notes`.
    11	- Changing `review_subject` alone leaves all of these on Complete attempts, so downstream stages and prompts never see an unfinalized or failed copy.
    12	- Making `input_where` count a failed review attempt's copy would change what implement and code review are handed. That is wider than the issue.
    13	- `Ticket::shown_where` also takes an open workflow attempt, never a failed one. `ticket.rs:1476-1488`. It is display only (`scheduler.rs:10917-10919`).
    14	
    15	The app does not touch the copy (question 2):
    16	- The app has no code that deletes or moves the plan copy or its feedback and response files. `RemoveWorkflow` only drops the run record and probes: `src/core/workflow.rs:1009-1031`.
    17	- `start_workflow` in the app only reads `plan` and clones the source's transcript. It writes nothing to that path: `src/core/workflow.rs:289-345`.
    18	- The only `remove_file` calls in `src/` are `app.rs:1497` (`remove_quietly`, unrelated), the hook binary's own files, the updater, and a PNG temp in `ui/document.rs`. `grep remove_file|remove_dir_all|rename src` found no others.
    19	- Dispatch's `fail_attempt` also touches no files (investigation item 6).
    20	- So review/1's `plan.md` is intact when attempt 2 starts. Dispatch deletes one file only: the owner's feedback file for a round the app refused (`scheduler.rs:7640-7647`).
    21	
    22	Partial or finished revision (question 3):
    23	- `revise` writes the feedback file, then pushes the `Revision` onto `Attempt.revisions`, then sends `WorkflowObject` (`scheduler.rs:7607-7628`).
    24	- If the app refuses the round, the feedback file is removed but the `Revision` stays (`:7640-7647`). A non-empty `revisions` therefore does not prove the planner revised anything.
    25	- `revise` runs only while the run is `Converged` or `AtCap` (`:7600-7606`).
    26	- The investigation's suggestion that "non-empty `revisions` says the copy holds an owner revision" does not hold. A finished revision is evidenced by the response file `<subject>.response-<round>.md` beside the copy, which the planner writes.
    27	
    28	Attempt order (question 4):
    29	- Attempts are appended to `t.attempts` (`scheduler.rs:7479`), and every reader above orders by `Vec` position. `started_ms` and `ended_ms` exist (`ticket.rs:375-376`) but no reader uses them to rank attempts.
    30	- Any "newest plan" rule built on position is well defined, with ties impossible.
    31	
    32	`source` and the new planner (question 5):
    33	- The app requires `source` to be a Claude Code session with a transcript and clones it as the planner (`src/core/workflow.rs:306-322`).
    34	- `review_subject` returns the plan stage's session as `source`. The new planner is therefore a clone of the original planner, which has seen neither the `revise` note nor the revision.
    35	- The reviser's context lives only in review/1's run planner clone (an app record the failed run leaves behind).
    36	- The owner's note survives on disk in the decision's `DecisionState::Answered { note }` and in `<subject>.feedback-<round>.md`. The `OWNER_ROUND_HEADING` text is in the feedback file.
    37	
    38	Wire types (question 6):
    39	- `LaneFile` is `#[serde(default)]`, so a new field is additive on the wire (`dispatch-control/src/lib.rs:584-599`).
    40	- The app reads only `plan_reviewing`, `plan_reviewed`, `plan_round` and `plan_stage` (`src/ui/ticket.rs:427-431`).
    41	- The CLI reads `plan_files` through `print_lane_files` (`dispatch/src/main.rs:896`, `:982`).
    42	- `grep planFiles|plan_files|reviewing client/` found nothing in the Swift client. The investigation's mention of `TicketFileView` could not be matched to a use of these fields.
    43	- A new `LaneFile` field or a new `PathsView` list should be additive and break none of the three readers.
    44	
    45	The `[run]` question:
    46	- The fake already reproduces the path up to the rerun: `a_failed_review_run_fails_the_attempt_and_a_rerun_starts_a_new_run` (`dispatch/tests/first_slice.rs:3257`), with `review_run_fails` (`:3249`).
    47	- Whether attempt 2 starts from `plan/1` follows from the read above (Complete filter at `scheduler.rs:10537`; no step marks a failed attempt Complete).
    48	- The plan stage is Complete in that test, and the test's review attempt has no revise before the failure.
    49	- A new test only needs a `revise` step (helper at `first_slice.rs:3417`) between `at_review_run` and `review_run_fails`. This is a test the plan writes, not a question for a spike.
    50	
    51	## Unresolved
    52	
    53	U1. Can the plan stage itself be rerun after review/1 failed, so `plan/2` is appended after review/1?
    54	- Tried: read `workflow_stage`, `review_subject` and `start_workflow` only. I did not trace the stage-ordering code.
    55	- Branch where it can: a "newest plan" rule by `Vec` position then prefers `plan/2` over review/1's revision. That is correct, since a fresh plan supersedes the old revision. The plan must test this order.
    56	- Branch where it cannot: the rule is simply "newest of failed review copy and Complete plan".
    57	- Build to: rank by `Vec` position in both cases. Add a test with a later `plan` attempt, and do not wait on this line.
    58	
    59	U2. Is a failed attempt's copy ever mid-edit when the rerun starts? The planner clone may still be running when `poll_workflow` sees `no such run` or a failed state.
    60	- Tried: nothing runs here (needs a live app and planner), and the code does not stop the planner on `fail_attempt`.
    61	- Build to: if the planner is quiet, take the failed copy. If not, the plan cannot tell. So the rule that copies a failed attempt's `plan.md` must require a finished revision. The evidence available is the response file for the latest `Revision.round` on that attempt. Otherwise fall back to the Complete plan, and the rerun question and `show` must say which file is used.
    62	
    63	## For the owner
    64	
    65	O1. Was `review/1/joined/plan.md` still the revised text on disk after the failure, and is the `revise` note held anywhere else?
    66	- Needs the ticket's data directory.
    67	- Both branches work: the plan builds to "the failed attempt's copy is intact" (the tree shows nothing deletes it, above) and to "the note lives in the decision and in `plan.feedback-<round>.md`".
    68	- If the owner reports the copy was empty or reverted, the plan adds nothing different. It falls back to the Complete plan and names it in the rerun question and `show`.
