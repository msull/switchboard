Everything the plan cites exists. I have what I need to score.

```
| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| D | 3 | 3 | 3 | 2 | yes |
```

## Grounding
- `docs/design.md:1002`: found (the "Plan review workflow" section opens with "A workflow is a durable record that drives other records")
- `dispatch/src/scheduler.rs:7755`: found (`poll_workflow`'s `Reply::Failed { reason } if reason == NO_SUCH_RUN` arm calling `fail_attempt` with "review run gone")
- `src/app.rs:3915-3918`: found (`Body::Workflow` handler returning `failed("no such run")` when `core.run_view` is `None`)
- `src/core/workflow.rs:1009`: found (`fn remove_workflow`)
- `dispatch/tests/first_slice.rs:3809`: found (`a_run_switchboard_no_longer_has_fails_the_attempt`, which does `env.sb().runs.clear()`)

Also checked beyond the five: `review_subject` at 10534 with the `Complete` filter, `start_workflow` at 7436 with the copy at 7471 and push at 7487, `rerun_carries` at 9382 with sole caller `rerun_ask`, `review_done`'s finalize question at 7693-7696, `DISCARDED_BY` import at line 25, `carried_from` at `ticket.rs:312-317`, `review.rs:3618-3625`/`3646`/`225`, `report.rs:156`/`201`/`413`, `serve.rs:538-548`/`1540`, `main.rs:896-909`, `LaneFile` at `lib.rs:586`, `RECORD_VERSION = 28` at `store.rs:26`, `docs/dispatch.md:550`/`2975`/`3171`, README lines 448/459. All correct. Runs are persisted in the workspace record, so the plan's claim that a runner restart alone does not produce `no such run` holds.

## Departures caught
- No rerun-specific path; the subject lookup shared by every attempt is the site: caught. The plan names `review_subject`'s `Complete` filter as the cause and hooks `carried_copy` into `start_workflow`, which every attempt passes through; a first attempt has no `latest_attempt` so it is unaffected.
- The plan review carries nothing between attempts today: caught. It states `rerun_carries` adds nothing for a workflow attempt and audits every `carried_from` reader to show they all filter `AttemptKind::Review`.
- The revision is already on disk and on the record: caught. It uses `prev.revisions`, the attempt directory's copy, and the existing feedback file; no parallel state.
- Extra, not in the reference: the repro's "runner restart mid-run" cannot produce `review run gone` because runs are durable in the app; the tests use `runs.clear()`. Verified. Also that the owner's note itself is not lost, only the revised copy, which matches reference decision 5.

## Decisions to overturn
- None. Copy source restricted to revised attempts (reference says fine), `carried_from` reused with widened doc, a Workflow arm in `rerun_carries`, one extra `show` line via a new `LaneFile::started_from` wire field, note named rather than replayed and never routed through `rework`, no app change, integration plus unit tests, docs and changelog. The reference's `docs/dispatch.md` rerun section (~318-350) is not touched, but the row in the first-slice table and design choice 6 cover it.

## Over-specification
- The `DISCARDED_BY` cancellation guard duplicates the "input ended after prev started" guard, and that second guard forces `review_subject` to grow a third return value. The finalize-question clause and the `unanswered_revision` helper with its sentence in two questions are beyond the issue's ask. The schema-bump paragraph says nothing will change and then explains how to change it anyway. Roughly a fifth of the plan.

## Hand-off
- Yes.

## Notes
- best: Audits every existing reader of `carried_from` by line so reusing the field on a Workflow attempt is shown safe, not assumed.
- worst: Two overlapping staleness guards and an `ended_ms` plumbing change through `review_subject` for a case the restart path already cancels.