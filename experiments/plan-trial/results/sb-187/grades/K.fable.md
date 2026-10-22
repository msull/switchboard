| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| K | 3 | 3 | 2 | 2 | note |

## Grounding
- `dispatch/src/scheduler.rs:7565` revise: found (`fn revise` begins at 7565; doc above it describes the revision record per round)
- `scheduler.rs:10534` review_subject: found (`fn review_subject` at 10534 with the `AttemptState::Complete` filter the plan names as the bug)
- `scheduler.rs:7465`–`7468` start_workflow copies from the plan attempt: found (the `review_subject` call and `copy = dir.join(..)` sit at 7462-7468; `WorkflowStart { plan: copy }` at ~7510 as the plan says)
- `dispatch/src/ticket.rs:1476` Ticket::shown_where: found (signature at 1477, doc comment 1471-1475; filter is `Complete || (Workflow && is_open())` exactly as described)
- `dispatch/src/serve.rs:538` ticket_paths: found (`fn ticket_paths` at 538, `plan_files` literal 539-547 without `..Default::default()`, so the plan's note that both new fields must be written explicitly is right)

Also spot-checked and real: `main.rs:896` print_lane_files, `main.rs:981` files block, `ticket.rs:350` revisions, `rerun_ask` 9335, `rerun_carries` 9382, `finish_parking` 2654, `store.rs:26` RECORD_VERSION 28, `LaneFile` at `dispatch-control/src/lib.rs:586`, `plan_label` `src/ui/ticket.rs:427`, the `ticket.rs:1858` test loop, `first_slice.rs:3809`, `report::round_file`, `Stage::runs_per_lane`, `lane_files_by`, design.md and agent-guide line ranges.

## Departures caught
- No rerun-specific path; the fix is the shared subject lookup: caught. The plan puts the fix in `review_subject` via a new `review_copy`, exactly at the reference's site.
- The plan review carries nothing between attempts: caught implicitly. The plan leaves `rerun_carries` alone, notes it only fires for `AttemptKind::Review`, and adds a separate Workflow clause rather than claiming points carry.
- The revision is already on disk and on the record: caught. The plan keys on `revisions` and the attempt directory, adds no record state.
- Extra, not in the reference: `revise` writes only the note and the planner edits in place, so the copy holds the revision only if the planner finished before the run died; the plan says it cannot detect a mid-edit death and lets the rerun question name the note so the owner can park. Also: parking cancels only open attempts, so a failed revised copy on a parked ticket needs an explicit rule; and `docs/design.md:2163` states the "complete files only" rule the change amends. All three are real.

## Decisions to overturn
- Provenance on the record (reference decision 2): the plan stores nothing on the new attempt about which copy it started from and derives it at read time through `review_copy` and a `prior_lane_files` walk. The reference wants `carried_from` reused or one new field. I would record it, since once attempt 2 completes the derivation only works because the old attempt still has `revisions`. One reversal.
- Not overturned but noted: the plan restricts the carry to revised copies (reference allows this), puts the question clause in a new `revised_copy_carries` instead of a Workflow arm in `rerun_carries` (equivalent), and adds two `LaneFile` bools where the reference wanted one extra `files:` line (modest, serde-default, no record version change). The parked-ticket exclusion makes `shown` depend on ticket state, which is unusual but defensible.

## Over-specification
- The Record section's `RECORD_VERSION` paragraph, the empty Signals section, the commit-message line, the README no-change note, the four gate commands the pre-commit hook forces, and the exact `lane_file_note` string table plus its own unit test. Roughly 15-20% of the plan.

## Hand-off
- Record the copy's source on the new attempt (reuse `carried_from`) instead of deriving it from the old attempt's `revisions`, and ask the owner whether an unrevised failed copy should carry too, per the reference's broader rule.

## Notes
- best: names the one-line cause (`review_subject`'s `Complete` filter) and finds the mid-edit death and parked-ticket cases the reference does not mention.
- worst: no provenance on the record, and a ticket-state-dependent `carries_revision` predicate means the shown plan flips on park and resume.