# Plan: a review rerun starts from the owner's revised copy (#187)

Line references are to this worktree at 7fe8949.

## Where this departs from the issue

- The issue asks for `show` to list each attempt's `plan.md` and say
  which one is the reviewed copy and which is a prior revision. Once the
  rerun starts from the revised copy, attempt 2's copy holds the revision
  and listing attempt 1's copy as well would show two near-identical
  plans. Instead the plan line for the open copy says which copy it
  started from and which owner round came with it (see UI).
- "The `revise` note should survive a runner failure the same way an
  answered decision does": the note's file survives
  (`review/1/joined/plan.feedback-<n>.md`, and `Attempt::revisions`), but
  the plan does not hand it to the new run. `WorkflowStart` has no field
  for an opening objection, and an objection is only sent to a run that
  is `Converged` or `AtCap` (the check in the `revise` path,
  `scheduler.rs:7608-7614`), so delivering it would be a wire change on
  both sides. Instead the plan makes sure the note is never dropped
  silently: when the planner never answered it (no
  `plan.response-<n>.md`), the rerun question, the log line, `show`, and
  the rerun's own `finalize` question name the note's file and say it
  was not answered, so the owner can `revise --file <that file>` once
  the rerun converges.

## The record

No record change. `RECORD_VERSION` stays as it is (28 in
`dispatch/src/store.rs:26` today). The failed attempt already holds what
the fix reads: `kind: Workflow`, `state`, `ended_ms`, `artifacts[subject]`,
and `revisions` (`dispatch/src/ticket.rs:350`, `Revision` at
`ticket.rs:510`); the ticket holds `restarts`. Where a copy came from is
recomputed from the attempts before it, so nothing new is stored.

## The signals

No new signal.

## The rule

A new review attempt of stage `s` in context `ctx` carries a revised
copy only when the review it replaces was *interrupted*, not when it was
*undone*. Walk the attempts of `s` in `ctx` (only that stage and that
context, so a per-lane review never stops at another lane's attempt),
newest first, and stop at the first one that:

- is `Complete`;
- is `Cancelled` with a reason starting with `DISCARDED_BY`
  (`dispatch/src/restart.rs:34`, set by `restart::discard`,
  `restart.rs:839-857`) or `SENT_BACK_FROM` (`scheduler.rs:8979`, set by
  `send_back`, `scheduler.rs:4960-4999`);
- ended before the ticket's last restart
  (`t.restarts.last().map_or(0, |r| r.at_ms)`, compared with
  `ended_ms.unwrap_or(started_ms)`, the cutoff `ask_rerun` uses at
  `scheduler.rs:7230-7241`). A Failed attempt that a restart left Failed
  is cut off here.

The attempts walked before stopping are all Failed, or Cancelled by a
park or a runner restart. If any of them has a non-empty `revisions`,
the source is the copy of the *newest* walked attempt whose copy is
still on disk. A second failure is covered this way: `review/2` has no
revisions of its own, but `review/1` (walked after it) has, so
`review/3` starts from `review/2`'s copy, which is `review/1`'s revised
copy plus whatever `review/2`'s planner wrote. `carried` names that
newest attempt; `unanswered` comes from the newest walked attempt with
revisions. No walked attempt has revisions: today's pick, unchanged.

What that means in each case:

- Run gone, a failed pause, a park then resume, a runner restart
  mid-run: carries.
- `restart` at the review stage or earlier: does not carry. A restart
  discards the later work (`restart.rs:1-9`).
- A send-back to the review stage from a later gate: does not carry,
  which is today's behaviour. The send-back asks for that review to be
  done again, and the finalized copy it cancels is the thing being
  objected to. The choice is stated in design.md and pinned by a test.
- A failed attempt with reviewer rounds but no owner `revise`: does not
  carry, as today and as existing tests expect.

The session handed to `WorkflowStart.source` is still the one
`review_subject` picks today, the complete writer's. The failed
attempt's planner clone was killed by `retire_replaced`
(`scheduler.rs:3578`), and cloning a conversation whose run died
mid-revise is a new mechanism this issue does not need.

`review_subject`'s own walk ignores context (`scheduler.rs:10534-10547`),
so in a per-lane pipeline it can hand one lane's review another lane's
plan. That is an existing bug outside this issue; the carry walk above
is per context either way.

`Ticket::input_where` (`ticket.rs:1452`) does not change. A later agent
stage's `{inputs.plan}` still reads only a finalized plan.

## The actions and effects

All in the dispatch crate.

1. Beside `review_subject` (`scheduler.rs:10534`), add:

   ```rust
   /// Where a plan review attempt's copy comes from.
   pub(crate) struct ReviewSource {
       pub path: PathBuf,
       /// The session the planner clones: always the complete writer's.
       pub session: String,
       /// The interrupted review attempt whose revised copy this is;
       /// none when it is the complete writer's own file.
       pub carried: Option<u32>,
       /// That attempt's newest owner round file the planner never
       /// answered (no response file beside it).
       pub unanswered: Option<PathBuf>,
   }

   pub(crate) fn review_source(
       t: &Ticket, before: usize, stage: &str, ctx: &str, subject: &str,
   ) -> Result<ReviewSource, String>
   ```

   `before` is the number of attempts it may look at (`t.attempts.len()`
   for the next launch, an attempt's index for `show`). It runs the walk
   in "The rule", then calls `review_subject` for the session, keeping
   its error strings. `unanswered` uses `report::round_file` and
   `report::response_file` (`dispatch/src/report.rs:156`, `:202`).
2. `start_workflow` (`scheduler.rs:7436`, the call at `7462`): use
   `review_source(t, t.attempts.len(), &stage.name, ctx, &subject)`,
   copy `source.path` as today (`7469-7472`). When `carried` is set, log
   "review/n starts from review/k's revised copy", adding "; the owner's
   round in <file> was never answered" when `unanswered` is set.
3. `rerun_carries` (`scheduler.rs:9382`, called from `ask_rerun` at
   `7266` and from `rerun_ask` at `9373`, which serves both the Failed
   and the Cancelled arms, `9336-9366`): return `String` and add a
   `Workflow` arm from `review_source` with the failed attempt included:
   " A rerun reviews {stage}/{k}'s revised copy, {path}." plus
   " The owner's round in {file} was never answered; revise with --file
   {file} once it converges." when `unanswered`, else
   " A rerun reviews {writer stage}'s {subject} from the start."
4. `review_done`'s `finalize` question (`scheduler.rs:7693-7696`): when
   the attempt's own `review_source` has `unanswered`, append the same
   "never answered" sentence, so the note is in front of the owner at
   the point they can resend it. The sentence goes once the owner has
   acted on it: a small helper beside `review_source`,
   `fn still_unanswered<'a>(a: &Attempt, s: &'a ReviewSource) -> Option<&'a PathBuf>`,
   returns `None` when `a.revisions` is not empty (the owner has
   revised this attempt, with the old note or a new one) and
   `s.unanswered` otherwise. Step 4 and `ticket_paths` both read it, so
   the question and `show` stop together. Steps 2 and 3 do not need it:
   they describe a launch, whose new attempt has no revisions yet, and
   when an attempt that was revised fails in its turn, the walk finds
   *its* revisions first and `unanswered` is computed from its own
   rounds.
5. The comment at `scheduler.rs:7812-7814` ("a rerun starts a fresh
   review") states the new rule: a fresh reviewer, on the newest copy
   the owner revised, unless a restart or send-back undid it.

## The UI

`show` only; the app's Plan tab is unchanged (its open copy now holds
the revision).

- `dispatch-control/src/lib.rs:586` `LaneFile`: add
  `started_from: Option<PathBuf>` and `unanswered: Option<PathBuf>`.
  `#[serde(default)]` is already on the struct, so older clients read
  them.
- `dispatch/src/serve.rs:538` `ticket_paths`: for a plan file written
  by a `Workflow` attempt, fill both from `review_source` over the
  attempts before that attempt, `unanswered` through
  `still_unanswered` (step 4), so it is `None` once that attempt has a
  revision of its own.
- `dispatch/src/main.rs:896` `print_lane_files`: under the line, print
  `    (started from <path>)` and
  `    (owner round never answered: <file>)` when set.

## The tests

In `dispatch/tests/first_slice.rs`, using `at_finalize` (`3365`),
`revise` (`3417`), `artifact_of` (`395`), `park_and_resume` (`10584`),
`restart_at` (`14660`), and the run-gone step from
`a_run_switchboard_no_longer_has_fails_the_attempt` (`3809`,
`env.sb().runs.clear()`). "Revised text" below means distinct text
written into `review/1`'s copy, standing in for the planner, plus its
`plan.response-<n>.md`.

- `a_rerun_after_revise_reviews_the_revised_copy`: revise, revised
  text, clear runs, step, answer `rerun`, step. `review/2`'s copy equals
  `review/1`'s, `WorkflowStart.plan` is the new copy, and
  `WorkflowStart.source` is still the plan stage's session.
- `a_rerun_question_after_revise_names_the_revised_copy`: the pending
  `rerun` question contains `review/1's revised copy` and the path.
- `a_second_failed_rerun_still_carries_the_revision`: fail `review/2`
  the same way after writing more text into its copy, rerun;
  `review/3`'s copy equals `review/2`'s, not `plan/1`'s.
- `a_revise_the_planner_never_answered_is_named_in_the_rerun`: revise,
  clear runs before any copy change or response file. The rerun
  question and `review/2`'s `finalize` question name the owner's round
  file and say it was never answered.
- `a_resent_note_is_no_longer_named_as_unanswered`: continue that
  test: answer `review/2`'s `finalize` with `revise --file <the named
  file>`, let the run converge again. The second `finalize` question
  does not contain the file or "never answered", and `ticket_paths`
  gives the open copy `unanswered: None`.
- `a_parked_review_after_revise_resumes_on_the_revised_copy`: revise,
  revised text, `park_and_resume`. The question comes through
  `rerun_ask`'s Cancelled arm and names the revised copy; answering
  `rerun` starts `review/2` from it.
- `a_restart_at_review_after_revise_starts_from_the_plan_stage`:
  revise, revised text, clear runs so `review/1` fails, then
  `restart_at(.., Some("review"))` and rerun. `review/2`'s copy equals
  `plan/1`'s.
- `a_send_back_to_review_starts_from_the_plan_stage`: a pipeline with a
  gate after review; revise, revised text, finalize, send back to
  review. The new attempt's copy equals `plan/1`'s.
- `a_rerun_with_no_revise_starts_from_the_plan_stage`: pins today's
  behaviour; the question says "from the start".
- `a_later_stage_reads_the_finalized_plan_not_a_failed_revision`: an
  agent stage after review; before the rerun completes, `input_where`
  is still `plan/1`'s; after it, `{inputs.plan}` is `review/2`'s
  finalized copy.

In `serve.rs`'s tests (`mod tests` at `996`):

- `ticket_paths` sets `started_from` and `unanswered` on the open copy
  after a carried rerun and leaves them `None` otherwise, and leaves
  `unanswered` `None` when the open attempt has a `revisions` entry.
- A per-lane variant of the lane fixture at `serve.rs:1521-1568`: a
  failed revised `review-plan` attempt in lane `repo` and a newer
  complete one in `docs`; `review_source` for `repo` still carries
  `repo`'s revised copy.

## Docs

- `docs/design.md`, "Plan review workflow", "As built (steps 1 and 2)"
  (`docs/design.md:1166`): one bullet: a rerun after a failed or parked
  review reviews the newest copy the owner revised, and names an owner
  round the planner never answered; a restart or a send-back to the
  review stage starts from the plan stage's copy.
- `CHANGELOG.md` Unreleased: "A plan review rerun after a failed or
  parked attempt starts from the owner's revised copy, and the rerun
  question and `show` say which copy it reviews and whether the owner's
  note was answered."
- README: no new file, no new dev aid.

## Commit

`Dispatch reruns a failed plan review from the owner's revised copy`

Contract additions:

- `dispatch_control::LaneFile::started_from: Option<PathBuf>`
- `dispatch_control::LaneFile::unanswered: Option<PathBuf>`
