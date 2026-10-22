| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| E | 3 | 3 | 2 | 2 | note |

## Grounding
- `start_workflow` (`dispatch/src/scheduler.rs:7436`): found (the `fn start_workflow` signature begins at 7436, after the `too_many_arguments` allow)
- `review_subject` (`scheduler.rs:10534`): found (the function with the `state == Complete` filter and the "nothing wrote {subject}" error)
- `review run gone` (`scheduler.rs:7761`): found (`Reply::Failed` arm with `NO_SUCH_RUN` calling `fail_attempt` with "review run gone: {reason}")
- `review: <reason>` paused-and-failed (`scheduler.rs:7823`): found (`RunState::Paused { failed: true }` arm, `cancel_pending(t, a, "paused")` then `fail_attempt` with "review: {reason}")
- `could not start the review` (`scheduler.rs:7525`): found (the launch failure arm of `start_workflow`)
- Beyond five: every other line I checked also matched (`ticket.rs:350` revisions, `:1476` shown_where, `rerun_carries` 9382, `rerun_ask` 9335, `store.rs:26` = 28, `report.rs:202` response_file, `main.rs:981`, `serve.rs:539`, `control/src/lib.rs:215` WorkflowStart, `first_slice.rs:3416/3810`, `tests/ui.rs:5859`, `docs/dispatch.md:550`, `model.rs:12` = 16 and `:612` = 7).

## Departures caught
- No rerun-specific path; the bug is `review_subject`'s Complete filter shared by every attempt: caught, precisely, and named as the fix site.
- Plan review carries nothing between attempts today: caught (notes `rerun_carries` returns an empty string for a Workflow attempt and never claims points carry).
- The revision is not lost; `revisions` and the files on disk already record it: caught ("already holds" bullet, cites `revisions` and `plan.feedback-<n>.md`, adds no store for the note).
- Beyond the reference: the plan observes that every end short of `finalize` (paused-and-failed, park-and-resume cancellation, launch failure) has the same effect, so the fix must not be in the `no such run` arm. Also notes `shown_where` listing only Complete or open Workflow attempts is by design, not a second bug.

## Decisions to overturn
- The `show` lineage grows into a new eight-field `PlanCopy` wire type, a `plan_copies` list, a new `print_plan_copies`, plus a second meta line on the app's ticket page Plan tab with its own UI test. The reference asks for one extra `files:` line and no app-side change. I would cut the ticket page change and shrink the wire addition to what one line needs.
- Not reversed but noted: a new `reviewed_from` field rather than widening `carried_from`. The reference allows one new field. The `RECORD_VERSION` bump follows the repo's existing practice for added fields, so I would keep it. Copy-source rule, session rule, the rerun-question sentence, and keeping the note out of `t.rework` all match the reference.

## Over-specification
- "The signals" and "The actions and effects" sections say nothing. The "Contract additions" block restates "The record" and "The UI". The paragraph on `SCHEMA_VERSION` and `VIEWS_SCHEMA_VERSION` not changing, the commit message, the README check, and the ticket page section are not needed. Roughly a fifth to a quarter of the plan.

## Hand-off
- Keep the `review_input` rule, the rerun sentence, and the tests as written. Drop the ticket page meta line and reduce `plan_copies` to one extra `show` line naming the prior revised copy.

## Notes
- best: pins the whole bug to one filter in `review_subject` and proves every failure path shares it, with every cited line real.
- worst: the display side balloons into a new wire struct and an app Plan-tab change the issue never asked for.