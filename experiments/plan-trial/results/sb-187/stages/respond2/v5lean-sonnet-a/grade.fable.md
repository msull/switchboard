All checks done. The five cited references are real, the fix site matches the reference, and the plan's one visible gap is dropping the `show` half of the issue.

```
| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| v5lean-sonnet-a | 3 | 3 | 1 | 3 | note |
```

## Grounding
- `dispatch/src/scheduler.rs:7616-7618` (revise writes `plan.feedback-<n>.md`): found (round_file, OWNER_ROUND_HEADING body, atomic_write at exactly those lines).
- `scheduler.rs:7621` (pushes a `Revision`): found (`review.revisions.push(crate::ticket::Revision {` at 7621).
- `scheduler.rs:9382-9390` (`rerun_carries` returns `""` for a workflow stage): found (RESOLUTION arm, `AttemptKind::Review` arm, else `""`).
- `src/core/workflow.rs:1009-1028` (`remove_workflow` is the only drop): found (fn at 1009, `workflows.retain` at 1023, refuses while `waiting()`).
- `dispatch/tests/support/mod.rs:690` (`no such run` from the fake): found (`None => Reply::failed("no such run")` at 690, so `runs.clear()` models the failure).
- Also verified: `start_workflow` 7465-7472 copy path, `review_subject` 10534-10548 Complete filter, `poll_workflow` 7784-7788 overwriting `Attempt.session` with the run's face, `ticket.rs:1856-1864` Cancelled/Failed copy not shown, first_slice 3257 and 3442 test anchors. All correct.

## Departures caught
- No rerun-specific path; fix belongs in the shared subject lookup: caught in effect. The helper sits beside `review_subject` and is called from `start_workflow` for every attempt, so it is built at the real site, though the plan never says outright that "the rerun path" does not exist.
- Plan review carries nothing between attempts today: caught (`rerun_carries` returns `""` for workflow stages; the plan adds the Workflow arm).
- Revision is not lost and the record already knows: caught explicitly ("No change, no bump. `Attempt.artifacts["plan"]` of the failed `review/1` already holds the revised copy's path, and `Attempt.revisions` the owner's round").
- Extra, not in the reference: the `revise` note already survives a failure as a file plus a `Revision` entry, so the issue's "should survive like an answered decision" needs no change. Correct.
- Extra: the repro's "runner restart gives `review run gone`" is not borne out by the tree (`remove_workflow` is the only drop). Correct and useful for the test design.
- Extra and valuable: `poll_workflow` overwrites a workflow attempt's `session` with the planner/reviewer face, so the rerun must keep `plan/1`'s session as `WorkflowStart.source` while taking the copy from the failed attempt. The reference does not call this out and a naive "pick the failed attempt as subject" fix would pass the wrong session.

## Decisions to overturn
- `show`'s `files:` block left alone. The issue's "Where to look" names it and the "at least" clause asks `show` to say which copy is reviewed; reference decision 4 adds one line listing the prior revision beside the reviewed copy. The plan declines this explicitly.
- No provenance on the record. Reference decision 2 reuses `carried_from` on the new attempt; the plan records nothing, so only the rerun question text and the files on disk say where attempt 2's copy came from.
- Not counted but worth redirecting: docs go to `docs/design.md` near line 2213 (which is the app's artifact-read text), whereas the rerun and `revise` documentation lives in `docs/dispatch.md` (rerun at 318-350).
- Agree with the plan against a strict reading of the reference: skipping Cancelled attempts is right, since `ticket.rs:1856-1864` already pins a parked copy as not the plan going forward.

## Over-specification
- Almost nothing to cut. The repro-step-2 bullet and the commit message are the only lines an implementer would not need, under a tenth of the plan. The unit tests for `rerun_copy` ordering (failed review then complete plan/2; Cancelled gives None) are justified by the walk's ordering subtlety.

## Hand-off
- note: also add the `show` `files:` line for the prior revision, set `carried_from` on the new attempt, and put the docs in `docs/dispatch.md` rather than `design.md`.

## Notes
- best: Catches that `poll_workflow` replaces a workflow attempt's session with the run's face, so the fix takes the copy from the failed attempt but keeps the plan stage's session as `source`, which is the trap a naive fix falls into.
- worst: Explicitly drops the `show` change the issue asks for, and records no provenance, so the ticket itself never says which copy attempt 2 reviewed.