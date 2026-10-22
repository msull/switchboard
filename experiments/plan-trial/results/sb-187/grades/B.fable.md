| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| B | 3 | 3 | 3 | 2 | note |

## Grounding
- `dispatch/src/ticket.rs:347-350`: found (the `revisions` doc comment and field on `Attempt`, pushed before `workflow.object`, popped on refusal)
- `dispatch/src/scheduler.rs:7616-7618`: found (`revise` writes the owner's round file beside the reviewed copy via `round_file` and `atomic_write`)
- `dispatch/src/ticket.rs:1471-1474`: found (the `shown_where` doc: an open workflow attempt edits its copy in place until `finalize` is answered)
- `docs/dispatch.md:550-557`: found ("A review is a copy" bullet)
- `dispatch/src/scheduler.rs:10534-10548`: found (`review_subject`, filtering on `AttemptState::Complete`, the bug site)
- Beyond five: every other anchor I spot-checked also lands exactly, including `store.rs:26` (`RECORD_VERSION = 28`), the 27→28 migrate comment at `:608-612`, `rerun_carries` at `:9382`, the rerun question at `:7263-7266`, `ticket_paths` at `serve.rs:538`, the `files:` block at `main.rs:981-982`, the finalize question at `:7693-7696`, the six test helpers in `first_slice.rs`, `plan_label` in `src/ui/ticket.rs:425-433`, and the doc/design/README lines.

## Departures caught
- No rerun-specific path; fix belongs in the shared subject lookup: caught. The plan puts the whole change in `review_subject` and `start_workflow`, never a rerun branch.
- Plan review carries nothing between attempts today: caught ("`rerun_carries` adds nothing for a workflow stage").
- Revision is on disk and on the record already: caught, as the plan's first departure bullet (note on the decision, in `revisions`, and in the round file; only the copy is discarded). It does add one parallel field, which is a decision, not a miss.
- Extra, not in the reference: a restart that reruns the plan stage writes a newer plan than any revised copy, so the revised copy must only win when its attempt started after the writer's. Real and worth the `started_ms` guard.
- Extra: the chain case (attempt 2 started from attempt 1's revision, then fails), which the reference's "replaced attempt of the same stage and context" covers but does not name.

## Decisions to overturn
- None. The copy source matches reference decision 1 (restricted to revised copies, with a disk fallback and warning). Provenance is one new field, which the reference allows, though I would lean to reusing `carried_from` and skipping the version bump and migrate step; the plan's argument that `carried_from` means "points carry" for code reviews is defensible. The rerun question gets a Workflow arm in `rerun_carries` (decision 3). `show` gets extra lines (decision 4). The note is not re-sent and never routed through `rework` (decision 5). No app change (6). Tests, docs rows, and CHANGELOG match (7).

## Over-specification
- Cuttable: the new `docs/design.md` section with a dated heading, the `attempt-started` event text change, the `PriorCopy.by` field and the five-field wire struct where a path plus round would do, the "LaneFile is unchanged" and "Dispatch has no Effects" asides, the UI section, and the commands block the pre-commit hook forces. Roughly a fifth of the plan.

## Hand-off
- Reuse `carried_from` for provenance if the version bump feels heavy, and keep the `show` addition to one line per prior copy; everything else can be built as written.

## Notes
- best: every line reference lands, and it catches the restart case where a rerun plan stage outranks the revision.
- worst: a new record field with a version bump, a new wire struct, and a chain walk where the previous failed attempt's copy of the same stage and context is enough.