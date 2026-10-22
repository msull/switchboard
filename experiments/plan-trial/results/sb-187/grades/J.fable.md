| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| J | 3 | 3 | 2 | 2 | note |

## Grounding
- `dispatch/src/scheduler.rs:10534` review_subject: found (the function starts at 10534 and filters `a.state == AttemptState::Complete`, exactly the bug site)
- `dispatch/src/ticket.rs:1476` shown_where: found (`pub fn shown_where` at 1476, keeps Complete or open Workflow attempts)
- `dispatch/src/serve.rs:539` ticket_paths: found (`pub fn ticket_paths` at 539, builds `plan_files` from `shown_lane_files`)
- `scheduler.rs:9382` rerun_carries: found (`fn rerun_carries` at 9382, empty string for anything but Resolution and Review kinds)
- `dispatch/src/main.rs:1176` show prints the owner's note: found (`print_answered` at 1176 appends `— {note}` to each answered decision line)
- Spot-checked beyond five: `revise` at 7565, the `revisions.pop()` refusal arm at 11313, `RECORD_VERSION = 28` at store.rs:26, `plan_label` at src/ui/ticket.rs:427, the four named tests, `PathsView` at dispatch-control lib.rs:546 and `LaneFile` at 586, design.md 1785 and 2160, dispatch.md 2975. All real, line-accurate.

## Departures caught
- No rerun-specific path; fix is in the shared subject lookup: caught. The plan edits `review_subject` itself and names the `Complete` filter as the cause.
- Plan review carries nothing between attempts today: caught. "rerun_carries speaks only for code review"; the new attempt starts with no `revisions`.
- Revision already on disk and on the record: caught. Gates on the existing `Attempt.revisions`, adds no ticket field.
- Found beyond the reference: the owner's note survives on the answered finalize decision and is already printed by `show`, so only re-sending it is missing; a `workflow.object` needs a Converged/AtCap run with round = count + 1, so a fresh run cannot receive it (verified at design.md:1190). Also that gating on `plan.response-N.md` would miss the repro because a run killed before the planner answers leaves no response file, and that the record constant is `RECORD_VERSION`, not `SCHEMA_VERSION`.

## Decisions to overturn
- Widening `shown_where` to keep a failed Workflow attempt with revisions, plus a `plan_failed` arm in the app's Plan tab with its UI test and two design.md edits. The reference's decision 4 is one extra `files:` line in `show`, and decision 6 is no app-side change. The `prior_plans` line alone satisfies the issue; changing what "the plan as it stands" means everywhere is a broader semantic shift than the issue asks for.
- Not counted but noted: no provenance on the new attempt (reference reuses `carried_from`). The plan derives it at display time from the failed attempt's `revisions`, which works, so I did not reverse it. Also the plan leaves `docs/dispatch.md`'s rerun section (318-350) untouched while editing design.md; I would add the dispatch.md sentence, but that is an omission rather than a wrong call.

## Over-specification
- The second test pinning that a rerun's own round 1 reads as the reviewer's (tests existing behaviour the plan already argues from), the empty "The signals" section, the `USAGE_VERSION` line, the "Name check" bullet referring to investigation notes not in the issue, the new dated design.md status section for a bug fix, and the four check commands the hook forces anyway. Roughly 15 to 20 percent.

## Hand-off
- Drop the Plan tab arm, the `shown_where` widening and the dated design.md section; keep `prior_plans` and the `failed` flag for `show`, and add a sentence to dispatch.md's rerun section saying which copy a plan-review rerun starts from.

## Notes
- best: every one of some forty line references is accurate, and the plan explains why the `revisions` gate beats a response-file gate for the exact repro shape.
- worst: it reaches into the app's Plan tab and redefines the shown plan when the issue asks for `show` and the rerun question only.