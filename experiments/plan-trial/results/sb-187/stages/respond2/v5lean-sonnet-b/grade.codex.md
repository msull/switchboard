| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| v5lean-sonnet-b | 3 | 2 | 0 | 1 | no |

## Grounding
- `facts.md`: found (supplied text; its Settled section documents the persisted note, Complete-only lookup, and cancelled-attempt rerun).
- `dispatch/src/scheduler.rs:7616-7642`: found (writes feedback, records the revision, sends `WorkflowObject`, and removes feedback on explicit refusal).
- `scheduler.rs:10534-10548`: found (`review_subject` selects the newest Complete attempt holding the artifact and requires its session).
- `:7465`: found (the shared workflow launch calls `review_subject` before copying the artifact).
- `:9363-9368`: found (cancelled attempts receive `rerun` and `park` options).

## Departures caught
- No separate rerun path: caught; identifies the shared subject lookup and changes shared workflow launch.
- Plan reviews do not carry points between attempts: missed as an explicit distinction, although the plan does not incorrectly propose carrying points.
- Revision survives on disk and is already recorded: caught; uses existing feedback and `revisions`.
- Additional departure: identifies cancelled attempts after park/resume as another affected case, supported by the shared launch and cancelled rerun arm.

## Decisions to overturn
- Remove the new `original` answer and its cross-module contract changes. The reference chooses automatic carry-forward with provenance; an additional reset choice is unnecessary scope.
- Remove the currency veto that silently substitutes a newer completed plan. Reference decision 1 chooses the replaced same-stage/context attempt’s existing copy; this veto can discard owner input again.
- Replace the provenance-only `show` annotation with an additional prior-revision path when copies differ. Reference decision 4 requires both copies to be listed.
- Document rerun/revise behavior in `docs/dispatch.md`, rather than only the record version. Reference decision 7 explicitly requires those behavioral updates.

The single new provenance field is acceptable under reference decision 2, although reusing `carried_from` would be preferable.

## Over-specification
- Roughly 40% is avoidable: the `original` answer machinery and associated tests, currency policy, repeated contract inventory, empty signals section, and commit-message prescription. These obscure the comparatively small copy-selection and display change.

## Hand-off
- no: resolve the copy-selection policy and missing prior-revision display before implementation; the proposed reset-answer contract adds substantial unnecessary scope.

## Notes
- best: Correctly identifies durable revision evidence and preserves it across chains of failed attempts.
- worst: Adds a policy that can discard the revised copy again while failing to list that prior copy in `show`.