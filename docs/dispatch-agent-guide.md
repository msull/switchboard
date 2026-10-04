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
dispatch decide <ticket> <decision> <answer> [--note <text>]
dispatch queue <project>                          the project's queue in order
dispatch queue <project> <ticket>...              reorder it
dispatch resume <ticket>                          a parked ticket back to active
dispatch close <ticket> [--reason <text>]         a ticket closed, its trees removed (its branches are kept; close lists them)
dispatch health [--timeout <secs>] [--stale <secs>] [--json]   is the runner alive and getting on; run it first
dispatch show <ticket> [--json]                   one ticket: stage, lanes, attempts, rounds, decisions, files
dispatch events [--since <seq>] [--follow] [--ticket <id>]... [--project <name>] [--json]
dispatch wait <ticket> [--for decision|stage|pr|closed|any] [--timeout <secs>] [--json]
dispatch report <ticket> [--json]                 how a ticket went: stage time, review points, fix passes, size
dispatch report --project <name> [--since YYYY-MM-DD] [--json]
dispatch tail <ticket> [--lines N]                what the ticket's running agents show
```

`dispatch run` and `dispatch worktrees` are the owner's: never run
them. The runner is already up; if a command says it cannot reach the
socket, stop and report that instead of starting one.

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
hour. A ticket with an open attempt the runner has not got on with for
longer than `--stale` is named. Exit 1 means something is wrong; read
the lines and report them. Do not restart the runner yourself.

**Show.** `dispatch show X` prints the header (id, project, source,
title), `stage <name> (i/n)` and the state; each lane with its branch,
worktree and the base, head and last pushed head (seven characters);
the attempts grouped by stage, each with its state and reason, head,
the head its checks ran at and a history rewrite, with any PR and the
code review rounds (`r<n> <state> open <k>`) beneath it; the pending
decisions with their options and the exact `dispatch decide` line; and
the files to read next: the plan, the latest round's findings, the
code review summary, the notes, and the PR's url and head. Read those
files rather than guessing what they say.

**Events.** Every ticket write that changes something appends one
line per change to `events.jsonl` in the data directory: `taken`,
`stage` (forward) and `sent-back`, `attempt-started` and
`attempt-ended` (with its state and reason), `decision` (its name,
question and options), `answered` and `decision-cancelled`, `pr` and
`pr-checks`, `pushed` and `refreshed`, `rewrite`, `round` (a code
review round and its open points), `nudged` (a line typed into an
agent's session after it stopped with a dirty tree), `parking`,
`parked`, `resumed`, `closing`, `closed`, and `void`. The human line is
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

**Wait.** `dispatch wait X --for decision` blocks until the ticket asks
something and prints that `decision` event with the `dispatch decide`
line to answer it. If a decision already waits, it returns at once,
and a decision answered before the wait reads it is passed over. A
ticket already closed ends any wait at once with exit 3 (exit 0 for
`--for closed`), and one already parked does too, except that
`--for any` waits for its resume.
`--for stage` waits for a stage move either way, `--for pr` for a PR
bound to an attempt, `--for closed` for the close, and `--for any`
(the default) for the next event of any kind. Every match is checked
against the record first, so a withdrawn event is never returned.

**Report.** `dispatch report X` gives the time per stage with the time
its decisions waited on the owner apart, the plan's size, the plan
review's rounds and points, the code review's rounds and points (each
point once, however many rounds it stayed open, split by reviewer),
fix passes, rebases (at least: a lane keeps only its last clean
bring-up), and the commits and diff at the PR. Cost and
turns read `not recorded`: Dispatch does not see them. A count marked
incomplete is missing a round's file.

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
- `paused`, `review-code`, `review-cap`, `finalize`: only with the
  owner's say for that project.

**Exit codes.** Every command exits 0 when it did what it says and 1
with a reason when it was refused or failed. `wait` exits 2 when its
`--timeout` passed and 3 when the ticket parked or closed, so what it
waited for will not come. A command line Dispatch cannot read exits
64 with the usage.

## Ordering the queue

```
dispatch queue <project> ed9b2305 3e0dcacd baea8dbe
```

List every ticket id in the order wanted. A ticket not in the project's
queue, or a missing one, is refused and nothing changes. Running
tickets keep running; the order only decides which waiting ticket
starts next when a slot frees. This is the one lever a grooming agent
should use freely.

## Answering decisions

A decision is a question Dispatch has recorded for the owner. The
owner decides which kinds you may answer; absent that, answer none and
report them instead. The kinds you will see:

| Decision | Options | What it means |
|---|---|---|
| `lanes` | lane names, comma-separated (`api,web`), or `park` | which repositories the ticket touches; multi-lane projects ask after the investigator's notes, single-lane ones pick their one lane |
| `finalize` | `finalize`, `park` | the plan's review has converged; approve the plan for implementation |
| `paused` | `continue`, `park` | the plan review stopped before converging (a reviewer objected past its cap) |
| `review-code` | `fix`, `accept`, `park` | code reviewers found points; start an implementer on them, take the branch as is, or stop |
| `review-cap` | `accept`, `more`, `park` | the review rounds hit their cap with points still open; `more` is one more fix and review pass |
| `inspect` | `proceed`, `rerun`, `park` | the owner's look at a branch before it goes anywhere; `rerun --note "<what to change>"` sends it back to the implementer with the note |
| `rerun` | `rerun`, `park`, and `check` when tests failed, or `keep` when a review's fold or squash failed | an attempt failed (no result, dirty tree, crash, failing tests, a fold that cannot apply); `rerun` is a fresh attempt, `check` runs the same tests again on the same commit, `keep` completes the stage with the history as it is |
| `pr` | `recheck`, `park` | no pull request was found for the branch, or it needs attention; `recheck` after the owner fixed it |
| `refresh` | `recheck`, `park` | the branch is behind its base and the rebase conflicts, with no rebaser left to try; `recheck` after the owner rebased the worktree by hand |
| `merge` | `park` | a confirmation: Dispatch watches the provider and closes the ticket itself when the PR merges; it cannot be answered by hand |

```
dispatch decide 3e0dcacd d2 proceed
dispatch decide 3e0dcacd d2 rerun --note "The grid still overflows at 50 rows; see the issue's second screenshot."
dispatch decide 314cb7a1 d1 accept
```

An answer takes effect on the runner's next pass, within a second or
two. `park` is always safe: it stops the ticket's work, kills its
agents, and leaves the record for the owner, who can `resume` it. It
also withdraws the ticket's other open questions; they read
`cancelled` and cannot be answered.

Two things never to do: answer a `merge` decision (its only option is
`park`, and parking a ticket at merge abandons a PR that is about to
land), and answer `finalize`, `review-code` or `review-cap` for a
project you have not been told to approve plans or code on. Those
spend money and change branches.

## Resuming

```
dispatch resume 314cb7a1
```

A parked ticket goes back to active and continues from its stage. Use
it after the owner has fixed whatever the park reason named. A resumed
ticket asks `rerun` again, under a new id, for each attempt that failed
or was cancelled by the park; answer it to go on. Resuming a ticket
parked for a reason you do not understand is the owner's call.

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
- `projects/<project>.json` holds the queue.
- `events.jsonl` is the event log `dispatch events` reads, one JSON
  object per line, appended at every ticket write.
- `runner.json` is the runner's status after its last pass, which
  `dispatch health` reads.

Tickets, queues, worktrees and those two files are hands-off: the commands are the
whole interface to them. Pipeline files are the owner's, and the
owner may delegate them to you; the next section says how.

## Managing a pipeline file, when the owner asks

Only when the owner has said so for a named project. Edit
`pipelines/<project>.toml` in place with an ordinary editor or `sed`;
nothing needs restarting. What a change reaches depends on the key:

- `[policy] slots`, `waiting_on_me`, `min_free_gb` and `refresh` are
  read from the live file on every pass, for every ticket of the
  project. Raising `slots` lets
  the next waiting ticket start within a second or two and
  `dispatch status` shows the new limit at once. When a project has a
  `.pr.toml` as well, its `[policy]` counts do not apply: the
  project's `.toml` governs both files' tickets, and `status` shows
  one line per project for that reason.
- Everything else (lanes, `setup`, gates, operators, stages, prompts,
  the `decisions` dials) is copied into a ticket when it is taken.
  Tickets already running keep their copy to the end, so a fix to a
  lane's `setup` or gate command reaches only tickets taken after it.
  To apply such a fix to a ticket that has already failed on the old
  command, the owner has to retake it; say so in your report rather
  than answering `rerun` or `check`, which both run the old copy.
- A lane's `setup` runs once per worktree, when it is cut. It is not
  run again later, whatever the file says now.

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
  `park` stops a ticket's work properly.
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
