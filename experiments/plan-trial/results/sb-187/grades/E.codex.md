| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| E | 3 | 2 | 1 | 1 | note |

## Grounding
- `dispatch/src/scheduler.rs:7436`: found (`start_workflow`, the shared review launch function).
- `scheduler.rs:10534`: found (`review_subject` selects the newest Complete attempt holding the artifact).
- `scheduler.rs:7761`: found (`NO_SUCH_RUN` fails the attempt with “review run gone”).
- `scheduler.rs:7823`: found (a paused run with `failed: true` fails the attempt with “review: <reason>”).
- `scheduler.rs:7525`: found (a failed workflow launch records “could not start the review”).

## Departures caught
- No separate rerun launch path: caught; changes the shared input lookup.
- Only code reviews carry points between attempts: missed; identifies Workflow’s empty rerun suffix but does not explain the distinction or explicitly preserve fresh rounds and points.
- Revision files and recorded owner input already survive: caught; reuses the copy and existing revisions.
- Additional: identifies cancelled attempts and other workflow failures as affected by the same selection rule.

## Decisions to overturn
- Replace the proposed full `plan copies` inventory with the reference’s single additional prior-revision line; the inventory expands the display and contract beyond the requested distinction.
- Drop the ticket-page Plan tab change and associated UI tests/design documentation; these conflict with reference decision 6, which requires no app-side change. The single new provenance field is acceptable under decision 2.

## Over-specification
- Roughly one-third is unnecessary: the expanded copy-view contract, app UI work, empty signals/actions sections, repeated contract summary, unrelated schema assurances, README checks, and prescribed commit message.

## Hand-off
- Narrow this to shared input selection, provenance, rerun wording, and one prior-revision files line; explicitly retain fresh workflow rounds and points.

## Notes
- best: Correctly locates the shared selection bug and protects against an older review copy shadowing a newer plan.
- worst: Expands a targeted recovery fix into a copy inventory and app UI change.