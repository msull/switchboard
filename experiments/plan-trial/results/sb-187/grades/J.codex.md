| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| J | 3 | 3 | 0 | 1 | note |

## Grounding
- `dispatch/src/scheduler.rs:10534`: found (`review_subject` selects the newest Complete attempt holding the artifact and its session).
- `dispatch/src/ticket.rs:1476` `shown_where`: found (includes Complete attempts and open Workflow attempts, excluding forgotten artifacts).
- `dispatch/src/serve.rs:539`: found (`ticket_paths` builds displayed plan paths and review metadata).
- `scheduler.rs:9382`: found (`rerun_carries` describes code-review point carryover and resolution reviews; Workflow gets no sentence).
- `dispatch/src/main.rs:1176`: found (`print_answered` prints answered decisions, including their notes).

## Departures caught
- No separate review rerun launch path: caught; changes the shared `review_subject` lookup.
- Plan reviews do not carry points between attempts: caught; distinguishes code-review carryover and preserves fresh workflow rounds.
- Revision survives on disk and in the record: caught; uses existing `revisions` and the revised artifact.
- Additional finding: answered finalize notes remain visible through `print_answered`; supported by the code.
- Additional finding: revision metadata precedes the send, so requiring a response file would be an incorrect gate. However, the issue does not establish the plan’s claim that the repro died before the planner answered.

## Decisions to overturn
- Constrain revised-source selection to the same stage and context, with an on-disk check and fallback. The proposed artifact-wide search lacks these reference-decision-1 safeguards. The revision-only gate itself is explicitly acceptable.
- Record provenance using `carried_from` or one new field, as reference decision 2 requires; the plan records neither.
- Put copy wording in the shared rerun-question helper, as reference decision 3 specifies. Updating only `rerun_ask` misses the initial failure question, which separately calls `rerun_carries`.
- Include the requested `docs/dispatch.md` rerun and revise documentation from reference decision 7; design-document edits do not replace it.

## Over-specification
- Roughly one-third is removable: empty signals/actions declarations, repeated schema assurances, exact prose substitutions, commit wording, unrelated constant corrections, and detailed tests of unchanged round attribution. The Plan-tab expansion also exceeds the reference’s small `files:` change.

## Hand-off
- Keep the core approach, but scope and validate source selection, record provenance, update both rerun-question paths, and add the requested dispatch documentation.

## Notes
- best: Preserves revised content without copying attempt-local revision round numbers.
- worst: Names the copy only in the reconstructed rerun question, missing the initial failure question.