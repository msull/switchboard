| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| C | 3 | 3 | 1 | 2 | note |

## Grounding
- `dispatch/src/store.rs:26` `RECORD_VERSION = 28`: found (exactly that constant at line 26)
- `src/core/model.rs:12` `SCHEMA_VERSION = 16`: found (exactly that constant at line 12)
- `dispatch/src/scheduler.rs:7565` `revise`: found (`fn revise(` at 7565, doc comment above it describes the round/revision semantics the plan relies on)
- `scheduler.rs:7436` `start_workflow`: found (`fn start_workflow` at 7436; the `review_subject` call, `attempt_dir`, `std::fs::copy` and the `t.attempts.push(new_attempt(...))` are at the lines the plan cites, 7468-7491)
- `scheduler.rs:10534` `review_subject`: found (filter `a.state == AttemptState::Complete` at 10539, `find_map` over `artifacts.get(subject)`, the two error strings the plan says it preserves)

Spot checks beyond five: `ticket.rs:311-322` carried_from/rework docs, `carry_of` callers at `review.rs:548/815/1776`, `report.rs:414` kind filter, `rerun_note` `matches!` at 8992-8995, `rerun_carries` at 9382 returning `&'static str`, `starts_over` at `review.rs:3639`, the "note leaves only with the attempt" comment at 6279-6280 and `remove` at 6370, `retire_replaced` insert at 3600, `sent_back` at 10601, `LaneFile` at `lib.rs:586`, `PlanRoundView` at 604, `ticket_paths` at `serve.rs:538` with `plan_rounds` built at 564-572 and `round_file` at 573, `main.rs:896/981/991-994`, `ui/ticket.rs:154` and 454-470, `first_slice.rs:3809`, `scheduler.rs:11338` `mod tests`, `ticket.rs:1827`, and every docs line (design.md 1785, 2160; dispatch.md 550, 2830, 2975, 3058-3060). All correct.

## Departures caught
- No rerun-specific path; the fix is in the shared subject lookup: caught. The plan names `review_subject`'s Complete filter as the cause and adds `revised_copy` beside it, consulted by every `start_workflow`, not a rerun branch.
- Plan review carries nothing between attempts today: caught. It verifies `carried_from` is read only by code review paths and lists "no REVIEW_CARRIED-style prompt for plan reviews" as a known gap rather than claiming points carry.
- Revision is not lost on disk and the record knows: caught. First bullet says the note and `Revision` survive; only the input choice is wrong; reuses `carried_from` instead of the investigation's `started_from` field.
- Extra, not in the reference: `rerun_note` excludes `StageKind::Workflow`, so a note on a plan review's `rerun` answer is dropped today; and `sent_back` excludes Workflow stages. Also the partial-edit caveat: a planner that dies mid-round leaves a half-edited copy that Dispatch cannot distinguish from a finished one.

## Decisions to overturn
- Listing an earlier attempt's rounds in `show` (`plan round N (owner, review/1)`), adding `PlanRoundView::attempt`, walking `carried_from` in `ticket_paths`, and retitling folds and the finalize count in the app's Plan tab. Reference decision 4 asks for one extra `files:` line, "not a redesign of `show`", and decision 6 says no app-side change. The plan's `(started from <path>)` line already satisfies the issue; the rounds machinery and `src/ui/ticket.rs` edits with three tests are scope the issue never asked for.
- The "start over" opt-out plumbed through `rework`: widening `rerun_note` to Workflow stages, storing the rerun note under `rework_key` in `retire_replaced`, setting `Attempt::rework` on Workflow attempts, and making `starts_over` `pub(crate)`. Reference decision 5's reason applies: `WorkflowStart` takes no note, so the stored note is a dead letter except as a two-word switch. If the switch is wanted, read the answered decision's note directly; the reference's fallback is "copy missing on disk", nothing more.
- Agree with the reference on the rest: default to the replaced attempt's copy (revised or not), reuse `carried_from`, Workflow arm in `rerun_carries`, source session from `review_subject`, no `WorkflowStart` change, no version bump.

## Over-specification
- Roughly 15 to 20 percent. The version preamble and the "Rehydration is derived from the record" line add little. The six-case `revised_copy` unit test and the two serve tests for the rounds listing would be cut with the reversed decisions. The docs section is long but the first-slice table rows and the `CHANGELOG` line are repo conventions, and the commit's "Contract additions" list is required by CLAUDE.md.

## Hand-off
- Implement the `revised_copy` picker, `start_workflow` copy choice, `carried_from` on the Workflow attempt, the `rerun_carries` arm and the `(started from …)` line; drop the earlier-rounds listing, the Plan tab changes and the `rework`/`rerun_note` plumbing.

## Notes
- best: pins the bug to `review_subject`'s Complete filter and proves `carried_from` is safe to reuse by listing every reader, so no record field or version bump.
- worst: grows into listing earlier attempts' rounds in `show` and the app's Plan tab, with a `rework` detour for a "start over" switch, none of which the issue asked for.