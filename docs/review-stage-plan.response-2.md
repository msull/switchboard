# Response to review 2

All six points stand, and each changed a rule the skeleton had
settled. None is rejected.

1. **Check reuse must match the gate.** Accepted. Reuse now requires
   the same resolved gate (argv, cwd, environment) at the same clean
   head, which in practice is the stage naming `implement`'s gate by
   reference with nothing moved. A different gate at the same head
   runs. Two acceptance rows added: different gates at one head (the
   review gate runs) and a referenced gate (reused).

2. **The base is the cut's SHA.** Accepted. The base is resolved once
   at the cut and persisted on the lane record as an additive
   `base_sha`, with a rule for lanes cut before the field existed
   (`merge-base` on the first pass, persisted then). The skeleton now
   says why: the shared clone's remote refs move on every other
   ticket's fetch. Rebasing is out of scope, and a head whose history
   no longer contains the recorded base is an unexpected movement.
   Row added for another ticket fetching a newer base between passes.

3. **Pid adoption replaced by a protocol.** Accepted. The pid rule is
   gone. The skeleton now states the properties the plan's mechanism
   must have (durable intent before execution, process identity not
   by pid alone, an exit result obtainable after restart, an ambiguous
   state resolving to a failed reviewer) and names a supervisor with
   durable acknowledgement and exit records as one shape, leaving the
   choice to the planner. The no-second-copy rule now carries its one
   exception, the stage's checks, which keep the command gate's
   restart rule and only after the previous check and its descendants
   are gone. Rows added for the spawn-to-acknowledgement window and a
   reused pid.

4. **The slot is the ticket's, held while any lane works.** Accepted.
   The slot is released only when the whole ticket is waiting with
   nothing running, and an answered decision reacquires it under the
   same limit before launching. Row added for two lanes under
   `slots = 1` with a second ticket queued.

5. **"One more round" authorises a fix pass.** Accepted. The answer
   now means exactly one fix pass against the last pass's findings,
   the checks, then one review pass, after which the cap decision
   returns if findings remain; it stands in for that round's per-pass
   `ask`. Row added for `cap = 1` answered with one more round, with a
   failed check stopping the sequence.

6. **Disputed points stay open until withdrawn.** Accepted. The next
   pass's reviewers receive both the feedback and the response and
   must withdraw or carry each disputed point; convergence requires no
   open point. Ids are scoped by round and a carried point keeps its
   original id. The single disputed-point row became two: withdrawn
   (converges) and rejected (stays unconverged).
