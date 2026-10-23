## Earlier points

1. `Failed` and `Cancelled` both qualify, with no support for `Cancelled`: closed. The planner cited the four `Cancelled` writes (`scheduler.rs:2872`, `2914`, `4962`, `5585`), which I did not have. The plan now qualifies `Failed` only and tests the `SENT_BACK_FROM` case.

## Cuts

**Where this departs from the issue**
- The `Cancelled` rationale is written out here, again in step 1 Path, and again in the tests. Keep it in Path only and drop the line list from this section. A one-clause pointer is enough here.
- The "Step 2 of the repro" bullet says only that the plan does not depend on the cause. Step 3 and Follow-ups say the same. Cut the bullet.
- The `show` bullet repeats the reason in "The UI". Keep one.

**The record**
- This is the largest cut. `started_from` is the second mechanism. The rerun question (step 2) already names the copy by calling `review_subject`. The issue's "say which copy" is met by the question alone. `started_from` brings in:
  - the field and the `RECORD_VERSION` bump with a `migrate` step;
  - `LaneFile.started_from`, the `ticket_paths` change and the `print_lane_files` line;
  - the store, serve and control tests.
  
  That is built in front of the default. Cut all of it, or defer it as a follow-up. If `started_from` goes, `Subject.from` goes too. `review_subject` can return the existing tuple, or a pair plus the chosen path.
- If the field stays, "Why a field and not a recomputation" is the only part worth keeping. Drop the "attempts already running keep None" sentence.

**The signals**
- "None new" is a section with nothing to say. Cut it. The "app is not changed" sentence already appears under Docs.

**The actions and effects**
- The opening "No new AppAction, Effect, port or adapter" repeats the Commit section. Cut it.
- The paragraph that explains why the tuple became a struct is only needed if `started_from` stays. Otherwise cut it.
- The "Chained failures work" bullet follows from the newest-failed rule. Cut it.
- Step 3 ("The default does not depend on the unresolved facts") restates `facts.md` U1, U2 and O1, and says "nothing is built behind the default". Cut it.
- In the Path bullet, drop the sentence that widens `Cancelled` to other reasons as a follow-up. It is speculation.

**The UI**
- If `started_from` is cut, the whole section goes. Otherwise cut the parenthetical about the Plan tab ignoring unknown fields, which is serde behaviour already settled in the facts file.

**The tests**
- The optional `Reply::Failed { NO_SUCH_RUN }` test exercises the cause, which the plan says it does not depend on. The seeded gate already covers the behaviour. Cut it and its "say so in the commit" fallback.
- The store, serve and control tests go with `started_from`.
- The closing `cargo fmt`, `clippy` and `test` line repeats CLAUDE.md. Cut it.
- "Read the fake first" is already in `facts.md`. Cut it.

**Docs and changelog**
- Cut the README bullet ("no change") and the app/`SWITCHBOARD_SCRIPT` bullet. Both are bullets with nothing to say. The `Ticket::input` sentence is already pinned by an existing test.
- Drop "the record version" and `started_from` from the design.md status line if the field is cut.

**Follow-ups**
- Cut the Plan tab bullet ("the tab was not read"). It is not a task.
- Cut the planner-clone bullet. It is a separate decision, as the plan itself says.
- O1 repeats `facts.md` and says "changes nothing here". Cut it.

**Commit**
- The "Contract additions" list restates the Record section. If the field stays, keep only the commit title and the three lines. Otherwise reduce it to the title.
