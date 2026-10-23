All of the plan's anchors check out, including the secondary ones (`rerun_carries` at 9382 returning `&'static str`, both callers at 7266 and 9373, `start_workflow`'s call at 7465, the planner-face assignment at 7786-7788, `lane_files_by` at 10917, the first_slice test at 3809, and the docs lines). One thing the code shows that the plan's `show` change does not handle: `shown_where` returns only the newest qualifying attempt via `find_map`, so once attempt 2 is open, attempt 1's copy still never appears.

```
| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| v5lean-sonnet-c | 3 | 3 | 2 | 3 | note |
```

## Grounding
- `scheduler.rs:7195-7230` (`fail_attempt_with` leaves the record intact): found (sets `state`, `ended_ms`, the two `failed_at_*` flags; touches neither `artifacts` nor `revisions`)
- `ticket.rs:1476-1491` (`shown_where` skips the failed review): found (filter is `Complete` or open `Workflow`; a `Failed` attempt is dropped)
- `scheduler.rs:10534-10548` (`review_subject`): found (one `filter(|a| a.state == AttemptState::Complete)` then `find_map` over artifacts; returns `(PathBuf, String)`)
- `ticket.rs:228-232` (`Cancelled` may be one Dispatch discarded): found (doc comment: "Stopped by Dispatch (the ticket parked, a rerun replaced it, or a later gate sent the work back)")
- `ticket.rs:1446-1450` (a review edits its copy in place until `finalize`): found (the `shown_where` doc comment says exactly that; line numbers are a few off but the text is there)

## Departures caught
- No rerun-specific path; the fix is the shared subject lookup: caught (the plan goes straight to `review_subject` and names `start_workflow` as its one caller; it never proposes a rerun-only branch)
- Plan review carries nothing between attempts: caught (it reads `rerun_carries` correctly as giving a `Workflow` attempt nothing and does not claim points carry)
- The revision is not lost on disk or on the record: caught ("It already does; what is lost is the pointer to the revised copy"), and it builds on `artifacts` and `revisions` rather than new state
- Found beyond the reference: before the rerun is answered, `show` is worse than the issue says, naming `plan/1`'s copy because `shown_where` skips the failed attempt. Correct per the code.
- Also found: the failed review's `session` is the planner clone's face (7786-7788), so the new attempt's `source` must stay the plan attempt's session. The reference assumes this but does not flag the trap.

## Decisions to overturn
- `show` after the rerun. The plan widens `shown_where` to accept a `Failed` `Workflow` attempt, but `shown_where` returns one attempt, the newest. Once attempt 2 is open it wins, so `show` still names only attempt 2's copy, which is the issue's exact complaint and the reference's decision 4 (one extra `files:` line naming the prior revision). The plan's change fixes only the pre-rerun window and its CHANGELOG line overclaims.
- Softer disagreement, not counted: the plan records no provenance on the new attempt. The reference reuses `carried_from`; the plan derives the copy at launch and names it only in the question text. Acceptable since the reference's `show` rule compares content, but I would add `carried_from` for the record.
- Also minor: the rerun question names the path but not that it holds the owner's round-N revision, which the reference's decision 3 includes.

## Over-specification
- The first departure bullet about why the run vanished is noise the implementer does not need. Everything else earns its place. Under 5% to cut.

## Hand-off
- `shown_where` picks the newest attempt only, so after `rerun` is answered add a `files:` line for the replaced attempt's revised copy beside attempt 2's; mention the round-N revision in the question and record the source in `carried_from`.

## Notes
- best: Splits the copy from the session in `review_subject` and spots that the failed attempt's `session` is the planner's face, so cloning from it would be wrong.
- worst: The `show` change only helps before the rerun; after it, the prior revision is still invisible, which is what the issue reported.