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

**Built** (#21). A `none` reading within `PR_YOUNG_HEAD_MS` (two minutes)
of the head's last move (the gate attempt's start, the end of a rebaser or
fixer in its context, or a `recheck` answer) waits; past it, it asks.

## 5. The runner's socket client reconnects

Filed as #15. One timed-out call and every later call fails with
`SocketDown` until the runner restarts; after the reboot three Delta tickets
failed their review attempts this way. Reconnect on the next call after a
timeout, with the one-call budget kept.

**Size.** `port.rs`. Half a day.

**Built** (#21). The reconnect was already in place (19db35d): the port
drops its connection on any error, a timeout included. #21 pins that with
a real-socket test and makes the next occurrence diagnosable: the log line
carries the io error kind and the whole error chain, and a `workflow`
query's failed reply other than `no such run` is asked again rather than
failing the attempt.

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
feedback file landed eight seconds after the Stop.

**Size.** `poll_agent` plus a fake-port test. Half a day. First in order.

**Built** (#21). A Stop with the card `working` holds agent, reviewer and
fix attempts with no failure and no completion; a missing artifact fails
after thirty idle polls in a row (`STOP_IDLE_POLLS`) or when the pane
exits. Answering a decision now clears every session it marked, so a mark
cannot hold a stopped agent forever.

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

## 11. A refresh at `ready` pushes when the lane has a pull request

**Problem, as first seen.** After `implement` opened PR 17 at one head, the
refresh step rebased the tree and five fix rounds committed, and none of
them pushed. `ready` then asked for a push by hand, and the push had to be
forced, since the refresh had rewritten the PR's commit.

**Most of it is gone by structure.** The Switchboard pipeline now opens the
pull request at a `pr` stage after code review, as Delta always did: the
fix rounds and the refreshes before it touch a branch that has no PR, and
nothing needs pushing until the `pr` stage pushes once. CI runs once per
reviewed branch, and nothing unreviewed is public.

**What remains.** The refresh at the transition into `ready` runs after
the PR exists. When main moved between the `pr` stage and `ready`, the
mechanical rebase (or the rebaser) rewrites the PR's commits and must
push with `--force-with-lease` on the head the PR was last seen at, or
`ready` asks its one-answer question. The merge gate's rebaser already
pushes.

**Acceptance.** A refresh that rebases a lane with a PR pushes once with
the lease; a lane without a PR pushes nothing; a lease failure leaves the
`ready` question as today.

**Size.** `Repo::push_with_lease`, one call site in `refresh_lane`, two
tests. Half a day. Issue #23, narrowed.

**Built** (#23). When the provider reports an open pull request for the
lane's branch, a branch the refresh brought up (mechanically, or on the
pass after the rebaser stops) is pushed with `--force-with-lease` on the
head the lane's records last saw: the newest non-refresh attempt in its
context that recorded one, or the head a refresh last pushed (the lane's
`pushed`, record version 3) when that came later, never the
remote-tracking ref. This holds at any stage the refresh runs, so the
rebaser is told not to push, and a second refresh leases on the first
one's push. A lane without a PR pushes nothing. A refused lease or a
failed push leaves `ready` to ask its `pr` question about the head.

## 12. Withdrawn: the pull request body is written once

The `ready`-stage body check is not needed once the PR is opened over the
reviewed branch (item 11). The body describes the final code, and the
fixer and rebaser have no PR to keep true. Delta was already shaped this
way. The merge gate's rebaser, the one agent that still touches a branch
with a PR, keeps its instruction to fix any claim its rebase changed.

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

9, 4 and 5 are built (#21), and 11 (#23). 1, 2 and 3 cut the round
count directly and are small. 6, 8 and 10 are prompt text; 12 is
withdrawn. 7 is a taste call for the user before anyone builds it.
