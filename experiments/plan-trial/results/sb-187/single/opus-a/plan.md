# A plan review rerun keeps the owner's revision (issue #187)

A plan review attempt that fails after the owner answered `revise` has
the only copy of the plan the reviser wrote for the owner's note. Today
a `rerun` copies the plan stage's original output into the new attempt,
and nothing says the revision was dropped. With this change the rerun
copies the failed attempt's reviewed copy instead. The record notes
which attempt it came from, the rerun question names that copy, and
`show` lists every revised copy with whether the copy under review now
started from it.

## Where this departs from the issue

- The issue asks for "the newest plan the stage holds". The plan
  carries a failed attempt's copy only when the owner revised it: the
  attempt's `revisions` is non-empty, or the attempt itself started from
  such a copy. A failure with no `revise` still starts from the plan
  stage's output. The planner's edits for reviewer rounds come back when
  the new attempt's rounds run again, but the owner's note does not.
  Cancelled attempts are not carried either. A park the owner asked for
  (at `finalize` or anywhere else), or a restart under a new pipeline
  copy, is a deliberate stop, and `shown_where` already treats a parked
  copy as not the plan going forward (`dispatch/src/ticket.rs:1856`).
- The issue asks for the `revise` note to survive. It already does: the
  note is on disk as `<stem>.feedback-<n>.md` beside attempt 1's copy
  (`scheduler.rs:7616-7618`), and the round is on attempt 1's
  `revisions` (`scheduler.rs:7619-7626`). What gets lost is the revised
  copy as the next attempt's input, so that is what the plan carries.
  The note is not sent again to the new run. `workflow.object` is only
  accepted by a finished run (`scheduler.rs:7608`), and the revised copy
  already contains the planner's answer to it. `show` names the note's
  file next to the prior revision.
- The issue offers "or at least say so" as an alternative. The plan
  does both: it carries the copy and says so.

## The cause, checked

- `workflow_stage` (`dispatch/src/scheduler.rs:7379`) starts a rerun
  through `start_workflow` (`:7403`) once `may_rerun` allows it.
- `start_workflow` (`:7436`) gets its input from `review_subject`
  (`:7465`) and copies it into the new attempt directory (`:7469-7472`).
- `review_subject` (`:10534`) takes the newest **Complete** attempt
  that wrote the subject. The failed review attempt is skipped, so the
  input is `plan/1/joined/plan.md`.
- `Ticket::shown_where` (`dispatch/src/ticket.rs:1476`) also skips a
  failed workflow attempt. That is why `show`'s `files:` block
  (`dispatch/src/main.rs:981-994`, from `serve::ticket_paths` at
  `dispatch/src/serve.rs:538`) names only the attempt 2 copy.

## The record

`Attempt` (`dispatch/src/ticket.rs:266`) gains
`revised_from: Option<u32>` with `#[serde(default)]`, placed beside
`carried_from` (`:317`). It holds the number of the same stage's
attempt, in the same context, whose revised copy this plan review
attempt copied at start. It is `None` for every other attempt and in
older records. `new_attempt` (`dispatch/src/scheduler.rs:10277`) sets it
to `None`. Every other construction site uses `..new_attempt(..)` or
`..a.clone()` (`report.rs:604`, `usage.rs:898`, `usage.rs:1211`,
`scheduler.rs:12878`), so none of them change.

`RECORD_VERSION` in `dispatch/src/store.rs:26` (28 today) goes up by
one: take the value in `store.rs` at implementation time and add one,
because a ticket landing in parallel may take the next number first.
`migrate` gets a comment-only step after the `27 to 28` one
(`store.rs:608`). In the same style it says: an attempt gains
`revised_from`, absent from its serde default, and nothing is
transformed. A build that would drop the field must refuse the record,
or `show` would report a carried revision as discarded.

## The rule

A new method, `Ticket::revised_copy(&self, a: &Attempt) -> Option<&PathBuf>`
in `ticket.rs`, placed beside `shown_where`. It returns `a`'s reviewed
copy when all of these hold:

- `a.kind == AttemptKind::Workflow` and `a.state` is `Failed`;
- `!a.revisions.is_empty() || a.revised_from.is_some()`;
- the attempt's one artifact (a workflow attempt holds exactly its
  subject, `scheduler.rs:7493`) is not in `a.forgotten` and is a file
  on disk;
- no Complete attempt later in `self.attempts` wrote an artifact under
  the same name. A plan stage that ran again since then wrote a newer
  plan, and that plan wins, as it does in `shown_where`.

The scheduler and `serve` both call this one method, so the rerun
question, the launch and `show` all apply the same rule.

## Actions and effects

- `start_workflow` (`scheduler.rs:7436`): before it pushes the new
  attempt, look up `latest_attempt(t, &stage.name, ctx)` and call
  `t.revised_copy(..)` on it. If that returns a copy, use it as the
  source of the `std::fs::copy` at `:7471` in place of `review_subject`'s
  path, set `revised_from = Some(that attempt's n)` on the attempt pushed
  at `:7487`, and log
  `ticket {} {stage}/{ctx}: attempt {n} starts from attempt {m}'s revised copy {path}`.
  `review_subject`'s session is still the `source` of `workflow.start`
  (`:7511`). The clone needs a session with a transcript, and the failed
  run's planner may be gone with it. Switchboard already tells the
  planner to edit only the copy it is given (`docs/dispatch.md:550-557`),
  and that copy is now the revised one.
- `rerun_carries` (`scheduler.rs:9382`) takes the attempt. For a
  workflow attempt where `t.revised_copy(a)` is `Some(path)`, it appends
  `" A rerun reviews the revised copy {path}, which the owner's revise
  left, not the plan stage's output."`. `rerun_ask` (`:9335`) already
  places the result in the question.
- No new `Effect`, wire request or event. The decision event carries the
  rerun question text, so the new sentence shows up in `events`, `wait`
  and the app with no further change.

## Signals

None new. The rerun question above and `show` below say which copy is
reviewed.

## The UI (`show`)

`dispatch-control/src/lib.rs` gains
`PathsView::prior_revisions: Vec<PriorRevision>` (`PathsView` is at
`:546`, which is `#[serde(default)]` like its siblings) and:

```rust
/// A failed plan review attempt's copy that the owner's `revise` changed.
pub struct PriorRevision {
    /// The attempt's context: `root`, `joined` or a lane.
    pub context: String,
    /// The attempt's number.
    pub attempt: u32,
    /// Its reviewed copy.
    pub path: PathBuf,
    /// The rounds the owner opened, with their feedback files.
    pub owner_rounds: Vec<PlanRoundView>,
    /// The copy under review now started from it, directly or through
    /// an attempt that itself carried it.
    pub carried: bool,
}
```

`serve::ticket_paths` (`serve.rs:538`) fills the list. It walks the
workflow attempts that are `Failed` with non-empty `revisions` and an
unforgotten artifact, oldest first. `owner_rounds` is `plan_rounds(a)`
(`serve.rs:504`) filtered to rounds with `by` set. `carried` is true when
some later attempt of the same stage and context reaches `a.n` by
following `revised_from` back.

`show` (`main.rs:981-994`) prints after the plan lines:

```
  prior revision (joined): <path>, attempt 1 failed
    owner round 2: <feedback file>
    (the reviewed copy started from it)
```

When `carried` is false, the last line is instead
`(not under review: a later attempt started from an earlier plan)`.
That second form is what a ticket recorded before this change, like the
one in the issue, shows. `--json` carries the field as-is. The app's
ticket page does not read `plan_files` or the new field, so the app is
unchanged.

## Tests

- `dispatch/tests/first_slice.rs`, next to
  `revise_writes_the_owners_round_and_sends_it_as_an_objection`
  (`:3442`) and `a_run_switchboard_no_longer_has_fails_the_attempt`
  (`:3809`):
  - `a_rerun_after_a_revised_review_fails_reviews_the_revised_copy`:
    1. `at_finalize`, then `revise`, then a step.
    2. Write new text into attempt 1's copy, as the reviser would.
    3. `env.sb().runs.clear()`, then a step. The `rerun` question names
       attempt 1's copy.
    4. Answer `rerun`, then a step. Attempt 2 has `revised_from == Some(1)`,
       its copy's bytes equal the revised text, the new run's `plan` is
       attempt 2's copy, and its `source` is the plan attempt's session.
    5. `dispatch show --json` lists one prior revision with
       `carried: true` and the owner's round 2 file. Plain `show` prints
       `(the reviewed copy started from it)`.
  - `a_rerun_after_a_failed_review_with_no_revise_reviews_the_plan_output`:
    `at_review_run`, clear the runs, then `rerun`. The copy equals the
    plan stage's output, `revised_from` is `None`, the question has no
    revised-copy sentence, and `prior_revisions` is empty.
  - `a_revision_survives_a_second_failed_rerun`: attempt 2, carried from
    1, fails with no `revise` of its own. Attempt 3 has
    `revised_from == Some(2)` and the same bytes, and `show` lists
    attempt 1 as carried.
- `dispatch/src/ticket.rs` unit test, next to
  `the_plan_shown_is_an_open_review_copy_but_an_input_never_is`
  (`:1827`):
  `a_revised_copy_is_the_rerun_input_until_a_plan_stage_writes_again`.
  `revised_copy` returns `None` with no revisions, for a cancelled
  attempt, for a forgotten artifact, for a missing file, and after a
  later complete plan attempt. Otherwise it returns `Some`.
- `dispatch/src/serve.rs` unit test, next to
  `the_paths_name_an_open_review_copy_with_its_round` (`:1429`):
  `a_prior_revision_says_whether_the_review_carried_it`. One case is
  carried through a `revised_from` chain. The other is not carried: an
  older record whose attempt 2 has `revised_from: None`.
- `dispatch/src/store.rs`: a record at the current version (before the
  bump) with no `revised_from` key reads with `None` and is written back
  at the new version.

## Docs

- `docs/dispatch.md`:
  - The "A review is a copy" bullet (`:550`) gains one sentence: a
    rerun after an attempt that failed with an owner's revision copies
    that attempt's copy, recorded as `revised_from`.
  - The record versions paragraph (`:393-401`) gains a line for the new
    version.
  - Design choice 6 (`:3171`) gains one sentence: a revision survives a
    failed run.
  - The failure table gains a row next to the `workflow` query row
    (`:3078`): "A review run gone after `revise`", answered `rerun` →
    the new attempt reviews the revised copy and `show` lists the prior
    revision, citing the three `first_slice` tests above.
- `README.md`: no new file, environment variable or data-directory
  entry, so the Layout table and dev-aid list stay as they are.
- `CHANGELOG.md` under Unreleased:
  "A Dispatch plan review that fails after you answered `revise` reruns
  on the revised plan rather than the plan stage's original, and `show`
  lists each revised copy and whether the review carried it."

## Commit

Dispatch: a plan review rerun after a revised attempt fails reviews the revised copy

Contract additions:
- `dispatch::ticket::Attempt::revised_from: Option<u32>` (serde
  default; `RECORD_VERSION` is the value in `store.rs` at
  implementation time plus one, with a comment-only `migrate` step)
- `dispatch::ticket::Ticket::revised_copy(&self, a: &Attempt) -> Option<&PathBuf>`
- `dispatch_control::PathsView::prior_revisions: Vec<PriorRevision>`
- `dispatch_control::PriorRevision { context, attempt, path, owner_rounds, carried }`
