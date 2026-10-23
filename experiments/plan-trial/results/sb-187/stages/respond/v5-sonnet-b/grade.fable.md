| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| v5-sonnet-b | 3 | 3 | 3 | 2 | note |

## Grounding
- `src/core/control.rs:441-455`: found (the `ControlAction::StartWorkflow` arm passes `source` and `plan` straight to `start_workflow`, as claimed)
- `ticket.rs:346-350` (`Attempt.revisions`): found (field with the doc "A plan review's rounds the owner opened with a `revise` answer")
- `Revision` at `ticket.rs:510`: found (struct with `round`, `by`, `at_ms`)
- `ticket.rs:1856` ("a parked review's copy is not the plan going forward"): found (the comment at that exact line in the `shown` test, followed by the Cancelled/Failed loop)
- `review_subject` at `scheduler.rs:10534`, only caller `start_workflow` at `7465`: found (filter on `AttemptState::Complete` at 10539; the call at 7465 followed by `std::fs::copy` at 7471)

Also spot-checked: `rerun_carries` 9382, `rerun_ask` 9373, question at 7263-7266, `print_lane_files` main.rs:896, `LaneFile` lib.rs:586, `RECORD_VERSION = 28` store.rs:26, `migrate` store.rs:452, `rerun_note` 8988, `may_rerun` 10665, `new_attempt` 10277, `fail_attempt_with` 7195, `poll_workflow` 7741, `retire_replaced` 3578, `revise` first_slice.rs:3417, `at_finalize` 3365, failure test 3809, dispatch.md decision 16 at line 3212. All correct.

## Departures caught
- No rerun-specific path; the fix is the shared subject lookup: caught. The plan says only `review_subject` changes and that `may_rerun` relaunches through `start_workflow` with no new action.
- Plan review carries nothing between attempts today: caught. The plan states `rerun_carries` returns "" for Workflow and that the planner clone is always of the plan stage's session, so earlier rounds are not carried; the question text says so.
- The revision is not lost and the record already knows (`revisions`, files on disk): caught. The plan's first departure bullet lists the three surviving places and keys the picker on `revisions`, not file contents.
- Found beyond the reference: the `CloneAllTranscript` limitation (the rerun's planner never saw the `revise` exchange) and the fact that `--note` on a Workflow rerun has no consumer. Both are real and correctly scoped as out of plan.

## Decisions to overturn
- None I would reverse. Two places I'd flag as differing from the reference without reversing: (a) when the revised copy is missing on disk the plan parks rather than falling back to the plan stage's output (reference decision 1 says fall back); parking loudly is defensible given the issue is about silent discard, so I side with the plan here. (b) `show` annotates attempt 2's line with "seeded from the revised copy of review/1" instead of listing review/1's path as its own `files:` line (reference decision 4); it answers the issue's "say which" but a reader cannot copy the prior path. Adding `seeded_from` rather than reusing `carried_from` is the reference's "acceptable" option, and the `RECORD_VERSION` bump follows the repo's own pattern for new `Attempt` fields.

## Over-specification
- "The signals" and most of "The actions and effects" say nothing changes; "Contract additions" and the commit message restate the record section; the migrate-step prose re-explains the store's version policy. The "no egui, no SWITCHBOARD_SCRIPT" line is noise. Roughly 15-20% could go.

## Hand-off
- Note: list review/1's `plan.md` path as its own prior-revision line in `show` (not just an annotation), and consider reusing `carried_from` to skip the new field and the version bump.

## Notes
- best: pinpoints the single `Complete` filter as the bug and makes the rerun question say the planner's earlier rounds are not carried, which the reference's "no app-side change" glosses over.
- worst: three additive surfaces (`Attempt.seeded_from`, `LaneFile.seeded_from`, `RECORD_VERSION` bump plus migrate test) for provenance that `carried_from` already models.