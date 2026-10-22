| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| G | 3 | 3 | 2 | 2 | note |

## Grounding
- `revise`, `dispatch/src/scheduler.rs:7565`, file at `:7616`: found (`fn revise` opens at 7565; `let file = crate::report::round_file(&subject, round)` is line 7616)
- `review_subject` (`scheduler.rs:10534`): found (the function, with the `AttemptState::Complete` filter the plan blames)
- `start_workflow` (`:7436`), copy at `:7465` to `:7471`: found (fn at 7436; `review_subject` call at 7465, `std::fs::copy` at 7471)
- `report::response_file`, `report.rs:202`: found (`pub fn response_file(subject, n)` at 202; the app's `respond_to_user` template does tell the planner to "Provide a response in markdown at {response}", so the applied-signal claim holds)
- `rerun_carries` (`:9382`): found (returns `&'static str`, empty arm for a Workflow attempt; callers at 7266 and 9373 as the plan says)

Also spot-checked: `ticket.rs:1476` shown_where, `ticket.rs:350` revisions, `store.rs:26` RECORD_VERSION = 28, `dispatch-control/src/lib.rs:586` LaneFile, `serve.rs:538` ticket_paths, `main.rs:896` print_lane_files, `scheduler.rs:10908` lane_files_by, `first_slice.rs:3809` and `at_finalize`, `design.md:1000`, `dispatch.md:2497`/`:3078`. All real.

## Departures caught
- No rerun-specific path; fix is in the shared subject lookup: caught. The plan replaces `review_subject` itself, used by every attempt, and does not add a rerun branch.
- Plan review carries nothing between attempts today: caught. It says `rerun_carries` says nothing for a plan review and never claims points carry.
- Revision is not lost on disk and the record already has `revisions`: caught. "What is lost is the record's way to reach them"; the selector reads `Attempt.revisions` and the files, no parallel state.
- Beyond the reference: a `Revision` entry is pushed before the send and does not prove the planner applied the note. The plan uses the planner's `plan.response-<round>.md` as the applied signal. Verified: the planner writes that file per the `respond_to_user` template. Also beyond the reference: the stored rerun question is not recomputed if the copy changes before the answer, and round files cannot be copied forward because the new attempt's round 1 would collide with `feedback-1`.

## Decisions to overturn
- Provenance: the plan explicitly does not record on the new attempt which copy it started from, so `show` can label prior copies but can never say what attempt 2 actually reviewed. The reference reuses `carried_from` (`ticket.rs:317`), already on the record and defaulted. I would reverse this; it is one field write at launch.
- Not reversed but noted: the plan requires "revised and applied" rather than the reference's "replaced attempt's copy whenever on disk". The reference accepts an "only if revised" restriction, and the unapplied case is equivalent to the plan output anyway, so this stands.
- Not reversed: `show`'s `others: Vec<OtherCopy>` lists every prior copy including the plan stage's. The reference wants one line for a differing prior revision. Heavier than needed, but not a redesign.

## Over-specification
- "The record" paragraph on how to bump `RECORD_VERSION` if one were needed, plus the unrelated `SCHEMA_VERSION`/`VIEWS_SCHEMA_VERSION` mention; "The signals: None"; the "Contract additions" section restating the UI section; the commit message; clippy and Linux-target steps that CLAUDE.md and the hook force. Roughly a fifth to a quarter of the plan.

## Hand-off
- Set `carried_from` on the new attempt to the attempt whose copy it took, and have `show` read it, so the reviewed copy's origin is on the record and not only in a stored question.

## Notes
- best: Found that a `Revision` entry does not mean the note was applied, and anchored the selection on the planner's response file, which the reference missed.
- worst: Refuses to record the rerun's source on the new attempt, leaving `show` unable to say which copy an attempt reviewed after the fact.