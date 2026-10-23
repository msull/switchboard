| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| v5-sonnet-a | 3 | 3 | 1 | 1 | note |

## Grounding
- `facts.md`: found (supplied text; the cited sections discuss surviving files and revision-record limitations).
- `scheduler.rs:10534`: found (the shared subject lookup selects the newest Complete attempt with the requested artifact and a session).
- `scheduler.rs:7607-7647`: found (records the revision before sending `WorkflowObject`; refusal removes the feedback file without removing the revision).
- `dispatch/src/ticket.rs:348`: found (the revisions comment incorrectly says refused revisions are popped).
- `dispatch/src/ticket.rs`: found (contains `Attempt`, its revision history, and existing `carried_from` provenance).

## Departures caught
- No separate rerun path: caught; replaces the shared subject lookup.
- Plan reviews do not carry points between attempts: caught implicitly; preserves fresh `WorkflowStart` and identifies the currently empty Workflow arm in `rerun_carries`.
- Revision files and revision records already survive failure: caught; proposes no duplicate storage for the owner's note.
- Additional departure: correctly identifies that a recorded revision can survive a refused round, contradicting the record's comment. However, a response file does not establish that the copy is final, as the plan itself acknowledges.

## Decisions to overturn
- Remove the response-file prerequisite and prefer the replaced attempt's existing copy. Reference decision 1 permits a revision-only restriction, but does not require proof of completion. The proposed rule can discard existing owner edits and, on repeated failures, bypass newer edits in attempt 2.
- Make `show` expose prior revisions independently of `revised_from`. Reference decision 4 requires the earlier differing copy beside the reviewed copy; the proposed UI hides precisely those copies rejected by its response-file gate.

## Over-specification
- Roughly one-third is expendable: unresolved-question labels, speculative completion-order discussion, exact helper shapes and sentences, repeated contract details, commit-body instructions, and tests cementing the unnecessary response-file gate. One new provenance field is acceptable under the reference, although reusing `carried_from` would be simpler.

## Hand-off
- Prefer the replaced attempt's on-disk copy without response-file gating, and show prior revisions even when they were not selected as the source.

## Notes
- best: Finds the shared selection bug and the genuine mismatch between revision-record documentation and refusal handling.
- worst: Turns uncertain revision completion into a fallback rule that can reproduce the owner's loss of edits.