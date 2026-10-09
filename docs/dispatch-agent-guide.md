# Dispatching tickets: a guide for project management agents

This is for an agent that grooms a backlog, manages contractors, or
otherwise decides what gets worked on, and needs to hand that work to
Dispatch directly. Dispatch is the scheduler that takes a GitHub issue
or a pull request, makes a ticket, and runs it through a per-project
pipeline of agent stages (investigate, plan, review, implement, code
review, inspect, merge) with the owner's decisions in between. You
start tickets, order them, read what is waiting, and answer only the
decisions you have been told you may answer. You never touch the
pipeline files, the ticket records, or the worktrees.

## The command

```
~/Applications/Switchboard.app/Contents/MacOS/dispatch <subcommand>
```

It is not on `PATH`. Use the full path, or define
`alias dispatch=~/Applications/Switchboard.app/Contents/MacOS/dispatch`
in the shell you run it from. Everything below writes `dispatch` for
short.

Every subcommand prints one line per thing and exits non-zero with a
reason on refusal. Read the reason; do not retry the same command.
A command that waits more than two seconds for the writer lock says so
on stderr, naming the process that holds it; stdout is unchanged.

```
dispatch take <project> <issue-number>           ticket from a GitHub issue
dispatch take <project> pr [<remote>:]<lane>/<n>  ticket from someone else's pull request(s)
dispatch status                                   every ticket: id, project, title, stage, state
dispatch decisions                                what waits on the owner, with the exact answer command
dispatch decide <ticket> <decision> <answer> [--note <text> | --file <path>]
dispatch queue <project>                          the project's queue in order
dispatch queue <project> <ticket>...              reorder it
dispatch park <ticket> [--reason <text>]          a ticket's work stopped, its questions withdrawn
dispatch resume <ticket> [--no-rerun]             a parked ticket back to active; what the park cancelled runs again
dispatch close <ticket> [--reason <text>] [--drop-evidence]
                                                  a ticket closed, its trees removed (its branches are kept; close lists them);
                                                  --drop-evidence removes its evidence directories now
dispatch restart <ticket> [<stage>] [--note <text> | --file <path>]
                                                  a ticket at its stage, or an earlier one, under the live pipeline; later work
                                                  discarded; a note goes to the stage's next agent
dispatch health [--timeout <secs>] [--stale <secs>] [--verbose] [--json]   is the runner alive and getting on; run it first
dispatch runner stop|start|restart                stop or start the runner the app runs; restart waits for the new pid
dispatch show <ticket> [--json]                   one ticket: stage, lanes, attempts, rounds, decisions, files
dispatch evidence <ticket> [<stage>]              each attempt's evidence files, by absolute path
dispatch events [--since <seq>] [--follow [--timeout <secs>]] [--ticket <id>]... [--project <name>] [--json]
dispatch brief <project>                          the project at a glance: tickets, what waits, recent events, the hand-off
dispatch supervisor <project>                     the project's supervisor session: its id, age, seed, workspace
dispatch wait <ticket> [--for decision|stage|pr|closed|move|any] [--since <seq>] [--timeout <secs>] [--json]
dispatch subscribe <ticket> [--for move] [--since <seq>]   the runner types the ticket's moves into the supervisor's pane
dispatch unsubscribe <ticket>                     stop that
dispatch subscriptions <project>                  each subscription, what it has not delivered, and why it waits
dispatch report <ticket> [--json]                 how a ticket went: stage time, review points, fix passes, size
dispatch report --project <name> [--since YYYY-MM-DD] [--json]
dispatch tail <ticket> [--lines N]                what the ticket's running agents show
```

`dispatch run`, `dispatch worktrees` and `dispatch supervisor <project>
--fresh`, `--resume` or `--kill` are the owner's: never run them.
`dispatch runner` is the owner's too, unless the project's
`[supervisor]` table lists `runner` in `may`: then the supervisor may
`restart` the runner after a merge that touches `dispatch/`, and
`start` it when `health` says it is down. A `stop` or `restart` is
refused while a deploy runs; retry it when that stage ends. A
supervisor without `runner` still stops and reports when a command
cannot reach the socket, and no supervisor ever runs `dispatch run`.
`dispatch restart` is the owner's too, unless `may` lists `restart`:
then the supervisor may restart its own project's tickets, but only a
ticket the owner named for a restart, or a ticket handed to it to take
to done whose pull request at `merge` cannot merge because it
conflicts. A restart of a ticket running a deploy is refused; retry it
when that stage ends. Its report says what it restarted, at which
stage, and why.

## Projects and what they take

A project is a pipeline file the owner keeps; `dispatch status` lists
every project by name with its load. Each project takes issues from
one GitHub repository, has one or more lanes (the repositories a
ticket may touch, by name), and may also have a pull-request pipeline
that takes other people's PRs, sometimes from a named mirror remote.
Which repositories, lanes and remotes those are is the owner's to tell
you; keep their table beside this guide. Project names are
case-sensitive and spelled as the owner gives them.

## Starting work from an issue

```
dispatch take <project> 104
dispatch take <project> "#5"
```

The number is the GitHub issue number in the project's issue
repository, with or without `#`. The output is the ticket id (eight
hex characters), the issue number and the title. Keep the id: every
other command takes it.

Rules the command enforces, so you do not have to check first:

- One live ticket per issue. Taking an issue that already has an
  active or parked ticket is refused. A closed ticket does not block a
  new take.
- The issue is snapshotted as it is at take time: title, body, labels.
  Edits to the issue afterwards do not reach the ticket. Finish
  grooming the issue (clear title, the problem in the body, acceptance
  criteria) before taking it, because that body is what the
  investigating and planning agents read.
- The ticket joins the end of the project's queue. Nothing runs until
  the project has a free slot, and the queue order is the order
  tickets start.

Before taking, check the project's load with `dispatch status`: each
project line shows slots in use and decisions waiting. A project with
all slots busy still accepts the take; the ticket simply waits. A
project at its decision limit (five waiting) starts nothing new until
the owner answers something, so a take then is not wrong, just slow.

## Starting a review of someone else's pull request

```
dispatch take <project> pr <lane>/3
dispatch take <project> pr <lane-a>/3 <lane-b>/12
dispatch take <project> pr <remote>:<lane>/3
dispatch take <project> pr 6
```

One ticket, one PR per lane. A change spanning two of a project's
repositories is one ticket naming both lanes; two unrelated PRs are
two tickets. Where a project's collaborators work on a mirror of the
repository rather than the one the lane builds from, the remote's name
goes in front (`<remote>:<lane>/<n>`); the owner's table says which
projects need it. With a single-lane project the lane may be omitted
and the number stands alone.

The pipeline checks out the PR's branch, has reviewers read it against
the branch it targets, puts their notes where the owner can see them,
asks the owner to inspect, and then watches the PR until it is merged.
Nothing is posted back to GitHub and nothing is merged by Dispatch.
The take is refused for a closed PR, a lane named twice, an unknown
lane or remote, or a PR already on a live ticket.

## Reading state

`dispatch status` prints one line per project (slots in use,
decisions waiting, and whether anything is holding new starts), then
one line per ticket. Each ticket line reads: id, project, source,
title, current stage, state (`active`, `parked: <why>`,
`closed: <why>`), the latest attempt and whether a decision is
pending. The source is `#12` for an issue ticket and `pr <lane>/<n>`
for a pull-request ticket (two lanes joined with `+`), as `take`
spells them, so an issue and a PR with the same number never read
alike. Grep by ticket id when reporting on one item; the output may
be piped through `head` or `grep -m`.

`dispatch decisions` lists what waits on the owner. Each entry gives
the ticket id, the decision id (`d1`, `d2`, …), the stage, the
question in full, the options, and the exact `dispatch decide` line.
Copy that line; do not compose one from memory.

`dispatch queue <project>` prints the queue in order with a rank
number, ticket id, source and title.

## Supervising

When you are the one watching tickets through to the end, these are
the commands, by the question you have:

| Question | Command |
|---|---|
| Is the runner alive? | `dispatch health`, run first |
| Where does ticket X stand? | `dispatch show X` |
| What changed since I last looked? | `dispatch events --since <seq>` |
| Tell me when it needs me | `dispatch wait X --for decision` |
| How did it go? | `dispatch report X`, or `dispatch report --project <name>` |
| What is the agent doing right now? | `dispatch tail X` |

Every one reads and nothing else: none of them changes a ticket,
starts an agent or spends money. `--json` gives the same thing as one
JSON document (one per line for `events`) for a program to read.

**Health.** `dispatch health` checks that `dispatch run` wrote its
status file within `--stale` seconds (default 30) and its process is
alive, that Switchboard's socket and Dispatch's own answer within
`--timeout` seconds (default 2), each with its latency, and lists the
calls to Switchboard and to the PR provider that failed in the last
hour. Failures that repeat for one ticket with one error are one line
with a count and the last and first time; `--verbose` lists each. A
ticket with an open attempt the runner has not got on with for longer
than `--stale` is named. Exit 1 means something is wrong; read
the lines and report them. Do not restart the runner yourself.

**Show.** `dispatch show X` prints the header (id, project, source,
title), `stage <name> (i/n)` and the state; each lane with its branch,
worktree and the base, head and last pushed head (seven characters),
and, once it was brought up, a clause in the words of its `refreshed`
event, such as `, rebased cleanly`, `, brought up with no commits of
its own`, `, rebased by the rebaser, conflicts in 2 commits, reviewed:
1 point fixed` or `, rebased by hand (adopted), conflicts in 1 commit,
under review` (the review of a conflict also reads `reviewed`,
`accepted with 1 point open` or `review failed`); the attempts grouped by stage, each with its state and reason, head,
the head its checks ran at and a history rewrite, with any PR and the
code review rounds (`r<n> <state> open <k>`) beneath it; the pending
decisions with their options and the exact `dispatch decide` line; and
the files to read next: the plan, the latest round's findings, the
code review summary, the notes, and the PR's url and head. When the
stage that last wrote the plan or the notes runs per lane, each lane's
file is its own line (`plan (A): ...`, `notes (B): ...`). While a plan
review is open, the plan line names the review's copy, which the
planner may still be editing, and the line under it says so, under
each lane's own line when the review runs per lane:
`(reviewed copy, round N, review open)`, or `(reviewed copy, review
open)` before round 1's first feedback file. That copy is what
`finalize` would accept. Read those files rather than guessing what
they say.

**Events.** Every ticket write that changes something appends one
line per change to `events.jsonl` in the data directory: `taken`,
`stage` (forward) and `sent-back`, `restarted` (a `dispatch restart`
applied, in place of the stage move), `attempt-started` and
`attempt-ended` (with its state and reason), `decision` (its name,
question and options), `answered` and `decision-cancelled`, `pr` and
`pr-checks`, `pushed` and `refreshed`, `rewrite`, `round` (a code
review round and its open points), `nudged` (a line typed into an
agent's session after it stopped with a dirty tree),
`check-orphan-killed` (checks a previous runner left running, stopped
by this one before they ran again or the attempt was cancelled),
`parking`,
`parked`, `resumed`, `closing`, `closed`, `refused` (a supervisor's
answer to a decision that is not its to answer), `forgotten` (a secret
artifact's file deleted, with its name and why), `revised` (the owner's
objection to a finished plan review sent as its next round, by whom),
`evidence-swept` (an attempt's evidence directory removed, with its
file count and why), and `void`. A take,
park, resume, close or answer a supervisor session made carries
`"actor":"supervisor"` and its text ends `(by supervisor)` or reads
`by supervisor`. The human line is
`seq  hh:mm:ss  ticket  stage  kind  text`, the time in the machine's
local zone (the stored `at_ms` under `--json` is UTC milliseconds).

- Keep the highest `seq` you have read and pass it as `--since` next
  time. The cursor is yours: Dispatch stores nothing for you, and two
  readers never disturb each other.
- An event can repeat: a crash between the log line and the record's
  write makes the next pass do the transition again and log it again.
  Treat a repeated `answered` or `stage` as one.
- A `void` line withdraws the seqs its `voids` names: the write they
  described failed, so the transition did not happen. Without
  `--follow` those lines are already left out; with it, drop them when
  the `void` arrives.
- `--follow` looks at the file four times a second, so an event can
  lag by up to 250 ms. It runs until you stop it.
- `--follow` with no `--since` starts at the tail: it prints what
  happens next, not the history. Give `--since <seq>` to replay from a
  cursor and keep following.
- Tickets taken before this build have no events before their next
  transition. `show` and `report` read the record, so they are whole.
- A `refreshed` line reads `<lane> from <base> to <base>, <how>`, where
  `<how>` is `brought up with no commits of its own`, `rebased
  cleanly`, `rebased by the rebaser`, `rebased by hand (adopted)` or
  `rebased after the rebaser stopped`, the last three followed by
  `, conflicts in N commits` when a conflict was recorded (`commits it
  could not list` when they could not be read). Under `--json` it
  carries `by` (`git`, `rebaser`, `hand` or `stopped`) and `conflicts`.
  `stopped` means a rebaser failed or was cancelled and nothing was
  answered since, so the record cannot tell whether the rebaser or a
  hand rebase finished the work: look at that bring-up. `conflicts` is
  0 for `git`; for the others it is the conflicting commits, 0 when
  they could not be listed, and absent when none was recorded. Lines
  written before this build read `..., rebased`. A `refreshed` line
  can name `root` for the ticket's tree, which is never rebased by the
  rebaser.

**Wait.** `dispatch wait X --for decision` blocks until the ticket asks
something and prints that `decision` event with the `dispatch decide`
line to answer it. The record is read first. If a decision already
waits, `--for decision` returns it at once, printed, with exit 0, and a
decision answered before the wait reads it is passed over. A ticket
already parked or closed ends every wait with exit 3 and the reason,
except `--for closed` on a closed ticket, which exits 0; see below for
`any` and `move` with `--since`. `--for stage` waits for a stage move or a restart,
`--for pr` for a PR bound to an attempt, `--for closed` for the close,
and `--for any` (the default) for the next event of any kind. `--for
move` waits for what a supervisor acts on: a stage move, a send-back
or a restart, a decision, a `pr` or `pr-checks` line; a park or close
ends it with exit 3. A stage move or a PR needs a before state, so
without a cursor `--for stage` and `--for pr` wait for the next one.
With `--since <seq>`, the first matching event after the cursor ends
the wait, whoever caused it: at once for `decision`, `stage`, `pr` and
`closed`, after the burst settles for `any` and `move`. Every match is
checked against the record first, so a withdrawn event is never
returned. An event logged just before its record lands is held until
the record catches up, so an `any` or `move` watch does not miss it.

`--for any` and `--for move` return a burst: every match logged
within two seconds of the one before (ten after the first at most),
in log order, measured by the lines' own times, so a watch from an old
cursor returns the history a burst at a time. A decision ends the
burst at once with exit 0; a park or close ends it after the lines
logged before it, with exit 3. On a ticket already parked or closed,
a watch with `--since` prints the matching lines after the cursor
before the park or close. Follow from the seq of the last event line
printed: the next watch returns whatever the burst left, the park
included. With `--json` each event is one object on its own line, so
these two filters can print several, and the last carries the cursor.

To wait on your own action, take the log's tail just before you act,
act, then wait with `--since`. The tail is the `seq` of the last line
of an unfiltered `dispatch events --json`, or 0 when it prints nothing.
A listing filtered with `--ticket` or `--project` gives an older seq,
and a wait from it can return an event from before your action, under
`--for any` most of all. The same recipe blocks on a resume: take the
tail, `dispatch resume X`, then `dispatch wait X --for any --since
<tail>`. Under `--for any` the wait returns the burst your action set
off, not only its first line.

**Report.** `dispatch report X` gives the time per stage with the time
its decisions waited on the owner apart, the plan's size (one
`plan (<lane>)` line per lane when the plan stage runs per lane), the plan
review's rounds and points, the code review's rounds and points (each
point once, however many rounds it stayed open, split by reviewer),
fix passes, rebases (at least: a lane keeps only its last clean
bring-up), and the commits and diff at the PR. Cost and
turns read `not recorded`: Dispatch does not see them. A count marked
incomplete is missing a round's file. A resolution review (the
`resolution` pass after a conflicted rebase) is code review work on the
branch: its rounds, points and fix count in those totals, and it is
listed under the stage `resolution`.

**Tail.** `dispatch tail X` prints the last lines (40, or `--lines N`
up to 200) of each running agent of the ticket's open attempt, headed
`== <stage>/<context> <role>`. Values of the project's secrets show as
`<NAME>`. `no agent running` means nothing of the ticket runs now. A
Switchboard too old for it says to update the app; report that.

**Answering what you find.** When `wait` or `show` gives you a
decision, read its question and the files `show` names, then answer
only the kinds you were allowed, by copying the `dispatch decide` line
and adding `--note "<what to change>"` where the table below allows
one (a `rerun` at `inspect`, a `rerun` of a failed attempt). By the
kind of failure:

- `rerun` after a crash, a missing result or a dirty tree: `rerun`,
  once. A second failure of the same kind is the owner's.
- `rerun` with `check` offered (the stage's tests failed): read the
  checks' log from `show`; `check` only if the failure looks flaky,
  else report it.
- `rerun` with `keep` offered (a fold or squash could not apply):
  `keep` completes the review with the history as it is.
- A gate that exited 127: neither `rerun` nor `check` helps; see the
  pipeline traps below and report.
- `pr` or `refresh`: the owner fixes the PR or the rebase by hand;
  report it, then `recheck` once told it is done.
- `paused`, `review-code`, `review-cap`, `finalize`, `message`: only
  with the owner's say for that project. `message`'s `rewrite` spends an
  agent run. `finalize`'s `revise` spends a planner turn and a reviewer
  turn, and its note is what the owner told you, not your own review.

**Exit codes.** Every command exits 0 when it did what it says and 1
with a reason when it was refused or failed. `wait` exits 2 when its
`--timeout` passed and 3 when the ticket parked or closed, so what it
waited for will not come. Exit 64 is a command line Dispatch cannot
read, printed with the usage, or a command the ticket's state never
allows (`park` on a closing or closed ticket), printed with its reason
instead.

## Ordering the queue

```
dispatch queue <project> ed9b2305 3e0dcacd baea8dbe
```

List every ticket id in the order wanted. A ticket not in the project's
queue, or a missing one, is refused and nothing changes. Running
tickets keep running; the order only decides which waiting ticket
starts next when a slot frees. This is the one lever a grooming agent
should use freely.

**With a timeout.** `dispatch events --follow --timeout <secs>` exits
0 as soon as it has printed something and 2 if nothing came in that
long, as `wait` does. An agent's Bash call is cut off after two
minutes by default and ten at most, so a watcher passes its tool the
longer timeout and keeps `--timeout` under it: `dispatch events
--project <name> --since <seq> --follow --timeout 540`, noting the last
seq it printed and starting again from it, one call at a time. While
driving one ticket, `dispatch wait <ticket> --for move --since <seq>
--timeout 540` is the better call: it returns on that ticket's next
stage change, decision, pull request line, park or close after
`<seq>`, with every such line that follows within two seconds. Arm the
first watch from the `follow from seq` that `brief` printed. After a
return, arm the next from the seq of the last event line it printed;
after `timed out`, from the same seq again. A decision raised between
two watches is then returned, and one left pending does not come back.

**Subscribe.** A supervisor in Switchboard does not watch at all:
`dispatch subscribe <ticket> --since <seq>`, from the `follow from
seq` that `brief` printed, has the runner type that ticket's moves
(what `wait --for move` returns, and its park or close) into the
supervisor's pane as its next prompt, once the pane is idle. Each
delivery is a settled burst, as `wait` returns one, headed `Dispatch
subscription: events on <tickets>`, with each line's seq and the
`decide` line for a decision still pending. Nothing is re-armed. The
subscription is on the project's record, so a fresh supervisor
inherits it; a delivered close ends it, and `dispatch unsubscribe
<ticket>` ends it sooner. Without `--since` it starts at the log's
tail and replays nothing, and a park standing when it starts is not
delivered. The runner types only when Switchboard says the pane is
ready for a prompt: a running Claude Code session between turns, no
permission prompt or trust question up, no answer of the owner's
waiting, and no key typed into its embedded terminal in the last
minute. `dispatch subscriptions <project>` lists each subscription,
how many lines it has not delivered, a delivery in flight, and why
deliveries wait (busy, at a prompt, the owner typing, not running,
or an app too old to know `session.prompt`). In a pane outside
Switchboard nothing is typed, so a watch is still the way.

**Brief.** `dispatch brief <project>` is what a supervisor reads
first: its own session's line, the project's open tickets with stage
and state, what waits on the owner with the `decide` line for each,
the last 20 events with the seq to follow from, its subscriptions with
the lines each has not delivered, the open worktrees, and the
supervisor's hand-off. It reads and changes nothing.

### Supervisor sessions

A project whose pipeline has a `[supervisor]` table gets one
long-lived Claude Code session that watches its tickets: the owner
starts it with `dispatch supervisor <project> --fresh` or the Fresh
button on the Dispatch page. Its seed names the `dispatch` executable
by full path, the decisions it may answer (`decides`), and its
hand-off file, which it keeps current for the next supervisor.

A command run from a supervisor's session is the supervisor's: the
session's `SWITCHBOARD_RECORD_ID` names it. It may run the read-only
commands, `take`, `queue` and `subscriptions` on its own project, and
`decide`, `park`, `resume --no-rerun`, `close`, `subscribe` and
`unsubscribe` on its own project's tickets. A plain
`resume` (which reruns, a paid run) needs `rerun` in `decides`.
Everything else is refused with exit 1: `run`, `restart` and `runner`
(each unless `may` lists it), `worktrees <path>` and `--migrate`,
`supervisor --fresh`, `--resume` and `--kill`, and any other project's
tickets. This is a guard against mistakes, not
a boundary: an agent can unset the variable.

## Answering decisions

A decision is a question Dispatch has recorded for the owner. The
owner decides which kinds you may answer; absent that, answer none and
report them instead. The kinds you will see:

| Decision | Options | What it means |
|---|---|---|
| `lanes` | lane names, comma-separated (`api,web`), or `park` | which repositories the ticket touches; multi-lane projects ask after the investigator's notes, single-lane ones pick their one lane |
| `finalize` | `finalize`, `revise`, `park` | the plan's review has converged or hit its cap; approve the plan for implementation, or `revise --note "<objection>"` (or `--file <path>`) to send the owner's objection to the planner as the next round, after which the reviewer re-reads and `finalize` is asked again. `revise` without a note exits 64. `--file` refuses the ticket's secret artifacts; what the file holds is saved on the decision and sent to the planner, so it must hold nothing secret |
| `paused` | `continue`, `park` | the plan review stopped before converging (a reviewer objected past its cap) |
| `review-code` | `fix`, `accept`, `park` | code reviewers found points; start an implementer on them, take the branch as is, or stop |
| `review-cap` | `accept`, `more`, `park` | the review rounds hit their cap with points still open; `more` is one more fix and review pass |
| `message` | `rewrite`, `accept`, `park`, or `accept`, `park` after a rewrite failed | a review folded its fixes and a folded commit's message names something neither the commit nor the tree has; `rewrite` starts an agent that rewords the message with the tree unchanged, `accept` keeps it as written |
| `inspect` | `proceed`, `rerun`, `park` | the owner's look at a branch before it goes anywhere; `rerun --note "<what to change>"` sends it back to the implementer with the note |
| `rerun` | `rerun`, `park`, and `check` when tests failed, or `keep` when a review's fold or squash failed | an attempt failed (no result, dirty tree, crash, failing tests, a fold that cannot apply); `rerun` is a fresh attempt, `check` runs the same tests again on the same commit, `keep` completes the stage with the history as it is |
| `pr` | `recheck`, `park` | no pull request was found for the branch, or it needs attention; `recheck` after the owner fixed it |
| `refresh` | `recheck`, `park` | the branch is behind its base and the rebase conflicts, with no rebaser left to try, or the worktree is mid-rebase or off its branch; `recheck` after the owner rebased, finished, aborted or checked it out by hand |
| `merge` | `park` | a confirmation: Dispatch watches the provider and closes the ticket itself when the PR merges; it cannot be answered by hand |

A supervisor's `decide` on a decision whose name is not in its
`decides` is refused with exit 1: the refusal is saved on the decision
(`show` lists it under `refused:`), logged as a `refused` event, and
the decision still waits on the owner.

```
dispatch decide 3e0dcacd d2 proceed
dispatch decide 3e0dcacd d2 rerun --note "The grid still overflows at 50 rows; see the issue's second screenshot."
dispatch decide 314cb7a1 d1 accept
dispatch decide 3e0dcacd d4 revise --note "The owner says step 3 deletes data without a backup."
```

An answer takes effect on the runner's next pass, within a second or
two. `park` as an answer stops the ticket at that question: it stops
the ticket's work, kills its agents, and leaves the record for the
owner, who can `resume` it. It also withdraws the ticket's other open
questions; they read `cancelled` and cannot be answered. To stop a
ticket whose open question is about something else, or that has none,
use `dispatch park` (below), not an answer.

Two things never to do: park a ticket at `merge`, by answering its
`merge` decision or by `dispatch park` (either abandons a PR that is
about to land), and answer `finalize`, `review-code` or `review-cap`
for a project you have not been told to approve plans or code on.
Those spend money and change branches. `revise` on `finalize` spends a
planner turn and a reviewer turn; send it only with an objection the
owner gave you, in the note.

## Parking

```
dispatch park 314cb7a1 --reason "the PR was opened mid-rebase"
```

This is how to stop a ticket now. It writes the intent to park, with
every open question withdrawn, and the runner finishes it on its next
pass: the running attempts are cancelled with the reason, their agents
and checks killed, and the ticket reads `parked`. Without `--reason`
the reason is "parked by hand". With no runner up the ticket stays
`parking` until one starts.

To know it has parked, `dispatch wait <ticket> --for stage` exits 3 at
the park, or watch `dispatch events --ticket <id> --follow` for
`parked`. Not `--for any`: it can return on the first cancelled attempt's
`attempt-ended`, before the ticket reads parked.

A closing or closed ticket cannot be parked (exit 64), and one already
parking or parked is refused (exit 1). A ticket at `merge` is not
parked without the owner's say, for the reason above.

## Resuming

```
dispatch resume 314cb7a1
```

A parked ticket goes back to active and continues from its stage. Use
it after the owner has fixed whatever the park reason named. The
resume is the answer for what the park cancelled mid-run: each such
attempt runs again with no question, recorded as a `rerun` answered by
`resume`, and spends an agent run. The resumed ticket still asks
`rerun`, under a new id, for each attempt that failed, for each that
an earlier park cancelled, and for each that was waiting on a question
when the park came (a converged review is not thrown away unasked);
answer those to go on. `dispatch resume <ticket> --no-rerun` reruns
nothing and asks about every one. Resuming a ticket parked for a
reason you do not understand is the owner's call.

## Closing

```
dispatch close 314cb7a1 --reason "fixed by #320"
```

The ticket's worktrees are removed and it is closed for good; the
branches and the record stay (`close` lists them); taking the issue
again deletes an unmoved one and asks about one with commits. It is
refused while anything of the ticket runs (park it first) or a tree
has uncommitted changes; the refusal names them. Closing is the
owner's call unless you were told to close that ticket.

A closed ticket's evidence directories stay for the pipeline's
`evidence_keep_days` (30 by default) and are then removed by the
runner. `--drop-evidence` removes them as the close finishes, or at
once on a ticket already closed; it is the owner's call too, since
screenshots are often what they keep to look back at.

## Where things live

Dispatch's data directory is `$DISPATCH_DATA_DIR`, by default
`~/Library/Application Support/Dispatch` (the `Data:` line of
`dispatch` with no arguments prints it). Under it:

- `pipelines/<project>.toml` is a project's pipeline: its
  repositories, lanes, operators, stages, gates and policy.
  `pipelines/<project>.pr.toml`, when present, is the pipeline that
  reviews other people's pull requests for the same project.
- `tickets/<id>.json` is a ticket's record and `tickets/<id>/` its
  artifacts, including `pipeline.toml`, the ticket's own frozen copy
  of the pipeline it was taken under.
- `projects/<project>.json` holds the queue and the supervisor's
  record (its session, the ones it replaced, its workspace).
- `projects/<project>/supervisor/` holds the supervisor's `seed.md`,
  its `handoff.md`, and each earlier hand-off as
  `handoff.<yyyymmdd-hhmmss>.md`. Its workspace is
  `supervisor-<project>` under the worktrees root.
- `events.jsonl` is the event log `dispatch events` reads, one JSON
  object per line, appended at every ticket write.
- `runner.json` is the runner's status after its last pass, which
  `dispatch health` reads.

Tickets, queues, worktrees and those two files are hands-off: the commands are the
whole interface to them. Pipeline files are the owner's, and the
owner may delegate them to you; the next section says how.

## Managing a pipeline file, when the owner asks

[Writing a pipeline file](dispatch-pipeline-guide.md) builds a file up
step by step and says what each key buys, what it costs, and what goes
wrong without it. This section is what to know while editing one.

Only when the owner has said so for a named project. Edit
`pipelines/<project>.toml` in place with an ordinary editor or `sed`;
Dispatch itself needs no restart. Whether a running ticket sees the
change depends on the key:

- `[policy] slots`, `waiting_on_me` and `min_free_gb` are read from
  the live file on every pass, for every ticket of the project.
  Raising `slots` lets the next waiting ticket start within a second
  or two and `dispatch status` shows the new limit at once. When a
  project has a `.pr.toml` as well, its `[policy]` counts do not
  apply: the project's `.toml` governs both files' tickets, and
  `status` shows one line per project for that reason. `refresh` is
  not read live: it is copied into a ticket at take like the rest of
  the policy.
- A code review stage's `style_rounds` is read from the live file (the
  `.pr.toml` for a ticket from a pull request) at every round, so an
  edit reaches a running review.
- Everything else (lanes, `setup`, gates, operators, stages, prompts,
  the `decisions` dials) is copied into a ticket when it is taken.
  Tickets already running keep their copy until they are restarted, so
  a fix to a lane's `setup` or gate command reaches only tickets taken
  after it and tickets restarted after it. To apply such a fix to a
  ticket that has already failed on the old command: fix the pipeline
  (when the owner delegated that), run `dispatch restart <ticket>`, then
  answer the `rerun` question `check` to run the new checks on the same
  work (or `rerun` for a fresh agent). Answering `check` without the
  restart runs the old copy. Name a stage (`dispatch restart <ticket>
  plan`) to go back to it and discard the later work; the branches go
  back to where they were as the ticket entered that stage. A ticket
  from pull requests goes back only when no branch moved since it
  entered that stage, since someone else's branches are never reset. A
  restart launches no agent by itself unless it gave `--note`, which
  reruns the stage's agent with the note (never `check`) and tells it
  where the previous attempt's notes are: `dispatch restart <ticket> try
  --note "seeded; scenarios 3-9 untried"` sends the tester back with
  what to cover. A note is refused at a stage without an agent or one
  the ticket never ran. If the ticket was parked, or its deploy wrote a
  secret, the restart goes through `deploy` again first, behind a
  `rerun` question; the note waits for the tester.
- A lane's `setup` runs once per worktree, before the lane's first
  agent, and again after a restart that changed it: before the next
  agent or the checks a `check` answer starts.
- A `take` or `dispatch restart` refused with "is a code review stage
  and may not hold" means the live file names a resource in `needs` on
  a code review stage. Remove that stage's `needs` (when the owner
  delegated the file). Tickets already running under an older copy are
  left alone and run on.
- `env = ["<set>"]` on an operator or a stage grants Switchboard
  environment sets: named variables and secrets the owner keeps in
  Switchboard, never in the pipeline file or the repository. A
  session gets its operator's sets, then its stage's, and its prompt
  ends with the one sentence on running commands through
  `switchboard-env exec -- <command>`. Only `claude` operators take
  `env`, and only agent stages with a `claude` operator, code review
  stages and command-gate stages. On a command gate, `env` makes the
  gate run under `switchboard-env exec --` with the runner's own
  grants, which the owner makes with `switchboard-env grant --runner
  <set>`; the gate fails with "this runner has no Switchboard record"
  when the runner was not started from the Dispatch overview. Add or
  change `env` only as the owner says: which sets exist and what is
  granted to whom is theirs to decide, and you neither create sets nor
  grant them. Like the rest of a stage, `env` reaches tickets taken or
  restarted after the edit.

A `tried` confirmation (or any `confirm = true` gate in the same hold
run as the agent stage before it) shows the first line of text of each
notes file, so a tester that could not test anything says so on its
notes' first line (`Result: nothing could be tested`). The owner can
then answer `rerun` with a note: the tester runs again on the same
deploy, its services stopped and started with a fresh `before`, while
the stack stays held. For a supervisor to answer it, `decides` names
the gate's decision (`tried`), not `rerun`. A running supervisor picks
up the seed's paragraph on a gate's answers only when it is started
fresh.

When an implement stage holds a deployable stack (`needs` on
`implement`), its agents may deploy their lane to it while they work.
Those deploys are throwaway: the pipeline's own deploy stage deploys
the branch again, and its commit is what `tried` shows. A deploy
wrapper that asks for MFA cannot be answered in an agent's pane, so the
owner keeps its session alive by running it once by hand. A
`dispatch runner stop` or `restart` refused while a deploy runs covers
only Dispatch's own deploy stage: an implementer's deploy runs in its
Switchboard session, which a runner restart does not touch.

A `lane:<name>` deploy with `without_lane = "base"` still runs for a
ticket that did not choose the lane: it deploys the lane's base branch
from the project's shared base tree, holds the stage's resource as any
deploy does, and its commit is what the tester and `tried` read. A
deploy whose stack already holds the commit must still exit 0, so make
the deploy task idempotent rather than wrapping it in the gate.

A stage that gives a tester something secret (a test user's
credentials from a gate-only `try-setup` that writes
`{ name = "personas", secret = true }`) reaches the tester's prompt as
`{inputs.personas}`, a path, and as `$DISPATCH_INPUT_PERSONAS` in its
session; named as `{inputs.try-setup.personas}`, it arrives as
`$DISPATCH_INPUT_TRY_SETUP_PERSONAS`. The file lives only while the
ticket holds the stage's resource; it is deleted when the hold is
released, the ticket parks or closes. Say so in the tester's stage
prompt, and say that the tester should not look for cloud credentials:
it has none unless its operator or stage names sets in `env`, and then
it reaches them only by running a command through `switchboard-env
exec`, as its prompt says, never by reading credential files or
profiles. Say also that it uses the file (passes its path to a probe or
a seeding script) without printing its contents, and that it prints no
variable `switchboard-env` gives a command: whatever an agent prints
stays in its transcript and in Switchboard's scrollback.

A stage may keep files it produced beside its notes, such as
screenshots, in one evidence directory: `writes = ["notes", { name =
"evidence", dir = true }]`. Dispatch makes `<attempt dir>/evidence/`
(mode 0700) before the launch, gives the agent its path as `{evidence}`
in the prompt and as `$DISPATCH_WRITES_EVIDENCE` in its session, lists
the regular files in it when the agent finishes (links, FIFOs and
anything over `evidence_file_mb`, or past `evidence_attempt_mb` in
total, are not listed), and shows the count on the `tried` question.
`dispatch evidence <ticket>` prints each file's path. No other stage
reads the directory (`{inputs.<stage>.evidence}` is refused), a
`codex` operator cannot keep one, and a gate-only command gets the
same variable.

What reaches the directory unasked, measured in spike 21:

- the Write tool: yes, under the allow rule Dispatch already passes for
  the attempt directory. Tell the tester to write its files with the
  Write tool;
- Bash `cp`, `mv` and `mkdir` into it: no, they ask for permission, and
  an added directory does not change that. Do not tell the tester to
  copy files in;
- a Playwright screenshot: only when the tester's Playwright MCP server
  runs with `--allow-unrestricted-file-access` and the tester passes
  `{evidence}/<file>.png` as `browser_take_screenshot`'s `filename`.
  Without the flag the server refuses any path outside the agent's
  cwd. The flag also lets the browser read any local file and open
  `file://` URLs, so it goes in the tester operator's own `--mcp-config`
  only when the owner says so:

  ```toml
  [operators.tester]
  kind = "claude"
  args = ["--mcp-config", "/path/to/tester-mcp.json"]
  # tester-mcp.json:
  # {"mcpServers":{"playwright":{"command":"npx",
  #   "args":["-y","@playwright/mcp@0.0.82","--allow-unrestricted-file-access"]}}}
  ```

Before saving, read the file back: a pipeline that does not parse is
refused at the next `take` with the parser's reason, and `status`
falls back to each ticket's copy for the limits. Never edit a ticket's
`tickets/<id>/pipeline.toml`.

Two traps that read as plain test failures:

- Gates and `setup` run with the runner's `PATH`, so a tool installed
  globally on the machine can stand in for one the lane's `setup` did
  not install. The result is a lint that passes from a global copy and
  a test runner that fails with `command not found`. When a Python
  lane keeps its tools in a dependency group, both `setup` and the gate
  need that group (`uv sync --all-groups`, `uv run --all-groups ...`).
- A gate that exits 127 is that case, and the decision's text says so.
  `rerun` and `check` cannot help; fix the pipeline and tell the owner
  which tickets were taken under the broken one.

## What not to do

- Do not edit or delete anything under the data directory by hand:
  tickets, queues, worktrees, `events.jsonl`, `runner.json`. Edit a
  pipeline file only when the owner has delegated that project to you,
  as above.
- Do not kill processes by pattern (`pkill claude`, `pkill -f dispatch`).
  The ticket's agents are Switchboard's and the runner is the owner's;
  `dispatch park` stops a ticket's work properly.
- Do not push to a ticket's branch while it is at `merge`. Dispatch
  watches the PR at the head it recorded; a push from outside reads as
  someone else's change.
- Do not delete or recreate a ticket by hand. There is no subcommand
  for it yet; ask the owner.
- Do not take an issue to "see what happens". Every ticket runs real
  agents and costs money from its first stage.
- Do not take the same issue on two projects, or an issue from a
  repository the project does not list.
- Do not run `dispatch run`, `dispatch worktrees`, or anything with
  `--migrate`.
- Do not comment on, label or close GitHub issues on Dispatch's
  behalf. Dispatch never does, and the owner tracks state in Dispatch,
  not in labels.

## A grooming session, end to end

1. `dispatch status` for the load per project and anything parked.
2. `dispatch decisions`; report what waits, answer only what you are
   allowed to.
3. For each groomed issue ready to go: confirm the body is complete on
   GitHub, then `dispatch take <project> <n>` and record the ticket
   id against the issue in your own notes.
4. `dispatch queue <project>` and reorder so the most urgent waiting
   ticket is first.
5. Report: tickets taken (id, issue, title), queue order, decisions
   pending for the owner with their questions, anything refused and
   why.
