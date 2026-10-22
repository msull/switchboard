| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| H | 3 | 3 | 1 | 1 | note |

## Grounding
- `scheduler.rs:7379`: found (`workflow_stage` handles first starts and reruns).
- `start_workflow` `(7436)`: found (shared launch function copies the selected subject).
- `review_subject` `(10534)`: found (selects the newest Complete attempt holding the artifact).
- `revise` `(7565)`: found (writes owner feedback, records the revision, sends `WorkflowObject`).
- `ticket.rs:350`: found (`revisions: Vec<Revision>` on the attempt; relevant, though not the type definition).

## Departures caught
- No separate rerun input path: caught; changes the shared subject lookup.
- Plan reviews do not carry points between attempts: caught through the Workflow/code-review distinction in `rerun_carries`, though not stated directly.
- Revision files and record survive failure: caught; reuses existing artifacts and revisions.
- Additional departure: identifies the interval where a revision is recorded but the planner has not applied it; appropriately distinguishes sending owner feedback from applying it.

## Decisions to overturn
- “No record change” omits durable source provenance; reference decision 2 calls for reusing `carried_from` or adding one field.
- Hiding the prior revision when the next attempt completes conflicts with reference decision 4: differing prior copies should remain listed beside the reviewed copy. Display eligibility should not simply inherit source-selection eligibility.

## Over-specification
- Roughly 40% is expendable: empty signals/actions sections, exact helper signatures and redundant missing-source machinery, repeated contract details, overlapping unit/integration cases, and speculative README work.

## Hand-off
- Record the chosen source on the new attempt and preserve differing prior revisions in `show` after completion; trim implementation prescriptions.

## Notes
- best: Correctly locates the shared selection bug and distinguishes retained owner input from an applied revision.
- worst: Treats recomputed source eligibility as a substitute for durable provenance and historical display.