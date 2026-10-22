| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| C | 2 | 3 | 1 | 1 | note |

## Grounding
- `dispatch/src/store.rs:26`: found (`RECORD_VERSION` is 28).
- `src/core/model.rs:12`: found (`SCHEMA_VERSION` is 16).
- `dispatch/src/scheduler.rs:7565`: found (`revise` writes feedback beside the subject and records a `Revision`).
- `review/1/joined/plan.md`: wrong (absent from the checkout; this is a runtime artifact example, not a checked-in file).
- `scheduler.rs:7436`: found (under `dispatch/src`; `start_workflow` obtains the subject and session through `review_subject`, then copies the subject).

## Departures caught
- No separate rerun launch path: caught; identifies the shared launch and Complete-only subject lookup.
- Plan reviews do not carry points between attempts: caught; distinguishes code-review carry behavior and acknowledges fresh plan-review rounds.
- Revision files and revision metadata survive failure: caught; reuses the existing copy and record.
- Additional finding: Workflow rerun notes are currently excluded by `rerun_note`; verified. This does not itself justify expanding their semantics.

## Decisions to overturn
- Replace provenance-only `show` output with the reference’s explicit prior-revision entry when copies differ. A source copy can diverge once the new attempt edits its own copy.
- Remove the Workflow rerun-note/`t.rework` expansion and “start over” feature. It conflicts with the reference’s instruction against routing owner feedback through `t.rework`; although this plan routes rerun notes rather than the original revise note, it adds unnecessary note semantics to a workflow whose start signal accepts no note.

## Over-specification
- Roughly one-third to two-fifths is avoidable: cross-attempt round aggregation, Plan-tab changes, associated protocol fields and UI tests, “start over” plumbing, repeated contract summaries, and explicit no-change declarations. The reference calls for a copy-selection fix, provenance, a small `show` addition, regression coverage, and documentation.

## Hand-off
- Keep the shared-launch copy fix and `carried_from`; add the explicit prior-revision line and drop rerun-note semantics and cross-attempt UI expansion.

## Notes
- best: Correctly separates the failed attempt’s surviving revised copy from the original planner session needed to launch a fresh workflow.
- worst: Explicitly substitutes provenance for the requested prior-revision display while substantially expanding unrelated UI and note behavior.