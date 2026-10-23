# Feedback, round 2

## Earlier points

1. Owner-only claim: closed. The claim is dropped and the wording now matches what the tree shows.
2. "Response file exists" is not "finished": closed. The plan now says "answered", names the ordering gap with the two code references, and falls back as before.
3. Slow planner and never-answered look the same: closed. One wording, "no response file was found", is used in every place.
4. `rerun_carries` subject: closed. The single `new_attempt(.., Workflow, ..)` call site with one key is evidence I did not have, and the exactly-one-key guard covers the rest.

## Cuts

**Where this departs from the issue**
- Bullet 4 (rerun's planner is still a clone of the original) is repeated under "For the owner". Keep one copy; delete it here.
- Bullet 3: drop the last two sentences' detail on the `revisions` doc comment. "The plan corrects the comment" is enough. The same fix is stated again at the end of "The actions and effects"; keep it in one place.
- Bullet 1: the evidence chain (`fail_attempt` touches no file, the app deletes none) is already in facts S. A one-line pointer is enough.

**The record**
- The "Rehydration" paragraph restates the previous paragraph ("a decision made once") and `workflow_stage` re-reading the ticket. Cut it.
- "Nothing is transformed. The store already refuses to write a newer version." The `migrate` convention and the existing tests force this. Cut.

**The signals**
- The parenthetical on owned `PathBuf`s and the borrow of `t.attempts`: the compiler will say so. Cut.
- The "Why these choices" bullets for U1 and O1 restate facts.md. Keep a one-line "pinned by test (c)" and "pinned by test (f)", and drop the prose.
- The U2 bullet: the "implementer's first check" is stated again in Docs and in "For the owner". Keep it here only.
- The last bullet (whole file carried) says the same thing twice: "reviews everything that attempt's planner had done" and "what is carried is all of its work". Keep one sentence.

**The actions and effects**
- The opening "No new `AppAction` or `Effect`. The wire to Switchboard is unchanged..." restates the code. Reduce it to "the same `WorkflowStart`, with `plan_to_review().path` copied".
- The `rerun_carries` evidence for the one-key guard (the `:8407` match-arm remark) was needed for the reply to the reviewer, not for the implementer. Cut the parenthesis and keep the guard.
- The paragraph "The question is a snapshot (`ensure_decision`...)": the behaviour is the default, and no code changes. Cut.

**The UI**
- The sentence about older readers and the app being unaffected, and the `grep planFiles` note, restate S "Wire types". Cut both, and keep "CLI `show` only".
- The sentence in the `print_lane_files` bullet explaining "which is the reviewed copy and which is a prior revision" restates the issue. Cut.

**The tests**
- Item 4 ("Update any assertion on the exact rerun question text..."): the suite fails and forces this. Cut.
- Unit (a) "a `Complete` plan only" is covered by the existing tests. Unit (e) (`Cancelled` counts as `Failed`) is a one-line match arm. Fold it into (b) or cut it.
- Item 3's parenthetical repeats the step 2 rule. Cut.
- "A `main.rs` test ... if one exists; otherwise cover it through the integration test's `show` output": drop the conditional and say the integration test covers it.
- The `store.rs` test names the fixture style by line number. "Add a previous-version fixture test like the neighbours" is enough.

**Docs**
- `README.md`: "no new file, no new environment variable" has nothing to say. Cut the bullet.
- The `design.md` bullet repeats the U2 caveat for the third time. State it once.
- The CHANGELOG line is two clauses too long; one sentence is enough.

**One commit**
- "Contract additions" restates "The record" and "The UI". The repo rule only asks the commit message to say what was added, so list the three names and no signatures or prose. Drop the "Internal" line (`plan_to_review`, `rerun_carries`), since those are not contract.

**For the owner, after this lands**
- Whole section: bullet 1 is U2 and the Docs text, bullet 2 is departure bullet 4, bullet 3 is the UI gap, and bullet 4 is O1/U2 again. Cut it. If anything stays, keep one line: the owner's data directory settles O1 and U2.
