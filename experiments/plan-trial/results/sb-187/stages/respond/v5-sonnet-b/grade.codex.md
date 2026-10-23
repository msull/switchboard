| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| v5-sonnet-b | 3 | 3 | 0 | 2 | no |

## Grounding
- `src/core/control.rs:441-455`: found (passes the supplied source and plan into `start_workflow`).
- `facts: Settled, "The planner clone and the source session"`: found (supplied facts describe transcript cloning and the app accepting the supplied path).
- `ticket.rs:346-350`: found (defaulted `revisions` vector records owner objections).
- `ticket.rs:510`: found (`Revision` contains round, author, and timestamp).
- `facts: Settled, "What survives"`: found (describes surviving decisions, feedback files, revision metadata, and revised plan files; metadata does not contain the note text).

## Departures caught
- No separate rerun launch path: caught; changes the shared subject lookup.
- Plan reviews do not carry rounds or points: caught; explicitly distinguishes their fresh workflow from code-review carry behavior.
- Revision survives on disk and in existing metadata: caught; uses `revisions` to select a copy.
- Additional observation: Workflow rerun notes have no consumer; correctly leaves that outside scope.

## Decisions to overturn
- Restrict selection to the replaced attempt’s stage and context. The proposed global subject scan can select another workflow’s revised copy, conflicting with reference decision 1.
- Fall back to the plan output when the previous copy is missing. Parking instead directly conflicts with reference decision 1; its caution is understandable but is a different policy.
- List the prior revision’s path beside the reviewed copy. A provenance annotation and pending question do not satisfy reference decision 4’s explicit additional `files:` entry.
- Using only revised attempts is expressly acceptable under the reference. One new persisted provenance field is also acceptable; the corresponding transport field is not a second independent provenance record.

## Over-specification
- Roughly one quarter is removable: empty signals/actions sections, repeated contract inventories, commit wording, unrelated negative assurances, and overlapping unit/integration test prescriptions.

## Hand-off
- No: source selection, missing-file behavior, and prior-copy visibility need correction before implementation.

## Notes
- best: Identifies the shared lookup bug and preserves the distinction between revised file content and workflow transcript history.
- worst: Claims both visibility requirements are covered while deliberately omitting the prior revision from `files:`.