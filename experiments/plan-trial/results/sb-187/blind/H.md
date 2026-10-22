# A review rerun starts from the stage's newest copy (#187)

## Where this departs from the issue

- **"The rerun starts from the plan stage's original output."** Borne
  out, and slightly wider than the issue says. `start_workflow`
  (`dispatch/src/scheduler.rs:7436`) takes the subject from
  `review_subject` (`dispatch/src/scheduler.rs:10534`), which picks the
  latest *complete* attempt that wrote it. A failed review attempt is
  never complete, so every rerun of a plan review copies the upstream
  stage's file again, revised or not. The loss is not only the owner's
  revision: the planner edits the copy in place in every round
  (`docs/dispatch.md`, "A review is a copy"), so a failed attempt's copy
  carries the reviewer's rounds too. The plan therefore starts a rerun
  from the stage's newest copy in the context, whether or not a `revise`
  touched it.
- **"The `revise` note should survive a runner failure the way an
  answered decision does."** It already does, as a record: the round
  file `revise` writes (`dispatch/src/scheduler.rs:7616`), the
  `revisions` entry on the attempt (`dispatch/src/ticket.rs:350`) and
  the `revised` event all outlive the failure. What is lost is its
  effect. The plan carries the revised copy, not the note: a fresh run
  has no way to take an objection (`workflow.object` is refused unless
  the run is converged or at cap). When the owner's round has no
  response file beside the failed copy, the planner never answered it,
  and the rerun question says so rather than pretending to carry it; the
  owner answers `revise` again once the new run converges.
- **"`show` names only the attempt-2 copy as the plan."** Borne out:
  `Ticket::shown_where` (`dispatch/src/ticket.rs:1476`) lists the open
  review's copy and skips a failed attempt's. The plan keeps that rule
  (the open copy is the plan as it stands) and adds where it started
  from.
- **The source session.** The issue does not say whose conversation the
  rerun's planner should clone. The plan keeps the plan stage's session,
  as today: the earlier attempt's planner clone may be gone with the
  run, and the prompts name the copy and say to edit only that, so the
  revised text is what the new run reads.

## The record

No field is added and `RECORD_VERSION` (`dispatch/src/store.rs:26`, 28
today) does not move. A plan review attempt that starts from an earlier
attempt's copy records it in `Attempt.carried_from`
(`dispatch/src/ticket.rs:317`), today written only for a code review
attempt, as `(stage, n)` of the attempt whose copy was copied. Its doc
comment widens to both kinds; its readers (`report.rs:416`,
`review.rs:3646`) already filter on `AttemptKind::Review`, so nothing
reads a workflow attempt's value by mistake. The copy itself is the new
attempt's `artifacts["plan"]`, as now.

The choice of copy is a pure function in `scheduler.rs` beside
`review_subject`:

```
fn prior_copy(t: &Ticket, stage: &str, ctx: &str, n: u32, subject: &str) -> Option<&Attempt>
```

The latest earlier attempt of `stage` in `ctx` (`a.n < n`, any state)
whose `artifacts[subject]` exists on disk, is not in `forgotten`, and
started at or after the last restart's `at_ms` (`Ticket.restarts`,
`dispatch/src/ticket.rs:1155`; a restart discards later work, as
`fail_attempt_with` already counts failures by that boundary at
`scheduler.rs:7232`). `None` when the `rerun` decision that authorised
attempt `n` (the acted one `may_rerun` finds, `scheduler.rs:10665`)
carries a note that `starts_over` accepts (`dispatch/src/review.rs:3639`,
made `pub(crate)`), mirroring the code review's "start over".

`start_workflow` copies from `prior_copy`'s file when there is one,
else from `review_subject`'s as today, keeps `review_subject`'s session
as the source either way, and sets `carried_from` on the attempt it
pushes (`scheduler.rs:7487`).

## The signals

- The `rerun` question for a failed or cancelled plan review attempt
  (`fail_attempt_with`, `scheduler.rs:7263`, and `rerun_ask`,
  `scheduler.rs:9335`, both through `rerun_carries`,
  `scheduler.rs:9382`, which today says nothing for a workflow attempt)
  gains: " A rerun reviews this attempt's copy at <path>, with the
  rounds so far; a note saying \"start over\" reviews the <subject>
  stage's output again." When the attempt's newest `revisions` round has
  no `response_file` (`dispatch/src/report.rs:202`) beside the copy, it
  adds: " The owner's round <n> was not answered; revise again once the
  review converges." `rerun_carries` needs the ticket's attempt, which it
  has.
- The `attempt-started` event text (`dispatch/src/events.rs:695`) for an
  attempt with `carried_from` set reads `joined #2 workflow from #1's
  copy`; the code review's already-recorded carry gets the same suffix
  for free, which is a wording change to an event line, not a new kind.

No new event kind, no new decision.

## The actions and effects

`dispatch-control` (`dispatch-control/src/lib.rs`), additive, serde
default:

- `LaneFile.from: Option<PriorCopy>` (`lib.rs:586`): the earlier
  attempt's copy this review started from; none when it started from the
  upstream stage's output.
- `PriorCopy { path: PathBuf, attempt: u32, owner_rounds: Vec<u32> }`:
  the copy, its attempt number, and the rounds the owner opened on it
  (`Attempt.revisions`), so a reader tells "the prior revision" from a
  copy the reviewer alone edited.

`ticket_paths` (`dispatch/src/serve.rs:538`) fills `from` on each plan
`LaneFile` from the writing attempt's `carried_from`, looked up with
`find_attempt`. Nothing else on the port changes; `plan_rounds` keeps
listing the open attempt's rounds, since the earlier attempt's round
files stay beside its own copy and the attempt is still listed under
`attempts`.

## The UI

- `show`'s files block (`dispatch/src/main.rs:896`, `print_lane_files`)
  prints under a plan line that has `from`:
  `    (started from attempt 1's copy <path>, which carries the owner's
  round 2)` or, with no owner rounds, `    (started from attempt 1's
  copy <path>)`. The existing `(reviewed copy, round N, review open)`
  line stays above it. `--json` carries the field as is.
- The ticket page's plan label (`src/ui/ticket.rs:425`, `plan_label`)
  appends `, started from attempt 1's copy` when the plan's `LaneFile`
  in `plan_files` has `from`. One clause; no new widget.

## The tests

- `dispatch/tests/first_slice.rs`, beside
  `a_run_switchboard_no_longer_has_fails_the_attempt`: from
  `at_finalize` (`first_slice.rs:3365`) answer `revise` (the `revise`
  helper, `first_slice.rs:3417`), write a revised text into the review
  attempt's copy and the round's response file as the planner would,
  clear the fake's runs so the attempt fails with `review run gone`,
  assert the `rerun` question names the copy, answer `rerun`, and step
  to the new run. Assert the `workflow.start` body's `plan` is attempt
  2's copy, its text is the revised text, `source` is the plan stage's
  session, `carried_from == Some(("review", 1))`, and `ticket_paths`
  lists `from` with `attempt 1` and `owner_rounds == [2]`.
- Same start, `rerun` with the note "start over": the new copy is the
  plan stage's text, `carried_from` is `None`, `from` is `None`.
- Same start without the response file: the question says the owner's
  round was not answered.
- `scheduler.rs` unit tests for `prior_copy`: a copy from before the
  last restart is not taken; a forgotten or missing file is not taken;
  the latest earlier attempt wins over an older one.
- `dispatch/src/serve.rs`, beside the `plan_files` tests
  (`serve.rs:1385`): a workflow attempt with `carried_from` yields
  `from` on its `LaneFile`; a plain one yields none.
- `events.rs` unit test: an attempt with `carried_from` starts with
  "from #1's copy" in its line.

Docs in the same commit: the "A review is a copy" bullet in
`docs/dispatch.md` (line 550) gains the rerun sentence and the "start
over" escape; a row in its test table beside the `review run gone` row
(line 3078) names the three new tests; `docs/design.md` gets a short
"Review rerun from the newest copy (2026-10-21)" section before "Open
questions", and its ticket-page Plan bullet (line 2160) the label
clause; the README Layout line for `dispatch-control/` (line 448)
mentions the copy a review started from; a CHANGELOG line under
Unreleased. The app runs at every step: the field is serde default, so
an older `dispatch` or app reads the records and views unchanged.

Commit: `Dispatch reruns a plan review from the stage's newest copy, keeping the owner's revision, and show says which copy a review started from`

Contract additions (copy into the commit message):

- `dispatch-control`: `LaneFile.from: Option<PriorCopy>`; new
  `PriorCopy { path, attempt, owner_rounds }`.
- `dispatch::ticket`: `Attempt.carried_from` now also set on a plan
  review attempt that started from an earlier attempt's copy; no new
  field, `RECORD_VERSION` unchanged (take the value in `store.rs` at
  implementation time; it stays).
- `dispatch::review::starts_over` made `pub(crate)`.
