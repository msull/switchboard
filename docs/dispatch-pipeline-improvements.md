# Dispatch pipeline improvements

The living backlog for the Dispatch pipeline. Items 1 to 12 were drawn
from tickets #4 (PR 13, five code review attempts) and #10 (plan review
rerun after the reboot) on 2026-10-02; later items are added as tickets
show them. Each item carries a **Built** paragraph once an issue lands it.
The Backlog section at the end holds items not yet sized into issues; it
is gone through periodically and entries become issues.

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

**Built** (#22). The field is flat, `Attempt.carried_from`, and a chain of
failed attempts points at the one whose rounds gathered findings. Carried
ids are qualified with the attempt they were raised in
(`a<n>/r<round>/<reviewer>-<k>`), since ids are unique only within an
attempt; the old open points are open coming into round 1, and the prompt
lists the settled and open points in full. A note containing "start over"
gives the plain attempt, and the rerun question says so. The send-back
note moves onto the attempt (`Attempt.rework`) when it starts, which
needed `RECORD_VERSION` 4.

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

**Built** (#22). Style is tagged per point, `style:` in the point's text,
and an untagged point from the reviewer named `style` is style too. "The
fix agent sees them once" means the rounds before `style_rounds`: a
style-only round at or past it converges at the head the reviewers read,
with no extra fix pass. A carried attempt counts its rounds on from the
old attempt's. The summary is a new artifact, `summary.md`, written when
the attempt completes.

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

**Built** as prompt text (2026-10-03). The fixer's guidance carries the
checklist (everything added is read by something, no block left from
moving code, every doc comment on one item, the three checks before the
commit), and the style reviewer names "debris a fix round left" as its
own. #43 adds the mechanical half, built: an agent that stops with a dirty
tree is nudged in its own session, up to the pipeline's `on_dirty`
(default one nudge), before its attempt fails.

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
`SocketDown` until the runner restarts; after the reboot three client-pipeline tickets
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

**Built** as prompt text (2026-10-03). The `pr` stage's body has a
"Beyond the issue" section, one line per thing the branch does that the
issue did not ask for and which review round asked for it, and the PR is
opened after review so the body describes the final branch.

## 7. A code review stage leaves clean commits

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

**Built** (#30). Neither option as written: the code review stage takes
`commits = "keep" | "fold" | "one"`, and with `fold` its completion folds
each fix round's commits into the commits they amend (a `fixup!` into its
target, any other fix into the tip of the folded history at the head that
round reviewed), so the implementation commits survive as units and the
round commits disappear; `one` squashes the branch with its first
implementation commit's message. The rewrite never changes the tree (it
is checked before and after a compare-and-swap move, and moved back on
any failure), so the checks are not run again, and nothing is pushed: the
`pr` stage publishes the clean history once. The record is
`Attempt.rewrite`, written as intent before git runs, which needed
`RECORD_VERSION` 5. A branch already on the remote (its remote-tracking
ref holds the branch's commits, since an agent's push records nothing
on the ticket) is skipped, not folded. One departure from the issue's
acceptance: a tree dirty when the checks finish keeps the checks
question (`rerun | check | park`), not `rerun | park`, because the
checks did not run on that tree; `rerun | park` is for a tree the
rewrite itself finds dirty (after a restart). #40 added `keep` to the
rewrite's own failures (`rerun | keep | park`): it completes the stage at
the reviewed head with the history as it is, recorded as `skipped =
"the user kept them after the rewrite failed"`, since a rerun folds
the same way and fails again.

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

**Built** (#22). `Refreshed` records `commits` (the head before the
bring-up was not the old base), `notes` and `at_ms`. The check goes to the
first round that reads the new base, not again to a later round or a
rerun at the same base. Notes are attached only from a rebaser started
after the lane's previous bring-up.

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
cannot hold a stopped agent forever. #157 adds the idle Stop that lists
work still in flight: the hold reads the `Stop`'s `background_tasks` and
`session_crons`, a wakeup holds until its time plus 30 s, background work
holds for at most 30 minutes, and the app's plan review follows the same
rule. A present artifact completes whatever the Stop listed.

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

**Built** (#22). The section is found by heading (a title starting with
"Decisions" after an optional number). A plan without one is said to have
none, rather than inlined whole. A contesting point is written
`decided: ...`, never holds the round open, and is listed under "Found but
not done" in the round file and the summary.

## 11. A refresh at `ready` pushes when the lane has a pull request

**Problem, as first seen.** After `implement` opened PR 17 at one head, the
refresh step rebased the tree and five fix rounds committed, and none of
them pushed. `ready` then asked for a push by hand, and the push had to be
forced, since the refresh had rewritten the PR's commit.

**Most of it is gone by structure.** The Switchboard pipeline now opens the
pull request at a `pr` stage after code review, as the client pipeline always did: the
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
fixer and rebaser have no PR to keep true. The client pipeline was already shaped this
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

9, 4 and 5 are built (#21), and 11 (#23); 1, 2, 8 and 10 are built
(#22). 3 cuts the round count directly and is small. 6 is prompt text;
12 is withdrawn. 7 is built (#30).

## Backlog

Observed, not yet issues. Each line says where it was seen. Entries that
became issues: the style declaration line and the remedy log line (#38,
built), checks left running on a park (#39, built), `keep` on a fold
failure (#40, built), the CLI blocking behind a pass (#41), a dirty tree
nudges the agent (#43, built), the supervising agent's CLI (#44, built),
the orphaned check (#53, built), the unreviewed conflict rebase (#54,
built), the stale folded commit message (#55, built), a refresh that
launched the next agent into a tree still mid-rebase (#58, built: the
stage holds on a `recheck | park` question until the rebase is finished
or aborted by hand), `wait` bunching and `--for move` (#100, built:
both shapes), the orphaned command reviewer and the leaderless check
(#101, built), Dispatch's own lane-blind reads (#102, built), the runner
restart from away from the Mac (#103, built: `dispatch runner stop |
start | restart` over the control port, the supervisor's when its
table's `may` lists `runner`, refused while a deploy runs).

- **A look at the work before the PR opens.** #71 produced screenshots
  of the new ticket page, but only after the PR was open, and the two
  notes they raised (a meta line that wraps so the PR chip sits alone,
  a pinned-decision area no closed ticket exercises) had no stage to
  land in short of another ticket. Two shapes, not yet chosen: a
  `look` stage (`gate = { kind = "human", decision = "look" }`) between
  `review-code` and `pr` on pipelines that draw UI, whose question
  carries the attempt's screenshot paths and whose `fix` answer runs
  the fixer once with the owner's note and folds, so the PR opens
  with the fix in; or a `respond_to_user` on the code review, so an
  owner's note at the review-cap question is one more round for the
  same fixer. The first keeps the owner's look separate from the
  reviewers' rounds and is the one to plan. Either way the implementer
  needs a path that can read the data directory: its classifier
  refused the copy of a closed ticket on #71, so the throwaway data
  directory for screenshots should be made by Dispatch (a `dispatch
  stage-copy <ticket> <dir>` the implementer runs), not by the agent.
- **A command reviewer orphaned by a runner restart is never killed.**
  `start_reviewer` (`dispatch/src/git.rs`) puts a command reviewer in its
  own process group as it does a check, so it outlives a runner stopped
  by Ctrl-C too. #53 records a check's group on `GateRun` and stops it
  after a restart; `ReviewerRun` could take the same `group`. Seen while
  building #53.
- **A leaderless orphaned check is left running.** `adopt_check`
  (`dispatch/src/git.rs`) adopts a recorded group only under a live
  leader with the recorded start time, so a member that outlived its
  leader, including one that ignored the TERM its `sh -c` leader died
  of, runs on beside the checks started again. A still-running
  `orphans_killed` entry (same pgid, within `STOP_LIMIT_MS` of its
  `at_ms`) could serve as the proof for the second case. Seen in #53's
  review.
- **An untagged non-wording point from the `style` reviewer counts as
  style.** `class_of` (`dispatch/src/review.rs`) counts every untagged
  point from the reviewer named `style` as style, so one about behaviour
  never holds a round open. Seen on #33's round 2. Whether it should block
  is open.

- **The planner's response described edits that were not in the file**
  (#25, plan review round 2). Fixed as prompt text on 2026-10-03: the
  `respond` prompt now orders read, edit, re-read, then response. Listed
  so the pattern is remembered if it recurs under other prompts.
- **The commit message goes stale after a fold.** A `fixup!` keeps the
  implementation commit's message, so when a review round renames
  something the message's contract list names, the PR body is corrected
  and the message is not (PRs 45 and 46). The fixer should commit with
  `squash!` when its fix changes a name the message carries (the fixer's
  guidance says so since 2026-10-04), or the `pr` stage should check the
  message against the tree. Issue #55. Built in #55, at the review stage
  rather than at `pr`: the fold happens as the review attempt completes,
  and the `pr` agent pushes and opens the PR itself, so a check after
  the fold and before the attempt completes is the last point where a
  reworded commit needs no force push. A stale name asks `rewrite |
  accept | park`; `rewrite` rewords the folded commits one for one with
  the tree proven unchanged (docs/dispatch.md, "Folded messages").
- **A `pr`-stage rebase with conflicts is reviewed by nobody.** The
  refresh at `pr` entry rebases the reviewed branch; the checks run again
  at the new head, but no reviewer reads the resolution, so a dropped line
  that breaks no test lands (#40 and #39 on 2026-10-04). One delta pass by
  the correctness reviewer when the rebase had conflicts, as item 8 does
  for review rounds. Issue #54, built: a conflicted bring-up after the
  last code review stage that pushed nothing gets one `resolution`
  review of the range-diff between the reviewed and the resolved branch.
- **A conflicted rebase at `ready` with an open PR is not reviewed.**
  The bring-up pushes the resolution before anything could read it, and
  a fix would need a push leased on the fixed head, which the pass's
  attempt does not hold. #54 leaves such a bring-up to the PR's checks.
- **tmux adapter tests flake on the Linux runner.** #48 made every wait
  in the tmux and dispatch tests poll to a deadline and skipped fsync in
  tests (the dispatch suite went from about two minutes to half a
  minute). `command_exit_code_is_reported` and
  `a_killed_check_takes_its_process_group_with_it` still failed once each
  on CI afterwards and passed on rerun; the remaining cause is open.
- **Agents stop one step short.** A planner described edits it had not
  made (#25), an implementer stopped with ten files uncommitted (#40).
  Prompt text now says the stage is not done until the file or the tree
  says so; #43 makes the tree check mechanical (built: a nudge, then
  the question).
- **A gate that fails for the environment reads as a code failure.** The
  client pipeline's backend checks need a local container runtime; after
  a reboot it was down, and the gate failed with 825 fixture errors and a
  `rerun | check | park` question that looked like a broken branch. The
  pipeline now runs `docker info` first and says what is wrong in one
  line. A Dispatch shape worth considering: a per-lane `preflight` argv
  run before any gate or agent in that lane, whose failure is reported
  as the environment's, not the attempt's, and retried on the next pass
  without a question.
- **A cancelled attempt whose branch moved by one commit offers no
  `check`.** A one-line formatting commit made by hand on a lane parked
  the ticket (right: the branch moved by something other than the
  implementer) and the resume asked `rerun | park`, a full review pass
  for a change the reviewers had already accepted. When the new head is
  the gate head plus commits that touch no file the review's open points
  name, `check` could be offered beside `rerun`. Related: a gate whose
  lint step reformats a committed file fails as "the tree changed while
  the checks ran", which reads as an agent problem; naming the formatter
  and the file would send the fix to the right place.
- **Dispatch's own artifact reads are lane-blind.** #98 gave the
  `{inputs.*}` fields one lane rule, but `t.input("plan")` and
  `t.input("notes")` still take the newest complete attempt from any
  context: the refresh agent and the remedy/fixer agent
  (`dispatch/src/scheduler.rs`, each beside a lane-aware `vars_for`),
  the human gate's `Notes (<stage>)` line, the reviewers in
  `review.rs`, `report.rs` and `serve.rs`. With an `each` plan stage
  over more than one lane, a lane can be handed another lane's plan.
  Each read is fixed with `Ticket::input_where` and the reader's lane;
  `report` and `serve` have no lane and need a rule of their own.
  Listed as a known gap in design.md. Seen in #98's plan. Issue #102,
  built: each read takes the reader's lane, and a reader with no lane
  (a root or joined gate, `show`, `report`, the port) lists one file
  per lane when the newest writer runs per lane.
- **The README's test-times table is stale.** It records the serial
  run just after #48 (55.9 s, 791 tests). On 2026-10-05 after #94 the
  same script gives 102.7 s for 1093 tests: `first_slice` 73 s (318
  tests), the dispatch lib 16.6 s (the three confined-check tests from
  #1 take about 2.3 s each). Nothing is over the 5 s budget on a quiet
  machine, but under another build's load
  `a_lost_unmark_is_sent_again_until_it_lands` went from 0.95 s to
  13.4 s. Parallel `cargo test --workspace` is about 61 s, against the
  README's two-minute local budget. Refresh the table when the suite
  is next touched, and consider a budget line for the whole serial
  run.
- **A subscription delivery drops a stage the ticket has already left.**
  On #154 the first delivery carried only `3708 lanes → plan`; the line
  a second earlier, `3707 investigate → lanes`, never reached the
  supervisor. A delivery reads its lines as replayed, and `confirmed`
  (`dispatch/src/events.rs`) keeps a replayed `stage` or `sent-back`
  line only while the record's stage still has that line's name, so a
  stage the ticket passed through inside one settled burst is filtered
  out. Decisions are matched by id and survive, so nothing actionable
  was lost here, but the supervisor's picture of the path skips steps
  (and would hide, say, a short `refresh` between `ready` and `merge`).
  Either keep a passed-through stage line when a later line of the same
  ticket bears it out, or have the guide say a delivery shows where the
  ticket is, not every stage it crossed. Seen on #154, 2026-10-08.
- **A new supervisor session cannot tell a live subscription from a
  stale one.** Subscriptions belong to the project's supervisor pane and
  survive a reseed, but `subscriptions` prints the session id that
  created each one, so a fresh supervisor reads them as another
  session's and subscribes again (harmless: only the `since` seq
  moves). Either print where a delivery goes (the supervisor pane) in
  place of the creating session, or have `brief` mark each in-flight
  ticket as subscribed or not, so the seed's "subscribe what is not yet
  subscribed" has a clear answer. Seen 2026-10-09 at the start of
  session eeeb2948.
