You are scoring one implementation plan for a GitHub issue, blind: you do not know who or what wrote it, and you score this plan on its own.
The repository at the issue's base commit is checked out at /Users/sully/code_repos/plan-trial/sb-187-base (read-only; change nothing there). Every file path in the plan is relative to that tree.

Score by the rubric, column by column. Length is not thoroughness. For grounding, take the first five distinct file or line references the plan makes, in order of appearance, open each in the tree, and say what you found. For departures and decisions, use the reference as the yardstick; where you disagree with the reference, say so in the evidence rather than silently scoring your own view.

Your final message must be exactly this shape and nothing else:

```
| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| v5lean-sonnet-b | <0-3> | <0-3> | <0-3> | <0-3> | <yes|note|no> |
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

# The plan (v5lean-sonnet-b)

# Plan: #187, a plan review rerun keeps the owner's revision

References are to this worktree. "Facts" is `facts.md` for #187; "Settled" lines are cited by their claim.

## Where this departs from the issue

- The issue says the `revise` note "should survive a runner failure the same way an answered decision does". It already does: `revise()` writes it to `review/<n>/joined/plan.feedback-<round>.md` before sending and deletes it only if Switchboard refuses (`dispatch/src/scheduler.rs:7616-7642`; Facts, Settled: the note survives on disk). The plan adds no copy of the note to the record.
- The issue frames the loss as a rerun-after-failure bug. The same `review_subject` rule (`scheduler.rs:10534-10548`, called at `:7465`) loses the revision on park then resume, where a cancelled review attempt is rerun through `rerun_ask`'s `Cancelled` arm (`:9363-9368`; Facts, Settled). The fix sits in the shared path, so it covers both.

## The record

One additive field on `Attempt` (`dispatch/src/ticket.rs`, beside `revisions` at `:350`): `seeded_from: Option<u32>`, `#[serde(default)]`. It is the number of the earlier attempt of the same stage and context whose copy this attempt's reviewed copy was copied from. `None` means copied from the newest complete attempt, as today. A later reader (`show`, a third attempt) needs to know what the copy holds, and a rerun answer can be withdrawn or re-asked, so the record says what was reviewed.

Bump `RECORD_VERSION` to the value in `store.rs` at implementation time, plus one. Add a `migrate` step with no transform, the same pattern as 18 to 19. Add the version line to `docs/dispatch.md` after the existing version list (`:385-400`).

`Attempt::holds_revision()`: `!revisions.is_empty() || seeded_from.is_some()`. It only decides whether to offer `original` and whether to default to the carried copy. `seeded_from` is set only when a revision was carried, so a chain of failures keeps carrying.

## The signals

None new.

## The actions and effects

No new `Effect`, `Body` or port method. Changes are in `dispatch/src/scheduler.rs`, plus one answer in `pipeline.rs`, one seed line in `supervisor.rs` and `Decision::acted_rerun` in `ticket.rs`.

1. **Rerun options.** A new `fn rerun_options` returns `["rerun", "original", "park"]` for a failed or cancelled `Workflow` attempt where `holds_revision()` is true, its copy exists on disk and the currency check in item 4 passes, else the current set. `rerun_ask` (`:9335-9379`) calls it for both its `Failed` and `Cancelled` arms (this covers park then resume); `fail_attempt` (`:7157`) passes it instead of the fixed `["rerun", "park"]`. `rerun` is the default and carries that copy forward; `original` is today's behaviour.
2. **Question text.** `rerun_carries` (`:9382-9392`) gets a `Workflow` arm for the `holds_revision()` case: `` `rerun` reviews <path of the failed attempt's plan.md>, the copy a revise was sent to, and `original` reviews <plan stage's path> again. If the review died while the planner was editing, the copy may be incomplete. `` When `seeded_from` is set and `revisions` is empty, it says "the copy this attempt started from". `ask_rerun` and `fail_attempt` both build their text through it.
3. **Answer handling.**
   - `Decision::acted_rerun` (`ticket.rs:930`) accepts `original` as well as `rerun`, so `may_rerun` (`scheduler.rs:10665`) launches on either.
   - The two matches on `("rerun", "rerun")` (`:3468`, `:3524`) become `("rerun", "rerun" | "original")`. Only the first has a note; `original` carries none.
   - `ANSWERS` (`pipeline.rs:111`) gets `("original", "`rerun`")`, as `keep` and `check` have.
   - `decision_words("rerun")` (`supervisor.rs:69-73`) gains: "`original` reviews the plan stage's copy again, when the question offers it".
4. **`start_workflow`** (`:7436`). Before the copy at `:7458-7471`, let `prev = latest_attempt(t, &stage.name, ctx)`. If it is a failed or cancelled `Workflow` attempt, `holds_revision()` is true, the answer is not `original`, its `plan.md` exists, **and the plan it was copied from is still current**, use that file as `source_path` and set `seeded_from = Some(prev.n)` on the new attempt after the push at `:7476`. A missing carried file logs a warning and falls back to the original.
   - Currency, `subject_unchanged_since(t, &subject, prev)`: no `Complete` attempt that wrote `subject` sits after `prev` in `t.attempts` (chronological; `review_subject` walks it in reverse on that assumption). A plan redone after review/1 started puts a newer `Complete` plan attempt after it, and the check fails.
   - When it fails, copy from `review_subject` as today, `seeded_from` stays `None`, and log a warning naming the newer plan attempt. The `rerun` answer is honoured; only the carry is withheld. `rerun_options` runs the same check, so a plan redone before the question offers no `original`; `start_workflow` checks again because a plan can be redone after it.
   - `source_session` still comes from `review_subject`, unchanged.
5. Don't change the `Complete`-only readers (`review_subject`, `input_where`, `shown_where`): a failed review's copy must never feed a later stage's prompt.

## The UI

`show` only. `dispatch-control` gains `LaneFile.seeded_from: Option<u32>` (struct-level `#[serde(default)]`, `lib.rs:584`), filled in `ticket_paths` (`serve.rs:~539`). `print_lane_files` (`main.rs:896-907`) prints:

```
  plan: .../review/2/joined/plan.md
    (reviewed copy, review open; started from review/1's copy)
```

When `seeded_from` is `None` and a plan review is open, the line says `started from stage plan`.

## The tests

Dispatch scheduler tests, beside the existing review tests near `scheduler.rs:12836`, with the fake app and `Clock::at`:
- Revised review/1 fails, `rerun` is answered: the review/2 copy has review/1's bytes, `seeded_from == Some(1)`, `WorkflowStart.plan` is the review/2 path with plan/1's session as `source`, and the question text names both paths and the incompleteness caveat. Answered `original`: the copy has plan/1's bytes and `seeded_from == None`.
- Plan redone after the failure, then `rerun`: the review/2 copy has plan/2's bytes and `seeded_from == None`. Plan redone before the question: options are `["rerun", "park"]`.
- Review/1 failed unrevised: options `["rerun", "park"]`.
- Chain: review/2 (seeded) fails, `rerun` gives review/3 the review/2 copy; `original` on review/2 makes review/3 unseeded.
- Park then resume of a revised open review: the cancelled attempt's question offers `original` and the rerun carries the copy. A missing carried file falls back to the original.
- Supervisor seed: the `rerun` line of `decision_words` mentions `original`; a pipeline whose `decides` lists `original` is refused with the `ANSWERS` message (tests near `pipeline.rs:2483`).
- `Decision::acted_rerun` is true for `original` (`ticket.rs` tests near `:1819`).
- Store: a record at the previous version loads with `seeded_from == None` and migrates (pattern at `store.rs:971`).
- `serve.rs`: `ticket_paths` fills `seeded_from`, and the `show` line matches.

## Docs and housekeeping

- `docs/dispatch.md`: the version line (see The record).
- `CHANGELOG.md`, under Unreleased: "Dispatch: a plan review rerun after a failed or parked attempt starts from the copy the revise was sent to, and the rerun question and `show` say which copy is reviewed; answer `original` to start from the plan again."

## Follow-ups for the owner

- `rerun --note` on a plan review is dropped (`rerun_note`, `scheduler.rs:8996-8999`); it stays on the decision record only.
- If a reviewer needs the note itself, cloning from review/1's planner session changes the clone-source rule (`src/core/workflow.rs:289`) and `WorkflowStart` has no field for it; a separate decision.

## Commit message

Dispatch: a plan review rerun starts from the owner's revised copy and says which copy it reviews

## Contract additions

- `Attempt.seeded_from: Option<u32>` (serde default); `Attempt::holds_revision()`.
- `RECORD_VERSION`: from the value in `store.rs` at implementation time, plus one; no-transform `migrate` step.
- `rerun` answer `original` (`acted_rerun`, `ANSWERS`, `decision_words("rerun")`).
- `dispatch-control`: `LaneFile.seeded_from: Option<u32>`.

---

# The facts file the planner was given

The plan may cite this file as facts.md or facts:<line>; such a citation is checked against this text, not the tree. It was written by an earlier stage of the same process, not by the owner, so it is not the reference and may itself be wrong.

     1	# Facts for #187
     2	
     3	No spike was run: every open question is either answered by the tree or needs a live run or the owner. Line numbers are in this worktree.
     4	
     5	## Settled
     6	
     7	Claims about the tree (closer read):
     8	
     9	- A rerun of a workflow attempt takes its plan input from `review_subject`, which keeps only `Complete` attempts. A failed `review/1` is skipped and `plan/1` is copied again. — `dispatch/src/scheduler.rs:10534-10548`, called at `:7465`; copy at `:7458-7471`.
    10	- The new attempt always gets its own directory: `attempt_dir` joins ticket dir, stage, `n`, ctx and only calls `create_dir_all`. — `scheduler.rs:6303-6317`. So a rerun never writes into `review/1/joined/`.
    11	- Nothing removes or overwrites a failed or cancelled review attempt's directory. The only `remove_dir_all` over attempt data is `drop_evidence`, which deletes evidence dirs only (`scheduler.rs:10075-10081`; callers `:2447`, `:2539`, `:10436`, all at close or park-cleanup). The only `remove_file` of an artifact is `prepare_writes` (`:10141-10158`), for the attempt being started (a gate-only command's own artifacts, in its own dir). `restart.rs` has no `remove`/`rename`. The one other `remove_file` near review is the refused-`revise` cleanup of the feedback file (`scheduler.rs:7642`). The grep covered `dispatch/src/*.rs`. Queries: `grep -n "remove_dir\|remove_file" dispatch/src/*.rs`, `grep -n "remove\|rename" dispatch/src/restart.rs`.
    12	- Round snapshots (`<data dir>/.../workflows/<run>/round-<n>/`, plan plus feedback and response files) are taken at the end of a reviewer round (`RunState::Converged`/`AwaitingResponse` transitions) and after the planner's response. — `src/core/workflow.rs:657`, `:669`, `:686-698`; `docs/design.md:1076-1082`. A reviser edit made after the last snapshot (the owner's `revise` round) is not snapshotted if the run dies before that round finishes. So `review/1/joined/plan.md` itself is the only durable copy of the revision, and it survives (previous bullet).
    13	- `revise()` writes the owner's note to `review/1/joined/plan.feedback-<round>.md` before sending, and removes it only when Switchboard refuses (`scheduler.rs:7616-7642`). It survives a runner failure on disk. The note is not on the ticket record; `Revision` holds `round`, `by`, `at_ms` only (`dispatch/src/ticket.rs:510-516`).
    14	- `Attempt.revisions` is pushed before the send (`scheduler.rs:7620-7624`) and popped only for a `Reply::Failed` that is not `NO_ANSWER` (`:11285-11313`, pop at `:11313`). A `NO_ANSWER` keeps it with the reply recorded as lost. So non-empty `revisions` on a failed attempt means the revise was sent or its fate is unknown, not that the reviser finished editing. Whether the run opened the round is separate evidence (the feedback file exists, since a refusal deletes it).
    15	- The `Complete`-only readers of the `plan` name: `Ticket::input_where` (`ticket.rs:1452-1463`, used via `lane_input` at `scheduler.rs:10849` and `Ticket::input` in `serve.rs:587`), `Ticket::shown_where` (`ticket.rs:1476-1491`, used by `serve.rs:570` and `lane_files_by` at `scheduler.rs:10899-10925`), and `review_subject`. Existing tests pin only the complete and open cases (`ticket.rs:1819-1892`, `scheduler.rs:12836`). The investigation also names `report.rs` `plan_review` (`:355-380`), not re-read here.
    16	- `rerun_carries` adds text only for `AttemptKind::Review` (and the resolution stage); a `Workflow` attempt's rerun question says nothing about which copy is reviewed. — `scheduler.rs:9382-9390`.
    17	- A park then resume hits the same path: a cancelled workflow attempt is rerun through the same `review_subject`. — `scheduler.rs:9363-9368` per investigation, same predicate.
    18	- The app's rerun clone uses `source` = the session that wrote `plan/1`, not review/1's planner. — `src/core/workflow.rs:289`; investigation notes 4. The owner's note went only to the review run (`ObjectWorkflow`, `src/core/workflow.rs:969-1000`). So the clone cannot see the revision by transcript; only the file carries it.
    19	- `LaneFile`/`PathsView` are built in `dispatch/src/serve.rs`; `show` prints them at `dispatch/src/main.rs:~896-993`. A new field there is a wire change in the dispatch control types and must be additive (CLAUDE.md, "Extending the contract").
    20	
    21	Claims about the world outside the tree:
    22	
    23	- `claude --resume <id> --fork-session` starts a new session id from the resumed state and leaves the original intact. Evidence: Anthropic's Agent SDK session docs (https://docs.claude.com/en/api/agent-sdk/sessions) say resuming continues the original by default and the fork option "create[s] a new session ID that starts from the resumed state"; third-party guides agree the history is copied. Covers Claude Code through the 2025 releases cited (2.0.x); no newer source was read. Queries: `claude code --resume --fork-session copies conversation history new session id`. This does not bear on the plan: Switchboard clones by copying a transcript prefix and a fresh `sessionId`, not by the flag (`spikes/07-clone-session/README.md`, summary in `spikes/README.md`).
    24	- The issue's claims about a browser, platform or crate: there are none. The only external behaviour is the runner restart that ends a run, which is Switchboard's own app (see Unresolved).
    25	
    26	## Unresolved
    27	
    28	- Why the app answers `no such run` after the restart (run missing from the store, not reloaded, or pruned: `run_view`, `src/core/control.rs:865`; store load around `src/adapters/store.rs:1082`). Tried: the `NO_SUCH_RUN` path in the scheduler only; the app-side store code was not read, and it needs a runner restart to observe. Build to: the plan must treat `review run gone` as terminal for the attempt either way (it is today, `scheduler.rs:7757-7763`), and must not depend on re-attaching the run. If the run turns out recoverable, that is a separate change.
    29	- Whether `review/1/joined/plan.md` is complete when the run is killed mid-edit. Cannot be known without a live kill. Build to: carry-forward is only safe if the plan adds a rule the owner can see (the rerun question names the copy that will be reviewed and says it is a revised one), so a half-written copy is the owner's call; the plan must not copy it silently. If it is always complete, the question can say so; the plan adds the same visibility either way.
    30	- A fresh clone of `plan/1`'s planner starting without the revision's context is expected from the code (above) but not observed live. Build to: if a carried-forward copy is reviewed, the clone's planner still has `plan/1`'s transcript only; the plan must not assume it knows the owner's note, and should hand the note file's path or text in explicitly if the reviser prompt needs it.
    31	
    32	## For the owner
    33	
    34	- Does `review/2/joined/plan.md` differ from `review/1/joined/plan.md` in the way the issue says, and does `review/1/joined/plan.feedback-<N>.md` hold the note as quoted? Favourable (matches): the code reading above is the cause; the plan fixes the input choice. Unfavourable (the files are equal or the note is missing): the plan must still make the rerun's input and `show` say which copy is reviewed, and must not rely on `revisions` alone to tell a revised copy.
    35	- Is the revised plan in `review/1` the one the owner approved? Favourable: carry it forward. Unfavourable: the plan must still let the owner choose the original at the rerun question; works in both cases.
