| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| v5lean-sonnet-c | 3 | 2 | 0 | 2 | no |

## Grounding
- `facts, Unresolved: "Why the app answered no such run"`: found (supplied facts, lines 29–30, explicitly leave the disappearance unexplained).
- `scheduler.rs:7195-7230`: found (`fail_attempt_with` sets failure state, timing and flags without clearing artifacts or revisions).
- `facts Settled`: found (supplied facts, particularly line 13, describe the preserved failed-attempt record).
- `ticket.rs:1476-1491`: found (`shown_where` accepts completed attempts and open workflows, excluding failed workflows).
- `scheduler.rs:10534-10548`: found (`review_subject` selects a completed attempt’s artifact and session, skipping failed attempts).

## Departures caught
- No separate rerun launch path: caught; changes the shared `review_subject` lookup.
- Plan reviews carry no rounds or points; code reviews do: missed; identifies the empty Workflow rerun wording but never explains this distinction.
- Revision survives on disk and in the record: caught; uses existing artifacts and acknowledges `revisions`.
- Additional finding: before rerun, `show` falls back to the completed plan output; supported by `shown_where`.
- Additional claim: the restart explanation is unsupported. The facts establish uncertainty about the cause, not evidence contradicting the reported reproduction.

## Decisions to overturn
- Selecting failed workflows globally by subject conflicts with reference decision 1: prefer the replaced attempt of the same stage and context, then the plan output.
- Explicitly making no record change conflicts with decision 2: record provenance using `carried_from` or one new field.
- Naming only the path in the rerun question omits decision 3’s identification of the owner’s round-N revision.
- Replacing the single displayed plan selection conflicts with decision 4: show the reviewed copy and a differing prior revision together. Once attempt 2 opens, the proposed lookup still hides attempt 1.

## Over-specification
- Roughly 15% is expendable: the prescribed commit message, repeated contract assurances, unrelated known-gaps discussion, and some restatement of unchanged plumbing. The chained-failure test is useful.

## Hand-off
- No: source selection needs stage/context boundaries, and provenance plus the required two-copy display remain unplanned.

## Notes
- best: Separates the reviewed file from its planner session and tests chained failures.
- worst: Treats changing `shown_where`’s winner as satisfying the requirement to display both copies.