# Response to review 1

The plan is a skeleton by design: it fixes the shape and the
invariants, and the separate planning process supplies the detail.
Where a point changes a rule the skeleton stated, the skeleton now
states the right rule. Where a point asks for detail the planner is
meant to produce, the skeleton now names it as a required `PLANNER:`
item rather than answering it here. Nothing is rejected outright;
three points are accepted in part.

1. **Checks on every completion path.** Accepted. A no-findings pass
   now runs the stage's checks at its head and completes only on a
   zero exit with a clean tree; the cap acceptance does the same.
   Earlier checks are reused only at the same clean head. The stage's
   `gate` is required. Acceptance row added: no findings, failing
   checks.

2. **Command reviewers are not command gates.** Accepted. The
   skeleton now says so explicitly: `poll_gate`'s restart of a lost
   child is right for a check and wrong here. Each command reviewer
   gets a durable launch record written before it starts; on restart
   an alive child is adopted, a gone one is failed and never
   relaunched. The process list covers command reviewers and checks.
   Descendant handling (a process group per reviewer) and the walk of
   every crash boundary are `PLANNER:` items, since they are
   implementation design, not shape. Rows added for restart during a
   command, partial fan-out, and park mid-round.

3. **A command result protocol.** Accepted. Exit 0 is diagnostic
   output and no findings; exit 1 with stdout is findings; anything
   else, or exit 1 with empty stdout, is an execution error. stderr is
   kept, never merged into feedback. Other conventions are normalised
   by a pipeline-owned wrapper. An empty or missing agent feedback
   file is a failed reviewer, never approval. The sentinel matches
   only the whole trimmed file. Rows added for each.

4. **Completion and retirement for both agent kinds.** Accepted in
   part. Claude completion is fixed as the ordinary agent rule (Stop,
   file present and settled), and the skeleton now says Stop is a
   finished turn, not an exit, so the session is killed and read back
   as gone before the implementer touches the tree. Implementer
   completion (clean committed tree, `response.md` required, failed
   checks a decision, nothing paid launched after a failure) is
   stated. The Codex rule stays a `PLANNER:` item: the workflow stage
   already settles Codex output by file mtime, so a supported
   mechanism exists, but choosing it and its stall rule belongs to the
   plan, and its verification to the live test the acceptance section
   now requires.

5. **Immutable target and head transitions.** Accepted. Base and head
   SHAs are persisted before fan-out; the base is the lane's base ref
   as cut, resolved to a SHA. The tree must be clean at the recorded
   head when reviewers finish. The implementer's commits are an
   authorised transition recorded before and after; any other movement
   is the design's moved-head rule and parks the ticket. The stage's
   accepted head is what downstream gates bind to and its checks
   replace `implement`'s. Another lane's commits do not touch this
   lane. Rows added for external mutation, the other lane's commit,
   and a stale decision.

6. **Read-only reviewers.** Accepted, with the honest framing:
   detection, not prevention. The skeleton no longer implies Dispatch
   can keep an agent from writing to a tree it can see. The launch
   shape reuses what the plan review does (`write_flags`,
   `reviews_in_tree`), the round records head and cleanliness at start
   and end, and any change fails it. Command tools that write caches
   are a `PLANNER:` item with the expected answer given (an ignored
   path is clean). Row added for an uncommitted change.

7. **Round, cap and decision ordering.** Accepted. `cap` counts review
   passes; the last pass is a review pass, never a fix pass, so no
   unreviewed head is ever offered. The cap decision offers the last
   reviewed head, one more round (cap plus one), or park. The per-pass
   `ask` decision authorises the fix pass only; a converged pass never
   asks. Decisions bind to lane, attempt, round and head, and a stale
   answer launches nothing. Rows added for `cap = 1`, the cap
   decision, and a stale answer. A no-op fix pass is covered by the
   disputed-point row: the response is handed forward instead.

8. **Schema, validation, persistence and views.** Accepted in part.
   The skeleton now states that the stage needs a fourth `StageKind`
   with `reviewers` as its discriminator, lists the validation cases
   to refuse, records that `Attempt` and `AttemptView` hold one
   session and one gate today and so need additive round records, and
   sets stable point ids and per-round artifact paths. Naming every
   field, its default and the view additions is the plan's job, as is
   confirming that the Switchboard side needs nothing in `model.rs`,
   `AppAction` or `Effect`; the skeleton states that expectation and
   requires the plan to confirm or name the additions and the schema
   step. The reviewer namespace stays an open choice with one answer
   demanded.

9. **Concurrency, cost and acceptance.** Accepted in part. Slot
   accounting is settled: one ticket, one slot, held from first launch
   and released while a decision is pending; per-reviewer budget
   charging is stated as the expectation for the planner to confirm.
   Sibling cleanup on a reviewer failure is stated. The acceptance
   table gained every case above and a two-lane row, and the reviewer
   items require one named test per row plus an `#[ignore]`d live
   test for Codex completion and write permissions. The test names
   themselves are not filled in here: they follow from the plan's
   choices (attempt-per-round or rounds-in-attempt changes what a test
   asserts), so writing them now would be guessing.
