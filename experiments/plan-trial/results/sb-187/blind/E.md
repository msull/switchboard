# A review rerun starts from the revised copy (issue #187)

A workflow stage's rerun after a failed attempt starts the new run from
the newest copy of the subject that carries the owner's revision, names
that copy in the `rerun` question, and `show` lists it under the
reviewed copy as a prior revision.

## Where this departs from the issue

- **The revise note already survives the failure.** The note is on the
  answered `finalize` decision, in the attempt's `revisions`
  (`dispatch/src/ticket.rs:347-350`), and in the round file written
  beside the copy (`dispatch/src/scheduler.rs:7616-7618`). What the
  rerun discards is the copy the planner edited in place after reading
  it (`dispatch/src/ticket.rs:1471-1474`, `docs/dispatch.md:550-557`).
  The plan therefore does not re-send the note to the new run: the new
  run reviews the copy the note produced. A run that died before the
  planner wrote anything leaves a copy equal to the original, and
  `show`'s new line points at the attempt directory whose round file
  still holds the note.
- **"The newest plan the stage holds" is a chain, not the previous
  attempt.** After attempt 2 starts from attempt 1's revised copy and
  fails too, attempt 3 starts from attempt 2's copy, since it holds the
  revision plus whatever the planner did in attempt 2. The record says
  which attempt a copy came from, so the chain is read, never guessed.
- **A plan written after the revision wins.** `review_subject`
  (`dispatch/src/scheduler.rs:10534-10548`) takes the newest complete
  writer of the subject across every stage. A restart that runs the plan
  stage again writes a newer plan than any revised copy, and the rerun
  must review that. The revised copy is preferred only when its attempt
  started after that writer's.

Everything else in the issue the code bears out: the copy is made at
`dispatch/src/scheduler.rs:7470-7472` from `review_subject`, which
skips failed attempts; `rerun_carries`
(`dispatch/src/scheduler.rs:9382-9390`) adds nothing for a workflow
stage; and `ticket_paths` (`dispatch/src/serve.rs:538-547`) lists one
plan file, the open attempt's copy, because `shown_where`
(`dispatch/src/ticket.rs:1476-1491`) drops a failed attempt.

## The record

`Attempt` gains one field, beside `carried_from`
(`dispatch/src/ticket.rs:316-318`), with the same discipline: written
when the attempt is made and never changed.

```rust
/// A workflow attempt started from an earlier attempt's copy of the
/// subject rather than the stage that wrote it: that attempt, of the
/// same stage and context, whose copy carried the owner's revision
/// (its own `revisions`, or started from one that did). None for a
/// copy of the writer's file.
#[serde(default)]
pub revised_from: Option<(String, u32)>,
```

`new_attempt` (`dispatch/src/scheduler.rs:10277`) sets it to `None`;
`start_workflow` sets it after the push when it copied a revised copy.

The bump: `RECORD_VERSION` in `dispatch/src/store.rs:26` goes from its
value at implementation time (28 today) to that plus one. The migrate
loop (`dispatch/src/store.rs:454`) gets a comment step after the 27 to
28 one (`dispatch/src/store.rs:608-612`): an attempt gains
`revised_from`, absent from its serde default; nothing is transformed;
a build that would drop it must refuse the record, or a rerun would go
back to the writer's file. `docs/dispatch.md:396-401` gets the matching
"Version N adds an attempt's `revised_from`" line.

The wire contract (`dispatch-control/src/lib.rs`) gains, on `PathsView`
(`dispatch-control/src/lib.rs:543-580`), additively:

```rust
/// The revised copies the shown plan descends from, newest first:
/// each a workflow attempt's copy that a later attempt of the same
/// stage started from, as `Attempt::revised_from` chains them.
pub plan_prior: Vec<PriorCopy>,
```

```rust
/// One earlier copy of the plan a rerun started from.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PriorCopy {
    pub stage: String,
    pub n: u32,
    pub path: PathBuf,
    /// The round the owner's revision opened, from `revisions`; none
    /// when the copy was itself started from a revised one.
    pub round: Option<u32>,
    /// Who answered `revise`, as the revision says: `you` or `supervisor`.
    pub by: Option<String>,
}
```

`LaneFile` is unchanged: a per-lane plan is never a workflow's
(`dispatch/src/scheduler.rs:10900-10906` picks one copy per lane from
per-lane stages, and a workflow stage runs once).

## The signals

- **The `rerun` question** names the copy. `rerun_carries`
  (`dispatch/src/scheduler.rs:9382`) returns a `String` and, for a
  workflow stage whose next attempt would start from a revised copy,
  says ` The next attempt reviews the revised copy at <path> (round
  <n>, revised by <by>), not the plan stage's file.` It is computed by
  the same function the launch uses (below), so the question and the
  launch agree. Both callers already format it into a `String`
  (`dispatch/src/scheduler.rs:7257-7267` in `fail_attempt_with`, and
  `dispatch/src/scheduler.rs:9368-9375` in `rerun_ask`, which the resume
  path reuses), so a resume's `rerun` question says it too.
- **The `attempt-started` event** text (`dispatch/src/events.rs:690-697`)
  gains `, from <stage>/<n>'s revised copy` when `revised_from` is set.
  No test asserts that text today.
- **A warning log** when the preferred copy is missing on disk (the
  attempt directory was removed by hand): the launch falls back to the
  writer's file and `log::warn!` names the missing path. No decision.

No new event kind, no new decision.

## The actions and effects

Dispatch has no `Effect`s; this is all in the scheduler.

- **`review_subject`** (`dispatch/src/scheduler.rs:10534`) takes the
  stage name and context as well as the subject and returns, beside
  the path and the source session, which attempt the copy came from:
  `Option<(String, u32)>`. It finds the newest complete writer as now,
  then looks for a newer workflow attempt of that stage and context
  that is ended (`Failed` or `Cancelled`, not `is_open`, not
  `Complete`), holds the subject in `artifacts`, has not forgotten it,
  and carries the owner's revision: its `revisions` is non-empty or its
  `revised_from` is set. "Newer" is `started_ms` later than the
  writer's. The newest such copy whose file `is_file()` wins; a copy
  missing on disk is skipped with the warning above. The source session
  stays the writer's: the planner clone is made from the plan session's
  transcript (`dispatch/tests/support/mod.rs:397-404`), which a copy
  does not change. A small pure helper `revised_copy(t, stage, ctx,
  subject) -> Option<&Attempt>` holds the search so `rerun_carries` and
  `ticket_paths` can call it; it lives beside `review_subject`.
- **`start_workflow`** (`dispatch/src/scheduler.rs:7436`) passes
  `&stage.name` and `ctx` to `review_subject`, copies the path it
  returns as now, and sets `revised_from` on the attempt it pushes at
  `dispatch/src/scheduler.rs:7487-7495`. Nothing else in the launch
  changes: the attempt directory, the reviewer's cwd and args, and the
  `workflow.start` body are the same, with `plan` the new copy.
- **`ticket_paths`** (`dispatch/src/serve.rs:538`) fills `plan_prior`
  by walking `revised_from` from the shown plan's attempt (`shown` at
  `dispatch/src/serve.rs:573`): for each link, the attempt's copy path,
  its newest revision's `round` and `by` when it has one, newest first.
  The walk stops at the first attempt it cannot find. `plan_rounds`
  (`dispatch/src/serve.rs:503`) is unchanged: it lists the newest
  workflow attempt's rounds, and the prior copies' round files sit
  beside the paths `plan_prior` names.
- **`show`** (`dispatch/src/main.rs:911`) prints, after the plan lines
  at `dispatch/src/main.rs:982` and before the round lines, one line per
  prior copy:

  ```
    plan (prior revision, review/1, round 2 by you): /…/review/1/joined/plan.md
  ```

  With no revision on it (a chained copy): `plan (prior copy,
  review/2)`. `--json` carries `plan_prior` as is.
- **A `finalize` question** (`dispatch/src/scheduler.rs:7693-7696`) is
  unchanged: it already names the reviewed copy.

## The UI

The ticket page reads `PathsView` for its plan label
(`src/ui/ticket.rs:425-433`) and ignores the new field; the port's
single-ticket reply is additive, so an older app reads it unchanged.
No app change.

## The tests

Integration tests in `dispatch/tests/first_slice.rs`, next to
`a_failed_review_run_fails_the_attempt_and_a_rerun_starts_a_new_run`
(`dispatch/tests/first_slice.rs:3257`), using `at_finalize`
(`dispatch/tests/first_slice.rs:3365`), `revise`
(`dispatch/tests/first_slice.rs:3417`), `artifact_of`
(`dispatch/tests/first_slice.rs:395`) and `answer_rerun`
(`dispatch/tests/first_slice.rs:13254`):

- `a_rerun_after_a_revise_reviews_the_revised_copy`: at finalize,
  answer `revise`, step, overwrite attempt 1's copy with revised text as
  the planner would, clear the fake's runs (as
  `a_run_switchboard_no_longer_has_fails_the_attempt` does at
  `dispatch/tests/first_slice.rs:3812`), step. Assert the attempt failed
  with `review run gone: no such run`, the `rerun` question names
  attempt 1's copy path and `round 2, revised by you`, then answer
  `rerun` and step until attempt 2 has a run. Assert attempt 2's copy
  holds the revised text, `revised_from == Some(("review", 1))`, the
  fake's new run's `plan` is attempt 2's copy, and the `attempt-started`
  event text ends `from review/1's revised copy`.
- `a_second_failure_reruns_from_the_chained_copy`: continue from the
  above, overwrite attempt 2's copy, clear the runs again, answer
  `rerun`; attempt 3's copy holds attempt 2's text and `revised_from ==
  Some(("review", 2))`.
- `a_rerun_with_no_revision_reviews_the_plan_stages_file`: extend the
  existing `a_failed_review_run_fails_the_attempt_and_a_rerun_starts_a_new_run`
  with two assertions: attempt 2's `revised_from` is `None` and the
  `rerun` question does not mention a revised copy.

Unit tests:

- `dispatch/src/scheduler.rs` tests module
  (`dispatch/src/scheduler.rs:11338`):
  `review_subject_prefers_a_revised_copy_newer_than_the_writer`, built
  from `crate::ticket::blank()` and `new_attempt` in a `tempfile`
  directory: a complete plan attempt, a failed workflow attempt with a
  revision and a copy on disk, and the cases where the copy is missing
  (falls back, writer's file), where the writer is newer (restart), and
  where the failed attempt has no revision and no `revised_from`
  (writer's file).
- `dispatch/src/serve.rs` beside
  `the_paths_name_an_open_review_copy_with_its_round`
  (`dispatch/src/serve.rs:1429`):
  `the_paths_list_the_revised_copies_the_plan_descends_from`: a plan
  attempt, a failed review attempt 1 with a revision, a running attempt
  2 with `revised_from` set; `plan` is attempt 2's copy and `plan_prior`
  is one `PriorCopy` for attempt 1 with round 2 by `you`; then attempt
  2 failed and attempt 3 running from it gives two entries, attempt 2's
  with no round.
- `dispatch/src/store.rs` beside
  `a_version_twenty_seven_record_migrates_with_no_transcripts_or_run_sessions`
  (`dispatch/src/store.rs:1368`): a record at the previous version reads
  at `RECORD_VERSION` with `revised_from` absent and round-trips once
  set.

Docs in the same commit: `docs/dispatch.md:550-557` ("A review is a
copy") gains the sentence that a rerun copies the newest revised copy
of the same stage and context, chained, and that the question and
`show` name it; the decisions table row at `docs/dispatch.md:2975` and
the first-slice row at `docs/dispatch.md:3078` name the new tests. A
new `docs/design.md` section before "Open questions"
(`docs/design.md:3672`), "Review reruns keep the revision
(2026-10-21)", in the shape of "Dispatch definitions pruned"
(`docs/design.md:3076`), with its known gap: the planner's unfinished
round edits in a failed attempt are carried along with the revision,
since the copy is one file. README's Layout table has no new file; its
`src/serve.rs` line (`README.md:456`) is unchanged. `CHANGELOG.md`
Unreleased gains one line under Fixed.

Commands before the commit:

```sh
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo clippy --locked -p dispatch --all-targets --target x86_64-unknown-linux-gnu -- -D warnings
cargo fmt --all
```

## Commit

`Dispatch reruns a failed plan review from the owner's revised copy, named in the rerun question and in show`

Contract additions:

- `dispatch::ticket::Attempt::revised_from: Option<(String, u32)>` (serde default; `RECORD_VERSION` from its value at implementation time plus one, with a migrate step comment)
- `dispatch_control::PathsView::plan_prior: Vec<PriorCopy>` (serde default)
- `dispatch_control::PriorCopy { stage, n, path, round: Option<u32>, by: Option<String> }`
