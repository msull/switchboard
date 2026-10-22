| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| B | 3 | 2 | 1 | 1 | note |

## Grounding
- `dispatch/src/ticket.rs:347-350`: found (persisted `revisions`, pushed before `workflow.object`).
- `dispatch/src/scheduler.rs:7616-7618`: found (writes the owner's note to the subject's round file).
- `dispatch/src/ticket.rs:1471-1474`: found (documents that an open workflow edits its copy in place).
- `docs/dispatch.md:550-557`: found (describes review copies and instructing the planner to edit the copy).
- `dispatch/src/scheduler.rs:10534-10548`: found (`review_subject` selects the newest complete attempt holding the subject and requires its session).

## Departures caught
- No rerun-specific launch path: caught; changes the shared `review_subject` lookup and `start_workflow`.
- Only code reviews currently carry points between attempts: missed explicitly; observing that workflow rerun questions lack a sentence does not establish the point-carrying distinction.
- Revision files and revision metadata already survive: caught; uses existing `revisions` and the edited copy.
- Additional finding: repeated failures require preserving revision ancestry. Useful extension, though not a contradiction of the issue.
- Additional claimed departure: a newer plan writer must supersede the revised copy. Existing lookup establishes complete-writer selection, not this proposed precedence rule.

## Decisions to overturn
- Remove the mandatory `started_ms` precedence gate: reference decision 1 prefers the replaced same-stage/context attempt's available copy, with the plan output as fallback.
- Replace unconditional ancestry listing with the relevant distinct prior revision: reference decision 4 calls for one extra `files:` line, whereas this plan prints every ancestor, including unchanged chained copies. Its ancestry-only lookup also omits prior revisions without the new provenance link.

## Over-specification
- Roughly one-third is unnecessary prescription: full contract declarations repeated at the end, exact event wording and event assertions, a design-history section, unchanged-file commentary, and detailed test mechanics. The ancestry display also exceeds the requested scope. One new provenance field is explicitly acceptable under the reference.

## Hand-off
- Align source precedence and prior-copy display with the reference, and explicitly document that workflow reruns start fresh rounds without carrying points.

## Notes
- best: Grounds the failure in the shared subject lookup and tests revised content surviving consecutive failures.
- worst: Adds an unsupported source-precedence rule and expands a small provenance display into an ancestry listing.