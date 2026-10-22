| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| K | 1 | 2 | 0 | 1 | no |

## Grounding
- `review/1/joined/plan.md`: wrong (absent from the tree; an illustrative runtime artifact).
- `dispatch/src/scheduler.rs:7565`: found (`revise` writes feedback, records a revision, and sends `WorkflowObject`).
- `plan.feedback-1.md`: wrong (absent from the tree; the code generates round-specific feedback files at runtime).
- `plan/1/joined/plan.md`: wrong (absent from the tree; another illustrative runtime artifact).
- `scheduler.rs:10534`: found (under `dispatch/src/`; `review_subject` selects the newest Complete attempt’s artifact and session).

## Departures caught
- No separate rerun launch path: caught; changes the shared `review_subject` lookup.
- Only code reviews carry points between attempts: missed; leaves `rerun_carries` unchanged without explaining this distinction.
- Revision survives on disk and in existing records: caught; uses existing `revisions` and artifacts.
- Additional finding: recorded revision intent does not prove the planner finished applying the note; correctly avoids promising otherwise.
- Grounding counts the first five literal references, including runtime examples; their absence does not refute the described runtime behavior.

## Decisions to overturn
- Replace the unrestricted artifact lookup with selection scoped to the replaced attempt’s stage and context, checking that its copy exists and falling back when absent (reference decision 1). Restricting eligibility to owner revisions is acceptable.
- Reverse “No change” to record provenance: populate `carried_from` and widen its documentation (reference decision 2). The two transient display fields are not two new provenance fields.
- Amend `docs/dispatch.md` at its rerun and revise guidance; the proposed design and agent-guide changes omit the reference’s required user documentation (reference decision 7).

## Over-specification
- Roughly one-third is expendable: empty signals/actions sections, hypothetical migration instructions, README non-changes, prescribed helper signatures and literal formatting branches, repeated contract additions, and a commit message.

## Hand-off
- no: source selection lacks stage/context and disk-existence safeguards, provenance is omitted, and required user documentation is missing.

## Notes
- best: Distinguishes recorded owner intent from a revision actually applied before failure.
- worst: Specifies presentation details extensively while leaving the source-selection contract incomplete.