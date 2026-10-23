# Response to feedback, round 2

## Earlier points

1–3. Closed; no change. The `owner_revised` test noted under point 3 is gone with the cut under The UI.

## Cuts

**Where this departs from the issue**
- Third bullet (schema 19 vs 28): cut. The Record says "the value in `store.rs` at implementation time, plus one".
- Fourth bullet: cut.

**The record**
- "Why a field and not a derivation" paragraph: cut. Its one useful sentence is folded into the field's description.
- `holds_revision()`: one line, saying it only decides whether to offer `original` and whether to default to the carried copy. The "not proof" and Facts bullets are gone.
- `owner_revised` bullet: cut.
- Version-bump comment: now "migrate step, no transform, same pattern as 18 to 19".

**The signals**
- You asked to delete the section. It is now one line, "None new.", because the standing plan format requires every section heading to appear, with an empty one reduced to a line. The re-attach remark is gone.

**The actions and effects**
- Item 1: the slice and `rewrite_choices` bullet and the "whichever way" bullet are cut.
- Item 2: the `String` return-type, wording-justification and mid-edit visibility bullets are cut. The question text stays.
- Item 3: the "need no change" sentences are cut, so it says only what changes.
- Item 4: `source_session` is now "unchanged", and the missing-file fallback is one clause. The currency-check paragraph stays, because it settles point 1.
- Item 5: replaced by the single sentence "Don't change the `Complete`-only readers".

**The UI**
- `PathsView.plan_earlier`, `EarlierCopy`, `owner_revised` and the "plan, earlier" line: cut. `LaneFile.seeded_from` and the "started from review/1's copy" line remain. This also removes the feedback-file existence test and the `report::round_file` use.
- The Dispatch page plan-label paragraph and the between-failure-and-answer paragraph: cut.

**The tests**
- "No UI test" line, the `NO_ANSWER`/feedback-file `show` test and the `plan_earlier` snapshot: cut.
- The question-text test is merged into the first test. The redone-before case is merged into the redone-after test. The unrevised-failure case is one line. The store migration test is one line. `serve.rs` keeps only `seeded_from` and the `show` line.

**Docs and housekeeping**
- The README, design.md and no-client-names lines and the command list are dropped. The `docs/dispatch.md` version line and the CHANGELOG line stay. The "app runs at every step" line is cut.

**Follow-ups for the owner**
- You asked to delete the section. I kept two one-line bullets: the `rerun --note` drop, which you allowed, and the clone-source rule at `src/core/workflow.rs:289`. Item 4 cites it as the place a reviewer would get the note, so it needs a home. The "confirm against the issue's files" item is cut because it is already in Facts.

**Contract additions**
- You asked to delete the section. I shortened it to four lines and kept it, because the implementer copies it into the commit. `PathsView.plan_earlier` and `EarlierCopy` are gone with the UI cut.
