| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| v5lean-sonnet-a | 3 | 2 | 0 | 2 | note |

## Grounding
- `facts.md`: found (the supplied facts text covers surviving artifacts, source sessions, and the Complete-only lookup).
- `dispatch/src/scheduler.rs:7616-7618`: found (constructs the owner feedback body and writes it beside the reviewed copy).
- `7621`: found (pushes a `Revision` onto the review attempt).
- `scheduler.rs:9382-9390`: found (`rerun_carries` supplies code-review carry text and otherwise returns an empty string).
- `src/core/workflow.rs:1009-1028`: found (`remove_workflow` refuses waiting runs and removes the selected stored run; this excerpt alone cannot establish every possible cause of disappearance).

## Departures caught
- No separate rerun launch path: caught operationally—the proposed lookup runs inside shared `start_workflow`, although it leaves `review_subject` unchanged.
- Plan reviews carry neither rounds nor points between attempts: missed explicitly; the plan makes no false carry claim, but does not explain this distinction.
- Revision files and revision records already survive: caught; uses existing artifacts and `revisions`.
- Additional departure: questions whether a plain runner restart explains `no such run`. Useful qualification, though the cited removal function alone does not prove the broader claim.

## Decisions to overturn
- Leaving `show` unchanged conflicts with reference decision 4: list the prior revision beside the reviewed copy, and test that visibility.
- “No change” to the record omits reference decision 2: populate existing `carried_from` and widen its documentation.
- Generic “with its revisions” conflicts with reference decision 3: identify the owner's round-N revision when present; avoid implying revisions exist on unrevised copies.
- Updating `docs/design.md` alone conflicts with reference decision 7: update the rerun and revise documentation in `docs/dispatch.md`.

## Over-specification
- Roughly one fifth is dispensable: exact helper signature and visibility, commit message, repeated session/default explanations, and overlapping helper/integration coverage for cancellation.

## Hand-off
- Restore provenance, prior-copy visibility in `show`, round-specific rerun wording, and the requested operational documentation before implementation.

## Notes
- best: Separates the revised input file from the planner source session and tests the actual copied bytes.
- worst: Explicitly drops the requested `show` behavior despite recognizing the visibility problem.