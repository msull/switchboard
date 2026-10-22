| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| A | 3 | 3 | 2 | 2 | note |

## Grounding
- `dispatch/src/scheduler.rs:7436` `start_workflow`: found (fn signature begins at 7436; pushes the `AttemptKind::Workflow` attempt at 7487 as the plan says)
- `dispatch/src/scheduler.rs:10534` `review_subject`: found (filters `AttemptState::Complete`, exactly the bug the plan names)
- `docs/dispatch.md` "A review is a copy": found (bullet at line 550, as the plan later cites)
- `dispatch/src/scheduler.rs:7616` revise writes the round file: found (`round_file` + `atomic_write` at 7617-7619, `revisions.push` just after)
- `dispatch/src/ticket.rs:350` `revisions` on the attempt: found (`pub revisions: Vec<Revision>` at 350; `carried_from` at 317 and `Revision` at ~507 also check out)

Spot checks beyond five: `shown_where` 1476, `restarts` 1155, `fail_attempt_with` 7232/7263, `rerun_ask` 9335, `rerun_carries` 9382, `may_rerun` 10665, `starts_over` review.rs:3639, `response_file` report.rs:202, events.rs:695, `LaneFile` lib.rs:586, `ticket_paths` serve.rs:538, `print_lane_files` main.rs:896, `plan_label` ui/ticket.rs:425, `RECORD_VERSION` 28 at store.rs:26, `at_finalize`/`revise` helpers 3365/3417, docs/dispatch.md 3078, design.md 2160, README 448 all land within a line or two. Every citation I opened is real.

## Departures caught
- No rerun-specific path, fix is in the shared subject lookup: caught (names `review_subject`'s Complete filter as the cause, fixes beside it, keeps one launch path).
- Plan review carries nothing between attempts today: caught (`rerun_carries` "today says nothing for a workflow attempt"; adds a Workflow arm rather than claiming points carry).
- Revision not lost on disk or on the record: caught (round file, `revisions`, `revised` event all survive; reuses `carried_from`, adds no record field).
- Extra, not in the reference: the failed copy also carries the reviewer's in-place round edits, so the fix is wider than "the owner's revise"; and the note's *effect* is what is lost since a fresh run cannot take `workflow.object`, so the plan carries the copy, not the note. Also notes the response-file check for an unanswered owner round, which is grounded (`src/core/workflow.rs:73` writes `plan.response-<n>.md`).

## Decisions to overturn
- The ticket-page `plan_label` clause in `src/ui/ticket.rs`. The reference's decision 6 is no app-side change and the issue asks only about the rerun question and `show`; the wire field already serves `show`, so this is scope the implementer should not take on. One reversal.
- Not reversed but noted: the "start over" escape and the restart-boundary filter in `prior_copy` are additions the reference does not ask for; both are cheap and mirror existing code review behaviour, so I would keep them. The `attempt-started` event wording change is harmless but unasked.

## Over-specification
- The events.rs line change and its unit test, the ticket-page label and its design.md bullet, the README Layout edit, and a new design.md section are things an implementer would not need for this issue; the `prior_copy` unit tests for restart/forgotten cases are nice-to-have. Roughly 15-20% of the plan.
- Missed rather than over: the plan does not touch the `revise` row of the decisions table (docs/dispatch.md ~2975) or the rerun text around lines 318-350, which the reference expects.

## Hand-off
- Drop the ticket-page label clause and the events wording change; add the `revise` decisions-table row update to the docs step.

## Notes
- best: Pins the bug to `review_subject`'s Complete filter with every line reference accurate, and reuses `carried_from` and `revisions` instead of inventing state.
- worst: Spreads into the app's ticket page and the event log, which the issue did not ask for and the reference explicitly keeps out.