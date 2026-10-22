| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| D | 3 | 3 | 1 | 1 | note |

## Grounding
- `docs/design.md:1002`: found (describes a workflow as a durable record).
- `dispatch/src/scheduler.rs:7755`: found (`poll_workflow` fails the attempt on `NO_SUCH_RUN`).
- `src/app.rs:3915-3918`: found (returns “no such run” when `run_view` finds no record).
- `src/core/workflow.rs:1009`: found (`remove_workflow` removes the stored workflow).
- `dispatch/tests/first_slice.rs:3809`: found (the named test clears `runs` and checks failure plus a pending rerun decision).

## Departures caught
- No separate rerun launch path: caught; changes the shared `start_workflow` input selection.
- Only code reviews currently carry points: caught; distinguishes Workflow provenance from existing code-review consumers.
- Revision files and revision metadata already survive: caught; uses existing artifacts, revisions, and `carried_from`.
- Additional departure: correctly distinguishes a missing workflow record from a runner restart alone. The cited code supports that distinction, although it cannot exclude every restart-related failure scenario.

## Decisions to overturn
- Replace provenance-only `show` output with visibility of differing prior revisions, as reference decision 4 requires. `started_from` alone hides prior copies whenever carry selection falls back to the plan output.
- Include the owner’s revision round in the rerun question, as reference decision 3 requires; the proposed generic “owner’s revision” omits it.

## Over-specification
- Roughly one-third is unnecessary prescription: exact helper signatures and question strings, an unanswered-note detection and finalize-warning subsystem, schema-bump contingencies despite no schema change, unchanged documentation sections, and a predetermined commit message. Missing response files also do not prove that the plan was never edited.

## Hand-off
- Make prior-revision visibility independent of carry provenance, name the owner’s revision round, and trim the unanswered-note subsystem.

## Notes
- best: Reuses existing revision metadata and provenance while preserving the planner session and carrying revisions through repeated failures.
- worst: Treats “started from” provenance as sufficient prior-revision visibility, leaving fallback cases opaque.