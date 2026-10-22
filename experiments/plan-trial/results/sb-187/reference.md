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
