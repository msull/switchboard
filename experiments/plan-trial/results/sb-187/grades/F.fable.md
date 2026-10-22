All anchors in the plan check out against the tree, including the secondary ones (RECORD_VERSION is 28, `carried_from` at ticket.rs:317, the `revise` docs row at line 2975). Scoring now.

```
| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| F | 3 | 3 | 2 | 2 | note |
```

## Grounding
- `shown_where` (`dispatch/src/ticket.rs:1476`): found (fn `shown_where` at line 1475, doc says it keeps complete attempts and an open workflow attempt, as the plan states)
- `plan_rounds` (`dispatch/src/serve.rs:504`): found (fn `plan_rounds(a: &Attempt)` at line 504; line 572 reads it from one `review` attempt, matching the plan's "newest workflow attempt only" claim)
- `review_subject` (`dispatch/src/scheduler.rs:10534`): found (fn at 10534, with the `.filter(|a| a.state == AttemptState::Complete)` the plan names as the cause)
- `start_workflow` (`dispatch/src/scheduler.rs:7436`): found (fn signature at 7436, takes `stage` and `ctx` as the plan says)
- `revise` (`dispatch/src/scheduler.rs:7565`): found (fn `revise` at 7565)

## Departures caught
- Rerun path does not exist; fix is in the shared subject lookup: caught. The plan names `review_subject`'s Complete filter as the one cause and replaces that function rather than adding a rerun branch.
- Plan reviews carry nothing between attempts today: caught. The plan says `rerun_ask` names no copy for a plan review and places its new clause beside `rerun_carries` without claiming points carry.
- Revision is already on disk and on the record: caught. The plan says the note and revised copy survive, uses `revisions` on the failed attempt as the candidate test, and says only the copy selection and `show` lose it.
- Extra, not in the reference: the planner clone still comes from the plan stage's session because the failed attempt's session is a clone, so the conversation does not hold the revise round; also that `plan_rounds` reads one attempt so attempt 1's owner round drops from `show`.

## Decisions to overturn
- `show` gains `LaneFile.from` plus a `PathsView.plan_earlier: Vec<EarlierPlan>` with nested rounds and two printed lines per earlier attempt. The reference's call is one extra `files:` line naming the prior revision, not a new wire type with its own rounds. I would cut the nested rounds and the second printed line.
- Everything else matches the reference: new field instead of reusing `carried_from` is allowed; "only if revised" is allowed; the note is not routed through `rework`; no app change; scheduler tests, docs row near line 2975, CHANGELOG line.

## Over-specification
- The commit message and "Contract additions" sections restate the record and wire changes already given above them.
- The README Layout conditional ("if it lists fields"), the `docs/design.md` conditional line, and the `attempt-started` event text change are things the implementer either would not need or the reviewer would not ask for.
- The `EarlierPlan` rounds plumbing follows from the overturned decision.
- Roughly a fifth of the plan.

## Hand-off
- Collapse the `show` change to one extra `files:` line for the prior attempt's copy; drop `EarlierPlan.rounds` and the owner-round sub-line.

## Notes
- best: Pins the bug to `review_subject`'s Complete filter and gives a candidate rule (revised or chained `subject_from`) that survives a second failure, with tests for each branch.
- worst: Turns the one-line `show` addition into a new wire struct with nested rounds and a version bump's worth of surface.