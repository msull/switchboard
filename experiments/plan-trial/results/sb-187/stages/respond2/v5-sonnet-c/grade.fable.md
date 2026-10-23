Checked: the attempt is marked Failed before the rerun question is built, so the plan's rerun-question lookup works. All references resolved. Writing the score.

```
| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| v5-sonnet-c | 3 | 3 | 1 | 2 | note |
```

## Grounding
- `dispatch/src/review.rs` (the code review, not the workflow stage): found (file exists; plan review is `AttemptKind::Workflow` in scheduler.rs as stated)
- `scheduler.rs:7611-7612`, `OWNER_ROUND_HEADING`: found, a few lines off (the `round_file` / `OWNER_ROUND_HEADING` write is at 7616-7618 inside the same `revise` body; `revisions.push` follows at 7620-7625)
- `review_subject` (`scheduler.rs:10534`): found exactly; filters `AttemptState::Complete`, returns path and session from one attempt, as the plan says
- `docs/design.md:2215` "a review edits its copy in place under one path": found exactly
- `Attempt.revisions` (`ticket.rs:350`): found (349-350, `#[serde(default)] pub revisions: Vec<Revision>`)
- Also checked: `ticket.rs:1476` shown_where, `:7465` call site, `:9382` rerun_carries, `:7266` / `:9373` callers, `:7692-7695` finalize text, `:7777-7781` session=face, the four `Cancelled` writes, `:8979` SENT_BACK_FROM, `:10665` may_rerun, design.md 1782-1786 and 1000. All real.

## Departures caught
- No rerun-specific path; fix is in the shared subject lookup: caught (plan changes `review_subject`, notes it has one caller, no rerun-only branch)
- Plan review carries nothing between attempts: caught (plan says `rerun_carries` is empty for a Workflow attempt and adds an arm rather than claiming carry)
- Revision is not lost on disk or the record: caught (plan locates the note in the decision and `plan.feedback-N.md`, the text in the failed attempt's copy, and reads `revisions`)
- Extra, not in the reference: the rule is wider than post-`revise`, since any failed attempt's copy is newer than the plan stage's (consistent with reference decision 1's "whether or not revised"); and a failed `review/1` must not beat a `plan/2` written after a send-back, hence the position test. The Cancelled exclusion is justified from all four write sites.

## Decisions to overturn
- Leaving `show` alone. Reference decision 4 (and the issue's "Where to look" item 2) wants one extra `files:` line naming the prior revision when an earlier attempt's copy differs. The plan's reason (show lists only Complete/open attempts) describes the current behaviour it was asked to change.
- No provenance on the record, deferred to a follow-up `started_from` field. Reference decision 2 reuses `carried_from`; the plan's only durable trace is the path quoted in the rerun question text. Softer than the first, but I would add it before implementing so `show` has something to read.
- Not counted: docs go to `design.md` instead of `docs/dispatch.md` (rerun around 318-350, the `revise` row), a hand-off note rather than a reversal. Source-of-copy rule, Cancelled exclusion, `exists` closure for testability, and the rerun question wording all match or reasonably extend the reference.

## Over-specification
- Small: the "Follow-ups for the owner" section, the "Contract additions: None" commit boilerplate, and the four-citation Cancelled argument could be trimmed. The owner-facing phrase "the planner may have been mid-edit" is debatable but not excess. Roughly 10-15% cuttable.

## Hand-off
- note: also add the `show` `files:` line for a prior revision that differs from the reviewed copy, and put the doc change in `docs/dispatch.md`'s rerun and `revise` text, not only `design.md`.

## Notes
- best: the position test (failed attempt must sit after the Complete writer) plus the Cancelled exclusion, each tied to a concrete send-back scenario, so the fix cannot regress a `plan/2` rerun.
- worst: drops the issue's explicit `show` request with a circular justification and leaves no provenance on the record.