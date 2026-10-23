| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| v5lean-sonnet-b | 3 | 3 | 2 | 2 | note |

## Grounding
- `dispatch/src/scheduler.rs:7616-7642` (revise writes the note, deletes only on refusal): found (`round_file` + `atomic_write` at 7616-7618, `revisions.push` at 7621, `remove_file` on a non-`NO_ANSWER` `Reply::Failed` at 7642)
- `scheduler.rs:10534-10548` review_subject, Complete-only filter: found (exact span, `.filter(|a| a.state == AttemptState::Complete)` at 7539)
- `:7465` the call site in start_workflow, copy at `:7458-7471`: found (`review_subject` at 7465, `fs::copy` at 7471; the 7458 end is the subject check, slightly loose but the copy span is right)
- `:9363-9368` rerun_ask's `Cancelled` arm: found (plain `Cancelled` arm at 9363-9365, same `["rerun","park"]` set, then shared question text)
- `dispatch/src/ticket.rs:350` revisions on Attempt: found (`pub revisions: Vec<Revision>` at line 350, doc comment at 346-348)
- Spot-checked beyond five: `acted_rerun` ticket.rs:930, `may_rerun` 10665, `latest_attempt` 9444, `fail_attempt` 7148 (plan said 7157, inside the fn), `ANSWERS` pipeline.rs:111, `print_lane_files` main.rs:896-907, `RECORD_VERSION` store.rs:26 all real.

## Departures caught
- No rerun-specific path; fix sits in the shared subject lookup: caught (plan puts it in `start_workflow` ahead of the `review_subject` copy and explicitly covers park-then-resume through the same predicate).
- Plan reviews carry nothing between attempts today: caught implicitly (plan states `rerun_carries` has text only for `AttemptKind::Review` and adds a Workflow arm; never claims points carry).
- Revision not lost on disk, record already knows via `revisions`: caught (uses `revisions` plus the file on disk; the note is noted as already durable, so nothing is copied onto the record).
- Extra beyond the reference: park then resume of a revised open review hits the same path through the `Cancelled` arm. Also flags the half-written-copy risk after a mid-edit kill and surfaces it in the question text rather than copying silently.

## Decisions to overturn
- Adding a new `original` rerun answer word, touching `acted_rerun`, both `("rerun","rerun")` matches, `ANSWERS`, `decision_words`, and a supervisor/pipeline test. Reference decision 1 carries the previous attempt's copy with a fallback and needs no new answer; the issue asks only that the question say which copy. I would cut it.
- Lesser note, not counted: `show` annotates the reviewed-copy line with "started from review/1's copy" instead of listing the failed attempt's `plan.md` as a prior-revision line (reference decision 4, and the issue's "which is a prior revision"). Either satisfies "say which"; the "started from stage plan" line on every open plan review is noise I'd drop.
- `seeded_from` as a new field rather than reusing `carried_from`: acceptable per reference (one new field is fine).

## Over-specification
- The `original` answer machinery and its tests (~15%), the currency check `subject_unchanged_since` paragraph with its double evaluation (~5%, defensible edge case but more than the issue needs), "The signals: None new", and the Contract additions restating The record (~5%). Roughly a fifth to a quarter could go.

## Hand-off
- Drop the `original` answer and carry the previous attempt's copy by default; in `show`, add one line for the failed attempt's `plan.md` as the prior revision instead of annotating the reviewed copy.

## Notes
- best: Correctly locates the one `Complete` filter as the bug, extends the fix to park-then-resume, and names the half-written-copy risk in the rerun question.
- worst: Widens the decision contract with a new `original` answer across pipeline, supervisor and ticket for a choice the issue never asked for.