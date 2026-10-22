| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| L | 1 | 2 | 1 | 1 | no |

## Grounding
- `plan.md`: wrong (no such file in the tree; this describes a generated runtime artifact).
- `review/1/joined/plan.feedback-<n>.md`: wrong (no such file in the tree; `report::round_file` does confirm this runtime naming pattern).
- `scheduler.rs:7608-7614`: found (the revise path requires `Converged` or `AtCap`).
- `dispatch/src/store.rs:26`: found (`RECORD_VERSION` is 28).
- `dispatch/src/ticket.rs:350`: found (`Attempt::revisions` records owner revision rounds).

## Departures caught
- No separate rerun launch path: caught; the proposed source helper feeds the shared `start_workflow` launch.
- Only code reviews currently carry points: missed as an explicit correction; the plan does not claim otherwise, but leaves this distinction unexplained.
- Revision files and revision metadata already survive: caught; explicitly uses existing files and `revisions`.
- Additional finding: `review_subject` ignores context, confirmed by its lookup; keeping the new source selection context-specific is sensible.
- The proposed rejection of listing prior copies is a design disagreement, not a departure between the issue and existing code.

## Decisions to overturn
- Recomputing provenance instead of recording it conflicts with reference decision 2: reuse `carried_from`. Historical provenance should not change when files disappear or subsequent restarts occur.
- Showing only “started from” instead of a prior-revision entry conflicts with reference decision 4. Copies can diverge after launch; near-identical initial contents do not justify omitting the prior revision.

## Over-specification
- Roughly 40% could be cut: the unanswered-note detection and acknowledgement mechanism, repeated messaging across four surfaces, exact helper signatures, and extensive peripheral test scenarios. The reference requires preserving the revised copy, naming its provenance, and listing prior revisions.

## Hand-off
- No: resolve durable provenance and prior-copy display before implementation; also include the reference-required `docs/dispatch.md` updates.

## Notes
- best: Covers repeated failures while preserving the original planner session and restricting carry selection to the same context.
- worst: Replaces durable provenance and the requested prior-revision display with recomputation and an elaborate unanswered-note mechanism.