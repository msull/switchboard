| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| I | 3 | 2 | 0 | 1 | note |

## Grounding
- `dispatch/src/ticket.rs:1856`: found (test confirms parked and failed review copies stop being the shown plan).
- `scheduler.rs:7616-7618`: found (constructs the owner feedback and writes its file).
- `scheduler.rs:7619-7626`: found (records the revision’s round, author and timestamp).
- `scheduler.rs:7608`: found (requires the workflow to be Converged or AtCap before revision).
- `dispatch/src/scheduler.rs:7379`: found (`workflow_stage` routes eligible failed attempts through `start_workflow`, also used for initial attempts).

## Departures caught
- No separate rerun path: caught with a wrong fix. Identifies the shared launcher and Complete-only lookup, but overlays failed-copy selection instead of fixing the shared subject lookup as the reference directs.
- Only code reviews carry points between attempts: missed explicitly; the plan makes no false claim that workflow points carry.
- Revision survives on disk and in the record: caught; correctly reuses existing revisions and preserves the revised content without replaying the note.
- Additional finding: a later completed plan can supersede an earlier review copy; the cited ticket test supports this distinction.

## Decisions to overturn
- Move copy selection into the shared subject lookup rather than overriding its result in `start_workflow`, per the reference’s required fix site.
- Replace the multi-line revision-history display and ancestry status with the reference’s compact prior-revision file line. “Not under review” is also misleading before the promised rerun starts.
- Include the owner’s revision round in the rerun question, as reference decision 3 requires.

## Over-specification
- Roughly one-third is expendable: exact helper placement, construction-site inventory, full UI struct, repeated contract declarations, empty Signals section, unchanged README discussion, and detailed migration choreography. The revision-history presentation exceeds the reference’s requested scope.

## Hand-off
- Consolidate subject selection, include the owner’s round in the question, and reduce `show` to the requested prior-copy line; the revision-only policy and single new provenance field are reference-acceptable.

## Notes
- best: Tests cover revised bytes surviving a second failure while retaining the original planner session.
- worst: Expands a focused copy-selection fix into revision-history machinery while omitting the required round-specific rerun wording.