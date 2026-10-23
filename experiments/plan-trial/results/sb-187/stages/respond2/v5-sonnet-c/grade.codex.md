| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| v5-sonnet-c | 2 | 3 | 0 | 2 | note |

## Grounding
- `dispatch/src/review.rs`: found (implements code review, distinct from the plan workflow).
- `scheduler.rs:7611-7612`: wrong (these lines finish a warning; feedback construction and writing are at 7616–7618).
- `scheduler.rs:10534`: found (`review_subject` selects the newest Complete attempt holding the artifact).
- `facts.md` Settled, root cause: found (the supplied facts identify the Complete filter and shared `start_workflow` caller).
- `docs/design.md:2215`: found (describes review edits occurring in place under one path).

## Departures caught
- No separate rerun launch path: caught; changes the shared subject lookup.
- Plan reviews do not carry points between attempts: caught; distinguishes Workflow from code review and identifies the empty Workflow rerun text.
- Revision files and revision records survive failure: caught; uses existing artifacts and revisions without parallel state. “Lost revised plan text” is imprecise: selection loses it, storage does not.
- Additional finding: missing source files need fallback; useful and consistent with the reference’s on-disk condition.

## Decisions to overturn
- Leaving `show` unchanged conflicts with reference decision 4: list the prior revision beside the reviewed copy.
- Deferring provenance conflicts with decision 2: populate existing `carried_from` and widen its documentation.
- Substituting `docs/design.md` updates for the requested `docs/dispatch.md` updates conflicts with decision 7.
- Reporting only a revision count conflicts with decision 3’s owner round-N identification.
- Searching older failed attempts when the replaced attempt’s file is missing can select an older review copy; decision 1 instead falls back to the plan stage. The proposed question then incorrectly labels that older copy as plan-stage output.

## Over-specification
- Roughly one fifth is dispensable: exhaustive cancellation-site narration, exact helper signature and closure mechanics, pinned error strings, commit-message boilerplate, and the deferred-field discussion. The source/session distinction and integration test are useful detail.

## Hand-off
- Restore provenance and `show`, align fallback and revision wording with the reference, and update the user-facing dispatch documentation before implementation.

## Notes
- best: Separates the preserved review copy from the planner session used to start the fresh workflow.
- worst: Explicitly drops `show` visibility despite both the issue and reference requiring it.