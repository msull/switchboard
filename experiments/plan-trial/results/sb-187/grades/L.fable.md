| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| L | 3 | 3 | 2 | 2 | note |

## Grounding
- `scheduler.rs:7608-7614` (objection only sent to a Converged/AtCap run): found (`if !matches!(view.state, RunState::Converged | RunState::AtCap)` inside `revise`, at 7608)
- `dispatch/src/store.rs:26` (`RECORD_VERSION` is 28): found (`pub const RECORD_VERSION: u32 = 28;` at line 26)
- `dispatch/src/ticket.rs:350` (Attempt's `revisions` and neighbours): found (`secret`, `forgotten`, `revisions` with its "pushed before `workflow.object`" doc at 350-360)
- `ticket.rs:510` (`Revision`): found (`pub struct Revision { round, by, at_ms }` at 509-515)
- `dispatch/src/restart.rs:34` (`DISCARDED_BY`): found (`pub const DISCARDED_BY: &str = "discarded by restart at "` at 34)

Spot-checked beyond the five: `review_subject` at 10534 and its context-free walk, `start_workflow` at 7436 with the `review_subject` call at 7462 and the copy at 7469-7472, the `since` cutoff at 7230-7241, `rerun_carries` at 9382 and `rerun_ask` at 9336-9373, `SENT_BACK_FROM` at 8979, the finalize question at 7693-7696, the "a rerun starts a fresh review" comment at 7812-7814, `LaneFile` at `dispatch-control/src/lib.rs:586` with `#[serde(default)]`, `ticket_paths` at `serve.rs:538`, `print_lane_files` at `main.rs:896`, `round_file`/`response_file` at `report.rs:156/202`, `input_where` at `ticket.rs:1452`, all six test helpers in `first_slice.rs`, and `docs/design.md:1166`. All correct.

## Departures caught
- No rerun-specific path; the fix is in the shared subject lookup: caught in effect. The plan adds `review_source` beside `review_subject` and calls it from `start_workflow` for every launch, not from a rerun branch. It never says "there is no rerun path" outright.
- The plan review carries nothing between attempts: caught implicitly. The plan notes `rerun_carries` has no Workflow arm and says the session stays the complete writer's. It never claims points carry.
- The revision is on disk and on the record, so no new state: caught explicitly (`plan.feedback-<n>.md` and `Attempt::revisions` survive; "No record change").
- Found and the reference lacks: listing attempt 1's copy beside attempt 2's shows two near-identical plans once the carry works, so the plan annotates the open copy with where it started instead. Also that `review_subject` ignores context (a real latent bug, correctly left alone), and that a Failed attempt a restart left Failed is cut off by the same `since` rule `ask_rerun` uses.

## Decisions to overturn
- The `unanswered` round machinery: a second `LaneFile` field, a `still_unanswered` helper, a change to the `finalize` question, and three of the ten tests exist for the case where the run died before the planner edited the copy. The issue's own trace shows the revised copy written before the failure. The reference (decision 5) treats the feedback file as optional to carry and would not route it anywhere; it says nothing that requires naming it in four places. I would reverse this to, at most, the log line, and ship the carry first.
- Not reversed, though the reference differs: `show` annotating the open copy with `(started from <path>)` instead of listing the prior copy as its own line (reference decision 4). The issue's "at least say which copy" is met and the prior path is still printed. Recomputing provenance instead of reusing `carried_from` (decision 2) is zero new record fields, which the reference allows.

## Over-specification
- The `unanswered` apparatus and its tests, roughly a fifth of the plan, as above.
- Empty sections ("The signals: No new signal", "README: no new file") and the restated `Attempt` field list in "The record".
- Three pinning tests (send-back, no-revise, later stage reads finalized) are useful but not required by the issue. Altogether about a quarter an implementer could skip.

## Hand-off
- Implement the carry walk, the Workflow arm in `rerun_carries`, and the `started_from` line in `show` first; defer the `unanswered`/`still_unanswered`/finalize-question additions to a follow-up unless the owner asks for them.

## Notes
- best: the carry rule is bounded by exactly the cutoffs the code already uses (Complete, `DISCARDED_BY`, `SENT_BACK_FROM`, the last restart's `at_ms`), with the double-failure and per-lane cases worked out and every anchor verified.
- worst: the "owner round never answered" tracking roughly doubles the surface of the change for a case the issue does not report.