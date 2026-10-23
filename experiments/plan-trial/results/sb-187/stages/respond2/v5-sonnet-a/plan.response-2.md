# Response, round 2

Earlier points 1-4 are closed; nothing to do.

## Cuts

**Where this departs from the issue**
- Bullet 4 deleted here. Kept once, in the closing section, because the cut of "For the owner" would otherwise have left no named follow-up for it.
- Bullet 3: reduced to "the plan corrects the `revisions` doc comment (`ticket.rs:348`)" and the second statement in The actions and effects is removed.
- Bullet 1: now a one-line pointer to S.

**The record**
- The Rehydration paragraph is cut. The "Nothing is transformed..." sentences are cut.

**The signals**
- The `PathBuf` parenthetical is cut.
- U1 and O1 are now "pinned by test (c)" and "pinned by test (f)".
- The U2 check is stated only in that bullet. Docs now says only that the copy is not proven final.
- The whole-file bullet is one sentence of reasoning.

**The actions and effects**
- The opening is "The same `WorkflowStart`, with `plan_to_review().path` copied".
- The `:8407` parenthesis and the construction evidence are cut; the exactly-one-key guard stays.
- The snapshot paragraph and the separate doc-comment paragraph are cut.

**The UI**
- The older-readers sentence and the `grep planFiles` note are cut; "CLI `show` only" stays.
- The explanatory sentence in the `print_lane_files` bullet is cut.

**The tests**
- Item 4, unit (a), the item 3 parenthetical, and the `main.rs` conditional are cut. Unit (e) is folded into (b). Integration test 1 now says it covers `print_lane_files` through the `show` text. The `store.rs` test reads "a previous-version fixture test like the neighbours".

**Docs**
- The `README.md` bullet is cut. The U2 caveat is stated once in `design.md`. The CHANGELOG line is one sentence.

**One commit**
- Contract additions are the three names. The Internal line is dropped.

**For the owner**
- The section is replaced by "Follow-ups for the owner": one sentence for the planner-clone gap (it was only in the departure bullet and the old section, both of which you asked cut) and one for O1 and U2. The UI gap bullet is gone because design.md already lists it. If you would rather have the planner-clone follow-up dropped entirely, it is a one-sentence cut.
