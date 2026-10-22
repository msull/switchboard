| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| A | 3 | 3 | 0 | 1 | no |

## Grounding
- `dispatch/src/scheduler.rs:7436`: found (shared `start_workflow` launch function; obtains its input through `review_subject`).
- `dispatch/src/scheduler.rs:10534`: found (`review_subject` selects the latest Complete attempt with the requested artifact and session).
- `docs/dispatch.md`, "A review is a copy": found (documents the attempt-local copy, prompts targeting it, and preservation of the original).
- `dispatch/src/scheduler.rs:7616`: found (writes the owner’s feedback, records a revision, and sends `WorkflowObject`).
- `dispatch/src/ticket.rs:350`: found (`revisions: Vec<Revision>` records owner-opened plan review rounds).

## Departures caught
- No separate rerun launch path: caught; places selection beside `review_subject` and uses it from the shared launch.
- Only code reviews currently carry points: caught; distinguishes existing code-review carry from carrying a workflow’s edited document.
- Revision survives on disk and in the record: caught; reuses the copy, `revisions`, and `carried_from`.
- Additional finding: reviewer-driven edits also survive in the failed copy, so preserving unrevised copies is useful too.

## Decisions to overturn
- The rerun question must identify the owner’s round-N revision when present, as reference decision 3 requires. The proposed normal wording mentions only “rounds so far”; its owner-round wording covers only a missing response.
- `show` must expose a differing prior revision even when it was not selected as the source, per reference decision 4. Deriving the extra line solely from `carried_from` hides that revision on fallback or the proposed “start over” path.
- Do not give code-review events the “from #1’s copy” suffix. The reference explicitly distinguishes their carried points from workflow document copies; this wording misrepresents existing behavior.

## Over-specification
- Roughly one third is unnecessary scope or prescription: the new “start over” behavior and tests, event changes, ticket-page label, README layout edit, design section, exact helper signature, and commit-message contract boilerplate. Some defensive selection checks are useful, but the core fix needs substantially less machinery.

## Hand-off
- no: the copy-preservation approach is sound, but the required provenance signals remain incomplete and the event change introduces misleading behavior.

## Notes
- best: Correctly preserves the edited document while retaining the upstream planner session and reusing existing record state.
- worst: Adds peripheral behavior while missing the reference’s explicit owner-revision wording and prior-revision visibility requirements.