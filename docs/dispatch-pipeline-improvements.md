# Dispatch pipeline improvements (draft, 2026-10-02)

Drawn from tickets #4 (PR 13, five code review attempts) and #10 (plan review
rerun after the reboot). The agents' output was good on both; the cost was
in the machinery around them. Ordered by expected return.

## 1. A review attempt carries its settled points forward

**Problem.** Every review attempt starts at round 1 over the whole diff.
#4 was reviewed five times because of a false dirty-tree failure, a reboot
and three rebases. Each attempt re-found style points already settled and
spent its first round re-reading code the previous attempt had passed.

**Change.** A new attempt of a `review-code` stage in the same context
starts from the last attempt's final state: the reviewed head, the points
still open, and the points withdrawn. The reviewer is told "the tree at
`<old head>` was reviewed through round N; these points were settled, this
one is still open; review the change from `<old head>` to `<new head>` and
the open point only." A `rerun` answered with a note still starts fresh when
the note says so.

**Record.** `Attempt.review.carried_from: Option<(stage, n)>`; the prompt
builder reads the earlier attempt's round files. Nothing new on disk.

**Acceptance.** A review attempt that failed at round 3 and is rerun opens a
reviewer whose prompt names the old head and the surviving points, and the
new attempt's round 1 feedback file lists no point withdrawn in the old
attempt. A rerun with the note "start over" gets the plain prompt.

**Size.** Scheduler prompt builder plus one test. A day.

## 2. Style does not hold convergence past round 2

**Problem.** On #4, rounds 2 to 5 of the long attempt were withdrawals plus
one or two style points each. Convergence waited on comment wording while
nothing about behaviour was in question.

**Change.** A review round whose only kept or new points are `style`
converges when the round number is 2 or more. The style points are still
written to the round file, the fix agent still sees them once, and the
summary names them as "left to the merge". Correctness points converge as
today.

**Policy.** `review.style_rounds` (default 2) on the stage, live.

**Acceptance.** A round-3 feedback with one style point and no correctness
point ends the review as converged with the point listed under "left".

**Size.** `review.rs` verdict reading plus a test. Half a day.

## 3. The fix agent tidies before it stops

**Problem.** Fix rounds left debris the next round had to catch: an enum
nobody reads, a bare block left from lifting a loop body, a doubled doc
comment. Each cost a round.

**Change.** The fix prompt ends with a fixed checklist: every item the
round introduced is read by something; no leftover scaffolding from moving
code; doc comments attach to one item; `cargo clippy` and the tests ran. The
reviewer prompt gains the matching line: "debris from the fix is a style
point, not a correctness point" (which keeps it under item 2).

**Acceptance.** Prompt text only; checked by reading. No test.

**Size.** An hour.

## 4. A young head is pending, not "no checks"

Filed as #16. `judge_pr` reads `Checks::None` as a question. In the first
minute after a push GitHub has not created the runs yet, and the gate asked
on #4 about thirty seconds too early. Treat `none` as pending while the head
is younger than a grace window (two minutes), then ask.

**Size.** One branch in `judge_pr` with the head's age, one test. Half a
day.

## 5. The runner's socket client reconnects

Filed as #15. One timed-out call and every later call fails with
`SocketDown` until the runner restarts; after the reboot three Delta tickets
failed their review attempts this way. Reconnect on the next call after a
timeout, with the one-call budget kept.

**Size.** `port.rs`. Half a day.

## 6. Scope delta in the pull request

**Problem.** #4 asked for closing a ticket by hand; the PR shipped record
versioning, a ledger `settled` flag, a reworked queue-view sync, a pre-commit
fix and a lint move, all from review points. Each was right. The human
merging it only learns the scope moved by reading the whole description.

**Change.** The `ready` stage's PR body gains a fixed section, "Beyond the
issue", written by the implementer from the plan's review rounds and its
own fix rounds: one line per thing the PR does that the issue did not ask
for, with the round that asked for it. Empty is a valid answer and is
written as "Nothing".

**Acceptance.** The PR body contains the section; a prompt test checks the
instruction is present.

**Size.** Prompt plus the PR template. An hour.

## 7. One commit per round is noise; squash at the merge

**Problem.** PR 13 carries twelve commits named "code review round N" and
"Rebased onto main: ...". None is a unit anyone would check out.

**Change.** Either of:

- the `merge` stage's question says "squash and merge", and the ready
  stage writes the squash message into the PR body's first section so the
  GitHub squash dialog picks it up; or
- the implementer squashes before `ready`, with the plan's commit message,
  then the PR has one commit and the merge mode does not matter.

The second keeps the repository's "one commit per milestone" habit and
survives a non-squash merge button. It costs a force push before `ready`,
which the refresh step already does.

**Acceptance.** A ticket reaching `ready` has one commit ahead of its base.

**Size.** One instruction in the ready-stage prompt plus a `Repo::squash`
helper if done mechanically. A day.

## 8. A refreshed review reviews the rebase too

**Problem.** After the refresh step rebases a branch, the next code review
reads the diff against the new base and cannot tell a conflict resolution
from the feature's own change. The hand rebase on #4 moved a field into the
wrong struct and only the compiler caught it.

**Change.** When a lane's `refreshed` is set and the branch had commits, the
reviewer prompt names the range that moved and asks for one explicit check:
"both sides of every conflicted hunk are present and the base's additions
are unchanged". The rebaser's notes file, when there is one, is attached.

**Acceptance.** Reviewer prompt contains the base range after a refresh
with commits; absent for a clean move.

**Size.** Prompt builder. An hour.

## 9. A Stop with the pane still busy is not completion

**Problem.** #18's investigator fanned its audit out to eight background
agents and ended its turn to wait for them. Claude Code fires `Stop` at the
end of every turn, so Dispatch saw a stop with no notes file and failed the
attempt ("stopped without writing notes") while the session was alive and
about to write them. A `rerun` there kills the work.

**Change.** After a `Stop` with the artifact missing, hold the attempt open
while the pane is `Running` and the session's status is not idle at a
prompt: a background agent still working, or a tool call in flight. Fail
only when the pane is idle at its prompt with no artifact after a grace of
a few polls, or when it exits. The `card` state the port already reports
("working", "idle", "waiting on you") is enough to tell these apart.

**Acceptance.** A session that stops, keeps working, and writes its notes
on a later turn completes the attempt; one that stops idle with no notes
still fails after the grace.

**Seen three times on #18.** The investigator (eight background agents),
then two code review attempts: the correctness reviewer's Stop fired while
its subagents ran, the round failed after three one-second polls, and the
failure killed the style reviewer mid-review. In the first of those the
feedback file landed eight seconds after the Stop. Until this is built,
every reviewer's guidance in the live pipeline files says to work
in-session and write before the turn ends.

**Size.** `poll_agent` plus a fake-port test. Half a day. First in order.

## 10. The plan's decisions reach the code reviewers as settled

**Problem.** #10's plan listed four decisions with their reasons (running
is not activity, pane output counts for every kind, every kind is
included, a rule set covers its own workspace). The code reviewers raised
two of them again as points, and the fix round spent itself re-arguing
the plan.

**Change.** The code review prompt carries the plan's "Decisions" section
(or the whole plan, when it has none) as settled: a point that contests a
listed decision is out of scope for the round and goes under "Found but
not done" in the summary instead.

**Acceptance.** The reviewer prompt contains the plan's decisions; a
prompt test checks the instruction.

**Size.** Prompt builder in `review.rs`. An hour.

## 11. A rebase or a fix round pushes when the PR exists

**Problem.** After `implement` opened PR 17 at one head, the refresh step
rebased the tree and five fix rounds committed, and none of them pushed.
`ready` then found the PR behind the tree and asked for a push by hand, a
question with one possible answer. The push also had to be forced, since
the refresh had rewritten the PR's commit.

**Change.** When the lane has a pull request, the refresh step and each
in-review fix round that passes its checks push the branch with
`--force-with-lease` on the head the PR was last seen at. The `ready`
question stays for the case where the push itself fails.

**Acceptance.** A fix round on a lane with a PR ends with the fake repo's
push list naming the branch and the lease; a lane without a PR pushes
nothing.

**Size.** `Repo::push_with_lease`, two call sites, two tests. Half a day.

## Not changing

- The reviewers' standard. They found real bugs every attempt (recovery on
  a closing ticket, a prune on the shared clone, a 30 s socket hold on the
  page's Close, a late reply reviving a cancelled attempt, a read outside
  the lock). The cost above is in reruns, not in what a round finds.
- The plan review catching stale plans. The refresh step now brings the
  branch up before implementation, so the reviewer's catch on #10 becomes
  the exception, and when it happens it is still the right outcome.
- Review from the original session's context (planner clone as rebaser,
  implementer continued for fixes). It is why logical conflicts get
  noticed.

## Order

9 first: it failed three attempts in one night. 1, 2 and 3 cut the
round count directly and are small. 4 and 5 are filed bugs. 11 removes a question with one answer. 6, 8 and 10 are prompt text. 7 is a taste call for the user before anyone
builds it.
