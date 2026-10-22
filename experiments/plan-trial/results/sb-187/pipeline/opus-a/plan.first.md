# Plan: a review rerun starts from the owner's revised copy (#187)

Line references are to this worktree at 7fe8949.

## Where this departs from the issue

- The issue asks for `show` to list each attempt's `plan.md` and say
  which one is the reviewed copy and which is a prior revision. Once the
  rerun starts from the revised copy, attempt 2's copy holds the revision
  and listing attempt 1's copy as well would show two near-identical
  plans. Instead the plan line for the open copy says which copy it
  started from (see UI). Attempt 1's owner round files stay out of
  `plan_rounds`; the revised plan already holds what they asked for.
- "The `revise` note should survive a runner failure the same way an
  answered decision does": the note was never lost. It is in
  `review/1/joined/plan.feedback-<n>.md` and in `Attempt::revisions`.
  What got lost is the revised plan, because the copy is chosen from
  Complete attempts only. The plan fixes the copy choice and does not
  move the note.

## The record

No record change. `RECORD_VERSION` stays as it is (28 in
`dispatch/src/store.rs:26` today). The failed attempt already holds what
the fix reads: `kind: Workflow`, `state: Failed`/`Cancelled`,
`artifacts[subject]`, and a non-empty `revisions`
(`dispatch/src/ticket.rs:350`, `Revision` at `ticket.rs:510`). Where a
copy came from is recomputed from the attempts before it, so nothing new
is stored.

## The signals

No new signal.

## The rule

A new review attempt starts from the newest plan the stage holds:

- Walk `t.attempts` newest first, stopping at the newest Complete
  attempt that wrote `subject` (today's pick in `review_subject`,
  `dispatch/src/scheduler.rs:10534`).
- Before reaching it, the first `Workflow` attempt of this stage and
  context that is `Failed` or `Cancelled`, whose copy still exists on
  disk, and that *carries* wins. It carries when its `revisions` is not
  empty, or when it itself started from a carried copy (the same rule
  run on the attempts before it). The second condition matters when a
  second attempt fails too: attempt 3 must not drop back to `plan/1`.
- The session handed to `WorkflowStart.source` is still the Complete
  writer's. The failed attempt's planner clone was killed by
  `retire_replaced` (`scheduler.rs:3578`), and cloning a conversation
  whose run died mid-revise is a new mechanism this issue does not need.
  The planner is handed the revised file as the plan, which is enough.
- A failed attempt with reviewer rounds but no owner `revise` still
  starts fresh from the plan stage, as today and as existing tests
  expect.
- `Ticket::input_where` (`ticket.rs:1452`) does not change. A later
  agent stage's `{inputs.plan}` still reads only a finalized plan; a
  failed review's copy is a starting point for another review, not a
  finished plan.

## The actions and effects

All in the dispatch crate.

1. `scheduler.rs:10534`: replace the body of `review_subject` with a
   pure-ish helper (it checks the copy exists on disk, nothing else):

   ```rust
   /// Where a plan review attempt's copy comes from.
   pub(crate) struct ReviewSource {
       pub path: PathBuf,
       /// The session the planner clones: always the complete writer's.
       pub session: String,
       /// The failed or cancelled review attempt whose revised copy
       /// this is; none when it is the complete writer's own file.
       pub carried: Option<u32>,
   }

   pub(crate) fn review_source(
       attempts: &[Attempt], stage: &str, ctx: &str, subject: &str,
   ) -> Result<ReviewSource, String>
   ```

   It takes a slice, not the ticket, so `show` can ask "where did
   attempt n start from" by passing the attempts before it. Keep
   `review_subject`'s error strings ("nothing wrote {subject}", "{subject}
   was not written by a session").
2. `start_workflow` (`scheduler.rs:7436`, the call at `7462`): call
   `review_source(&t.attempts, &stage.name, ctx, &subject)`, copy
   `source.path` as today (`7469-7472`). When `carried` is `Some(k)`,
   send a log line through the existing event path saying "review/n
   starts from review/k's revised copy" so the timeline shows it.
3. `rerun_carries` (`scheduler.rs:9382`, called at `7266` and `9373`):
   return `String` instead of `&'static str`, and add a `Workflow` arm.
   It calls `review_source` with the ticket's attempts (the failed one
   included, as it will be when the rerun starts) and says
   " A rerun reviews {stage}/{k}'s revised copy, {path}." when carried,
   else " A rerun reviews {writer stage}'s {subject} from the start."
   Using the same helper as the launch means the question and the launch
   cannot disagree.
4. The comment at `scheduler.rs:7812-7814` ("a rerun starts a fresh
   review") states the new rule: a fresh reviewer, on the newest copy
   the owner revised.

## The UI

`show` only; the app's Plan tab is unchanged (its open copy now holds
the revision).

- `dispatch-control/src/lib.rs:586` `LaneFile`: add
  `started_from: Option<PathBuf>`, the revised copy an open or finished
  review copy started from. `#[serde(default)]` is already on the
  struct, so older clients read it.
- `dispatch/src/serve.rs:538` `ticket_paths`: for a plan file written
  by a `Workflow` attempt, set `started_from` from `review_source` over
  the attempts before that attempt, when `carried` is set.
- `dispatch/src/main.rs:896` `print_lane_files`: print
  `    (started from <path>)` under the line when set, beside the
  existing "(reviewed copy, round N, review open)" note.

## The tests

In `dispatch/tests/first_slice.rs`, using `at_finalize` (`3365`),
`revise` (`3417`), `artifact_of` (`395`), and the run-gone step from
`a_run_switchboard_no_longer_has_fails_the_attempt` (`3809`,
`env.sb().runs.clear()`):

- `a_rerun_after_revise_reviews_the_revised_copy`: at finalize, revise
  with a note, write distinct text into `review/1`'s copy (standing in
  for the planner), clear runs, step, answer `rerun`, step. `review/2`'s
  copy equals `review/1`'s, `WorkflowStart.plan` is the new copy, and
  `WorkflowStart.source` is still the plan stage's session.
- `a_rerun_question_after_revise_names_the_revised_copy`: the pending
  `rerun` question contains `review/1's revised copy` and the path.
- `a_second_failed_rerun_still_carries_the_revision`: fail `review/2`
  the same way, rerun; `review/3`'s copy equals `review/1`'s text.
- `a_rerun_with_no_revise_starts_from_the_plan_stage`: pins today's
  behaviour; the question says "from the start".
- `a_later_stage_reads_the_finalized_plan_not_a_failed_revision`: a
  pipeline with an agent stage after review; after a failed revised
  attempt and a completed rerun, its `{inputs.plan}` is `review/2`'s
  finalized copy, and before the rerun completes `input_where` is still
  `plan/1`'s.

In `serve.rs`'s tests (`mod tests` at `996`): `ticket_paths` sets
`started_from` on the open copy after a carried rerun and leaves it
`None` otherwise.

## Docs

- `docs/design.md`, "Plan review workflow", "As built (steps 1 and 2)"
  (`docs/design.md:1166`): one bullet on what a rerun after a failed or
  cancelled attempt reviews.
- `CHANGELOG.md` Unreleased: "A plan review rerun after a failed attempt
  starts from the owner's revised copy, and the rerun question and
  `show` say which copy it reviews."
- README: no new file, no new dev aid.

## Commit

`Dispatch reruns a failed plan review from the owner's revised copy`

Contract additions:

- `dispatch_control::LaneFile::started_from: Option<PathBuf>`
