| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| I | 3 | 3 | 2 | 2 | note |

## Grounding
- `dispatch/src/ticket.rs:1856`: found (the `shown_where` unit test's loop over Cancelled "parked" and Failed states asserting the review copy is no longer the shown plan, lines 1856-1864)
- `scheduler.rs:7616-7618`: found (`round_file`, the owner-heading body, `atomic_write` of the feedback file inside `revise`)
- `scheduler.rs:7619-7626`: found (`find_attempt_mut` then `revisions.retain`/`push(Revision { round, by, at_ms })`)
- `scheduler.rs:7608`: found (`if !matches!(view.state, RunState::Converged | RunState::AtCap)` warn-and-return, so `revise` only reaches a finished run)
- `dispatch/src/scheduler.rs:7379`: found (`fn workflow_stage`, which calls `start_workflow` at `:7403` once `may_rerun` allows)
- Beyond the five: `start_workflow` `:7436`, `review_subject` `:10534` with the `Complete` filter, `rerun_carries` `:9382`, `new_attempt` `:10277`, `Attempt` `:266` / `carried_from` `:317`, `RECORD_VERSION = 28` at `store.rs:26`, the `27 to 28` migrate comment at `:608`, `main.rs:981-994`, `serve.rs:538`/`:504`, `PathsView` at `dispatch-control/src/lib.rs:546`, all four named tests, and the docs anchors at `:550`, `:393-401`, `:3171`, `:3078` all check out. No wrong reference found.

## Departures caught
- No rerun-specific path; the fix belongs in the shared subject lookup: caught. The plan traces `workflow_stage` → `start_workflow` → `review_subject` and names the `Complete` filter as the cause; it patches `start_workflow` beside `review_subject` rather than adding a rerun-only branch (first attempts see `latest_attempt` as None).
- Plan reviews carry nothing between attempts today: caught. The plan never claims points carry, says reviewer-round edits "come back when the new attempt's rounds run again", and adds a Workflow arm to `rerun_carries`.
- Revision is not lost and the record already knows: caught. The plan says so explicitly, keys the rule on non-empty `revisions`, and reuses the on-disk copy and feedback file.
- Found that the reference lacks: `revise` is only accepted by a Converged/AtCap run, so the note cannot be resent to the new run (grounds decision 5 better than the reference does); the `RECORD_VERSION` "take the value at implementation time plus one" caveat for parallel landings; the chain case where a carried attempt fails again without its own `revise`.

## Decisions to overturn
- Carrying only from a `Failed` attempt, never a `Cancelled` one. The reference's decision 1 is "the replaced attempt of the same stage and context when its copy is on disk". `rerun_ask` and `may_rerun` apply to cancelled attempts too, and `restart --note` cancels and relaunches without asking, so an owner's revision would still be silently dropped on that path. The plan's justification (`shown_where` treats parked as not the plan) is about display, not about what an explicit `rerun` should start from. I would widen it.
- Not overturned but noted: a new `revised_from` field instead of reusing `carried_from` is within the reference's "a new field is acceptable"; the version bump follows the store's own convention (27→28 added serde-default fields the same way).

## Over-specification
- The `show` addition is heavier than needed: a five-field `PriorRevision` struct with `owner_rounds` and a `carried` flag computed by following a `revised_from` chain, plus a three-line block with two alternative closing lines. The reference wants one extra `files:` line. Roughly a third of the implementation detail.
- "Signals: none new", the README "nothing changes" line, and the Commit section's contract list add little; the cause walk restates code an implementer would read anyway. Altogether about 20% to cut.

## Hand-off
- Carry the copy from any replaced attempt whose copy is on disk, cancelled included, and shrink the `show` change to one `prior revision` line per attempt with its path and `carried` flag.

## Notes
- best: every reference verified real and on point, and the cause chain (`workflow_stage` → `start_workflow` → `review_subject`'s `Complete` filter) is exactly the bug.
- worst: the `show`/`PriorRevision` design grows a one-line listing into a nested sub-report with a carried-chain computation.