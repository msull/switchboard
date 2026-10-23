## Earlier points

1. Plan redone between the failure and the rerun: closed. The `subject_unchanged_since` check in `start_workflow` and `rerun_options` settles it. The tests cover both orderings.
2. `original` not threaded through the supervisor seed: closed. `decision_words("rerun")` is changed, and the `decides` loops are shown to reject `original`.
3. "The copy you revised" on `revisions` alone: closed. The question wording is now "the copy a revise was sent to". (See the cut under The UI: the `owner_revised` file test it introduced is not needed.)

## Cuts

**Where this departs from the issue**
- Drop the third bullet (schema version 19 vs 28). It is a correction to the investigation, and an implementer reads `RECORD_VERSION` from `store.rs` anyway.
- Drop the fourth bullet. It restates the UI section.

**The record**
- Drop the "Why a field and not a derivation" paragraph. The field is the decision, and the reason is already in the first paragraph.
- Cut the `holds_revision()` bullets to one line: it only decides whether to offer `original` and whether to default to the carried copy. The "not proof the reviser finished" and "Facts, For the owner" bullets restate Facts.
- Drop the `owner_revised` bullet (see The UI).
- Drop the version-bump comment text beyond "migrate step, no transform, same pattern as 18 to 19".

**The signals**
- Delete the section. It says "None new", and the re-attach remark duplicates Facts, Unresolved.

**The actions and effects**
- Item 1: drop the bullet on `&'static [&'static str]` slices and `rewrite_choices`. The compiler and the existing code force it. Drop the last bullet (the "works whichever way the owner answers" justification), since `rerun` default plus `original` is the design as stated.
- Item 2: drop the bullet explaining why `rerun_carries` changes its return type to `String`. The compiler forces that. Drop the two bullets that re-argue Facts (the wording follows `scheduler.rs:11285-11313`, and the mid-edit visibility). Keep the question text itself.
- Item 3: drop the sentences saying the `decides` loops, the `supervisor.rs:1274` assertion and the `:10674` check "need no change". Say only what changes.
- Item 4: shorten the `source_session` paragraph to "unchanged". The reasoning about the app clone is in Facts, and the follow-up is below. Fold the missing-file fallback into one clause.
- Item 5 ("Readers left alone"): delete. A list of code left untouched does not tell an implementer what to do. If one warning is wanted, keep "don't change the `Complete`-only readers" as a single sentence.

**The UI**
- Drop `PathsView.plan_earlier`, `EarlierCopy`, `owner_revised` and the "plan, earlier" line. They are an enhancement in front of the default: the question already names the copy before the answer. `LaneFile.seeded_from` plus the "started from review/1's copy" line is enough to meet the issue's "say in `show` which copy it will review". That also removes the feedback-file existence test and `report::round_file` use from the plan.
- Drop the paragraph on the Dispatch page's plan label. It says nothing is done there.
- Drop the paragraph on what `show` names between the failure and the answer.

**The tests**
- Drop "No UI test: `tests/ui.rs` is untouched".
- Drop the `NO_ANSWER` / feedback-file `show` tag test (goes with `owner_revised`).
- Merge "The question text names both paths..." into the first test. Merge the "redone before the question" case into the redone-after test as a second assertion, and keep the unrevised-failure case as one line.
- Drop the `serve.rs` snapshot of `plan_earlier`. Keep `seeded_from` in the `show` line check only.
- Store migration test: the existing migration test pattern forces it. Keep one line.

**Docs and housekeeping**
- Drop the README, design.md and "no client names" lines (CLAUDE.md already says them), and the list of commands to run (also in CLAUDE.md). Keep the `docs/dispatch.md` version line and the CHANGELOG line.
- Drop "The app runs at every step: the field defaults...". It is true by construction.

**Follow-ups for the owner**
- Delete the section. It is out-of-scope notes, and the "confirm against the issue's files" item is already in Facts, For the owner. Keep, at most, the one-line `rerun --note` remark if the implementer should know it is dropped.

**Contract additions**
- Delete the section. It restates The record, The actions and The UI.
