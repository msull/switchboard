| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| v5-sonnet-a | 3 | 3 | 2 | 2 | note |

## Grounding
- `review_subject` (`scheduler.rs:10534`): found (the fn is at 10533-10548, `.filter(|a| a.state == AttemptState::Complete)` is the one filter)
- `scheduler.rs:7607-7647` (revise pushes `Revision` before sending, keeps it on refusal): found (push at 7620-7626, refusal arm at 7636-7647 removes the feedback file only; the `Revision` is not popped)
- `dispatch/src/ticket.rs:348` (`revisions` doc comment): found (the doc says "popped when Switchboard refuses it", which the code does not do; the plan's correction is right)
- `new_attempt` (`scheduler.rs:10277`): found (fn starts at 10276, sets no `carried_from`, so `revised_from = None` there is the right default site)
- `RECORD_VERSION` (`store.rs:26`): found (`pub const RECORD_VERSION: u32 = 28;`)
- Also checked: `rerun_carries` at 9382 returns `&'static str` with "" for Workflow; callers at 7266 and 9373 both `format!` it; `response_file` at report.rs:202; `LaneFile` at lib.rs:586 is `#[serde(default)]`; `print_lane_files` main.rs:896; `review_run_fails`:3249, the failed-rerun test :3257, `revise` :3417, `at_finalize` :3365; design.md:1783-1790 "a rerun kills the old run's agents and starts a fresh run"; dispatch.md:393 is the version list. All as cited.

## Departures caught
- No rerun-specific path, fix belongs in the shared subject lookup: caught. The plan replaces `review_subject` at its one caller in `start_workflow` with `plan_to_review`, so every attempt goes through it.
- Plan review carries nothing between attempts today: caught implicitly. It states `rerun_carries` returns "" for a Workflow attempt and that the new planner is a clone of the plan stage's original session that has seen neither the note nor the revision. It never claims points carry.
- Revision is not lost on disk and the record knows: caught. It says the note already survives, the copy is intact, and adds only one provenance field plus a wire field for `show`.
- Found beyond the reference: the `revisions` doc comment claims the entry is popped on refusal but the code keeps it, so a non-empty `revisions` alone does not prove a finished revision. Confirmed at 7636-7647. Also the ordering gap between the planner's plan edit and the response file (model.rs:279-284 does not order them; workflow.rs:663-668 marks responded on file appearance), which the reference does not mention.

## Decisions to overturn
- Gating the copy on a response file for the latest revision round, with a `skipped` branch and a third question wording when the file is missing. The reference takes the replaced attempt's copy whenever it is on disk, revised or not, and allows "only if revised" as a restriction. The response-file gate is stricter than that, the plan itself admits it does not prove the edit is complete, and a run killed between the edit and the response file would discard the revision again, which is the issue's bug. I would reverse it to "copy exists on disk, use it" and drop `skipped`.
- Everything else matches the reference: one new record field (`revised_from`, a new field is acceptable), a Workflow arm in `rerun_carries` naming the copy, one extra `show` line, the note not routed through `t.rework`, no app change, an integration test through `revise` then `review_run_fails` then `rerun`, docs and changelog. The `RECORD_VERSION` bump follows the repo's convention for defaulted fields (version 19 did the same for `revisions`).

## Over-specification
- The "Why these choices" block on U2 and the respond-prompt ordering, the instruction that the implementer re-read the template and write about it in two docs, the `skipped` tuple and its question wording, the store fixture test, and the design.md "Plan tab does not show prior" gap note. Roughly 15 to 20 percent. The facts-file shorthand (S, U1, O1) also makes the plan harder to read standalone than it needs to be.

## Hand-off
- Drop the response-file gate and the `skipped` branch: take the replaced attempt's copy whenever it is on disk, keep two question wordings, and leave the rest as written.

## Notes
- best: Places the fix in the shared `review_subject` lookup and catches that the `revisions` doc comment lies about popping on refusal, a departure the reference missed.
- worst: The response-file gate and `skipped` machinery add a walk, a tuple, and a third question wording for an edge the plan admits it cannot prove, and can re-discard a revision in the issue's own failure mode.