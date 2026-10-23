1. **"`Failed` and `Cancelled` both qualify" rests on nothing, and the tree cuts against it for `Cancelled`.**
   - The facts file settles only failed attempts (facts.md l.5-12, l.34). It says nothing about cancelled ones.
   - The tree sets `Cancelled` on a workflow attempt in three places:
     - the ticket closing, `scheduler.rs:2914`;
     - a stop with `reason`, `scheduler.rs:2872`;
     - a send-back, `scheduler.rs:4962` and `scheduler.rs:5585`. The reason there is `SENT_BACK_FROM…`, meaning the owner discarded that attempt on purpose.
   - The position test ("after the `Complete` writer in `t.attempts`") only covers the case where the plan stage reruns afterwards. A review attempt cancelled by a send-back from a later stage, with no new `plan` attempt in between, would still win. A rerun would then start from a copy the owner sent back.
   - The plan does not cite a line showing that a cancelled copy is wanted.
   - Robust default: qualify `Failed` only. Qualify `Cancelled` only if you cite the tree for which cancel reasons leave the copy newest and wanted, for example by excluding reasons that start with `SENT_BACK_FROM`. Drop the test "a `Cancelled` one counts" or change it to match.
   - The Departures section also says "every failed or cancelled review attempt". Narrow it to match.
