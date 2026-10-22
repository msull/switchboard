All references I checked are real, including the secondary ones (rerun_carries at 9382, the call sites at 7266 and 8294, LaneFile at dispatch-control lib.rs:586, ticket_paths at serve.rs:538, print_lane_files at main.rs:896, RECORD_VERSION 28 at store.rs:26, the revisions push at 7621 before the send, the docs rows at 550 and 2975).

```
| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| H | 3 | 3 | 2 | 2 | note |
```

## Grounding
- `workflow_stage` (scheduler.rs:7379): found (fn `workflow_stage` at 7379, under the `// --- workflow stages` banner)
- `start_workflow` (7436): found (fn at 7436, `review_subject` called at 7465 as the plan says)
- `review_subject` (10534): found (filters `AttemptState::Complete`, returns path and session; exactly the bug site)
- `revise` (7565): found (fn `revise` at 7565; the `revisions.push` is at 7621, before the send, as the plan's gap bullet claims)
- `Revision` on the failed attempt (`ticket.rs:350`): found (`pub revisions: Vec<Revision>` at 350, doc says "pushed before `workflow.object` is sent and popped when Switchboard refuses it")

## Departures caught
- No rerun-specific path; fix is in the shared subject lookup: caught, with the right fix (`review_subject`, one caller at 7465).
- Plan review carries nothing between attempts today: caught (`rerun_carries` returns "" for a Workflow attempt; the plan adds the arm rather than claiming points already carry).
- Revision not lost on disk or on the record; use it, don't add parallel state: caught ("it already does... what is lost is their use"; no record change, no `RECORD_VERSION` bump).
- Extra, not in the reference: `revise` pushes the `Revision` before the send and keeps it on a lost send, so a failed attempt with a revision may hold a copy the planner never edited. The plan documents this honestly and pins it with a test instead of pretending the copy always holds the note. Also notes the planner clone still forks from the Complete attempt's session, which the reference agrees with implicitly.

## Decisions to overturn
- Threading an `exists` predicate through `review_subject`, `rerun_carries` and `rerun_ask` so the rerun question can hedge with "If `<path>` is still there...". The reference's decision 3 is a plain sentence naming the previous attempt's copy; the file-existence check belongs in `start_workflow` (where the plan already logs the fallback), not in the question text or its signature. I would reverse this one.
- Not recording provenance on the new attempt (reference decision 2 reuses `carried_from`). The plan derives it from the ticket on every call, which works and is deterministic, so I do not count it as an overturn, but note that after `review/2` completes nothing on the record says which copy it reviewed.

## Over-specification
- The template sections "The record", "The signals", "The actions and effects" each say only "no change"; the "Contract additions" block restates earlier bullets; the commit message and the README line-459 note add nothing. The `prior_lane_files` bullet and the test list are longer than an implementer needs (the serve.rs test enumeration lists seven cases where three would pin the behaviour). Roughly a fifth to a quarter of the plan.

## Hand-off
- Drop the `exists` predicate from the question path: have the rerun question name the previous attempt's copy unconditionally, and keep the missing-file fallback and warn in `start_workflow` only.

## Notes
- best: Every line number checked out, and it found the lost-send gap (a revised failed attempt whose planner never applied the round) that the reference missed, then scoped it honestly rather than widening `WorkflowStart`.
- worst: The file-existence predicate threaded into `rerun_ask`/`rerun_carries` and the hedged question text it produces is complexity the fix does not need.