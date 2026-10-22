| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| G | 1 | 2 | 0 | 1 | no |

## Grounding
- `review/1/joined/plan.md`: wrong (absent from the checked-out tree; this is a runtime artifact, although the copy-generation code supports the path).
- `plan.feedback-<round>.md`: wrong (a filename template, not an existing file; `revise` generates the corresponding round file).
- `dispatch/src/scheduler.rs:7565`: found (`revise` reads the answered decision, writes feedback, records a revision, and sends `WorkflowObject`).
- `:7616`: found (constructs the feedback filename using `report::round_file`).
- `scheduler.rs:10534`: found (`review_subject` selects the latest Complete attempt with the subject artifact and requires its session).

## Departures caught
- No separate rerun path: caught; changes the shared subject selector used by `start_workflow`.
- Plan reviews do not carry rounds or points between attempts: caught; describes fresh rounds and preserves the separate code-review behavior.
- Revision files and revision metadata already survive: caught with a wrong fix; acknowledges both, but makes reuse depend on an additional response-file signal.
- Additional finding: revision metadata alone does not prove the planner finished applying the note. Useful distinction, but response-file absence also does not prove the copy lacks edits; the reference deliberately requires no such detection.

## Decisions to overturn
- Remove the response-file eligibility requirement. The reference accepts restricting reuse to revised attempts, but does not require proof of completion; this gate can discard an edited copy.
- Record the source on the new attempt using `carried_from`. Explicitly declining provenance conflicts with reference decision 2.
- Remove the instruction to repeat the note in the rerun answer as a recovery mechanism. `rerun_note` excludes Workflow stages, and `WorkflowStart` takes no note; this conflicts with reference decision 5.
- Narrow `show` to the relevant prior review revision. Listing every other plan artifact, including ordinary plan-stage copies, exceeds reference decision 4’s single additional line.

## Over-specification
- Roughly one-third is expendable: speculative schema-migration instructions, unchanged app version numbers, empty signals/actions declarations, README non-changes, repeated contract details, and extensive tests formalizing the unsupported response-file gate.

## Hand-off
- no: the selector can still discard owner edits, the suggested note recovery does not work, and source provenance is deliberately omitted.

## Notes
- best: Correctly identifies the shared Complete-only selector and preserves the planner-author session.
- worst: Treats a missing response file as permission to discard the copy, then offers a recovery route unavailable to Workflow attempts.