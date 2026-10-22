| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| F | 3 | 3 | 2 | 1 | note |

## Grounding
- `dispatch/src/ticket.rs:1476`: found (`shown_where` includes complete attempts and open Workflow attempts, excluding failed copies).
- `dispatch/src/serve.rs:504`: found (`plan_rounds` reads feedback files and revision authors; its caller selects the newest Workflow attempt with artifacts).
- `dispatch/src/scheduler.rs:10534`: found (`review_subject` selects the newest complete attempt holding the subject and requires its session).
- `dispatch/src/scheduler.rs:7436`: found (`start_workflow` copies that subject into the new attempt and sends `WorkflowStart`).
- `dispatch/src/scheduler.rs:7565`: found (`revise` writes feedback, records `Revision`, and sends `WorkflowObject`).

## Departures caught
- No distinct rerun launch path: caught; changes the shared subject lookup.
- Plan reviews do not carry points between attempts: caught; distinguishes code-review point provenance from plan-copy provenance.
- Revision survives on disk and in the record: caught; uses existing `revisions` and the retained copy without resending the note.
- Additional finding: the newest-attempt selection also hides earlier owner feedback rounds from `show`; confirmed by `ticket_paths`.

## Decisions to overturn
- Replace the proposed all-attempt history and nested owner-round display with the reference’s single additional prior-revision file line, shown when the earlier copy differs. The broader UI and wire expansion exceed decision 4.
- The revision-only source policy and single new provenance field are explicitly acceptable alternatives in the reference; neither counts as an overturn.

## Over-specification
- Roughly one-third is removable: the expanded history UI, event embellishment, overlapping unit/integration assertions, conditional README/design instructions, commit message, and repeated contract inventory. The migration detail is justified by choosing a persisted field.

## Hand-off
- Keep the source-selection and repeated-failure handling, but reduce `show` to the reviewed copy plus one relevant prior-revision line.

## Notes
- best: Handles a second failure without losing the revision inherited by the intervening attempt.
- worst: Expands a focused file-provenance fix into a broader attempt-history presentation.