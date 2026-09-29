# Dispatch

A system that sits on top of Switchboard the way Switchboard sits on top
of tmux and Prompt Box. Nothing here is built; this is the design to
argue with, with three real pipelines written by hand to test the
vocabulary before any code. Decisions I made without you are collected
at the end.

The idea in one line, moved up a level: **tickets are data, sessions are
a cache.** Switchboard keeps sessions and files durable on top of tmux
and shows which agents are waiting. Dispatch keeps tickets, plans,
decisions and operators durable on top of Switchboard, and shows which
tickets are waiting on you.

## The boundary

Switchboard stays the console and the process host. Dispatch is the
scheduler and the policy. Dispatch knows nothing about tmux,
transcripts, hooks, or panes; it only ever says "make a project at this
root", "start a plan review with this definition", "send this prompt",
and asks "which sessions are waiting, what did the reviewer conclude,
does this file exist". Switchboard knows nothing about tickets,
GitHub, worktrees, or what "ready" means.

That mirrors Switchboard and Prompt Box: Prompt Box knows nothing about
sessions, Switchboard nothing about the voice runtime, and the join is
one small port (`PromptSink`). The join here is a **control port**: a
Unix socket beside the wake socket that accepts actions and answers
read-model queries (see "Control port"). Dispatch is a separate process
talking over it. One process owns the data directory, the flock story
stays simple, and you can always drop into the real window and see
every card, because Dispatch draws nothing of its own.

Dispatch's own state lives in its own data directory
(`~/Library/Application Support/Dispatch`), keyed by ticket. It never
writes into a repository's own dot-directories; only the agents commit
code. Same hard rule as Switchboard.

## Objects

- **Ticket.** The unit of work. Where it comes from is per project (a
  GitHub issue with a label, a line in a task file, a command you run).
  The record holds the source, the lanes, the stage it is in, the
  decisions taken, a log, and the budget spent.
- **Lane.** One repository the ticket touches: a worktree, a branch,
  and the Switchboard project made for it. A ticket has one or more
  lanes; all of a ticket's lanes live in one Switchboard workspace named
  for the ticket, so the workspace privacy rule keeps tickets out of
  each other's view.
- **Pipeline.** Per project, a list of stages. A stage names an
  operator, a prompt, which lanes it runs on (each, or joined), and a
  gate that says when it is done.
- **Gate.** How a stage is known to be done. Three kinds cover
  everything in the three pipelines below:
  - *command*: a command exits zero (tests, lint, a deploy script,
    a smoke check);
  - *external*: something outside says so (PR checks green, a review
    run finalized, a file exists and has settled);
  - *human*: a decision surfaced to you, or auto-resolved once you
    have lowered the dial for that gate.
- **Operator.** A named role: guidance, an agent kind and model class,
  a tool allowance, a budget, and a definition of done. Switchboard's
  `WorkflowDefinition` is already most of a reviewer operator, so an
  operator is a definition plus policy, and the review stage of every
  pipeline below is literally a Switchboard plan review with a named
  definition.
- **Decision.** A point where a human would normally answer: which
  lanes a ticket needs, whether a reviewer's objection is right,
  whether to spend past a budget, whether a deploy may go. Each
  carries the options, the operator's recommendation, a confidence,
  and the policy kind that governs it. Every decision is logged
  whether you took it or Dispatch did.
- **Resource.** Something only one ticket can hold at a time: a shared
  test environment, a deploy lock. A stage that needs one waits its
  turn. Raising a resource's count is how parallelism grows later.
- **Queue.** An ordered list of runnable tickets per project, reordered
  by hand. Dispatch takes from the top whenever a slot is free.

## The pipeline file

One file per project, TOML, in Dispatch's data directory (never in the
repository). The three below are written against the real projects and
are the test of this vocabulary: if it expresses all three cleanly the
abstraction is right, and if it does not, this is the cheapest place
to find out.

```toml
[project]
name = "..."                 # Switchboard project name
root = "/path"               # the repository, or the workspace repo
switchboard_workspace = "..." # Dispatch makes one workspace per ticket under this name

[source]                     # where tickets come from
kind = "github" | "task-file" | "manual"

[[lanes]]                    # repositories a ticket may touch
name = "..."
path = "relative/to/root"    # "." for a single-repo project
worktrees = "/path/for/worktrees"

[[resources]]
name = "..."
count = 1

[operators.<name>]
kind = "claude" | "codex"
guidance = "path or inline"
definition = "..."           # a Switchboard workflow definition, for review stages
budget_usd = 0.0             # per ticket; 0 means the project default

[[stages]]
name = "..."
operator = "..."
lanes = "each" | "joined" | ["front"]
prompt = "..."               # templates: {issue} {plan} {lane} {branch} {notes}
gate = { kind = "command", run = "...", in = "lane" | "root" }
     | { kind = "external", check = "pr-checks" | "review-finalized" | "file-settled", path = "..." }
     | { kind = "human", decision = "..." }
needs = ["resource name"]    # held for the stage's duration

[policy]
slots = 1                    # tickets in flight in this project
waiting_on_me = 2            # tickets allowed to sit on a human gate
decisions = { lanes = "ask", review_objection = "ask", deploy = "ask", budget = "ask" }
```

`decisions` values are `ask` (surface it), `recommend` (surface it with
the operator's answer preselected, auto-taken after a delay), or
`auto`. Everything starts at `ask`; the dial only ever moves per
decision kind, by you.

### Pipeline: Switchboard

Single repository, GitHub issues, deployable only in the sense of
"merge and rebundle". Ready means a PR with checks green.

```toml
[project]
name = "Switchboard"
root = "/Users/sully/code_repos/personal/switchboard"
switchboard_workspace = "Dispatch · Switchboard"

[source]
kind = "github"
repo = "msull/switchboard"
label = "dispatch"           # marking an issue with it is the handover

[[lanes]]
name = "repo"
path = "."
worktrees = "/Users/sully/code_repos/personal/switchboard-worktrees"

[operators.investigator]
kind = "claude"
guidance = "Read CLAUDE.md, docs/design.md and docs/lessons.md first. Find the code the issue touches, name the files, and say what the on-disk record and rehydration would be before what the pane looks like."

[operators.planner]
kind = "claude"
guidance = "Write the plan the way docs/design.md sections are written: the record, the signals, the actions and effects, the UI, the tests. One commit, README and design status updated, the app runs at every step."

[operators.reviewer]
kind = "codex"
definition = "Built-in review"   # the shipped WorkflowDefinition; a Switchboard-specific one can replace it
guidance = "Hold the plan to the layering in src/lib.rs and the hard rules in CLAUDE.md. No keystroke injection, no writes into a project's dot-directories, additive contract, schema bump for any serialized change."

[operators.implementer]
kind = "claude"
guidance = "Implement the finalized plan on the branch. cargo test --locked, cargo clippy --locked --all-targets -- -D warnings, cargo fmt --all. Commit messages carry no tool attribution. Open a PR with gh."

[[stages]]
name = "investigate"
operator = "investigator"
lanes = "each"
prompt = "Issue #{issue.number}: {issue.title}\n\n{issue.body}\n\nInvestigate and write your findings to {notes}."
gate = { kind = "external", check = "file-settled", path = "{notes}" }

[[stages]]
name = "plan"
operator = "planner"
lanes = "each"
prompt = "Using {notes}, write a plan for issue #{issue.number} to {plan}."
gate = { kind = "external", check = "file-settled", path = "{plan}" }

[[stages]]
name = "review"
operator = "reviewer"
lanes = "each"
# A Switchboard plan review run: StartWorkflow on the plan with the
# operator's definition; done when the run is Finalized. Objections
# the planner refutes are decisions.
gate = { kind = "external", check = "review-finalized" }

[[stages]]
name = "implement"
operator = "implementer"
lanes = "each"
prompt = "The plan at {plan} was reviewed and finalized. Implement it on branch {branch}, then open a PR against main."
gate = { kind = "command", run = "cargo test --locked && cargo clippy --locked --all-targets -- -D warnings && cargo fmt --all -- --check", in = "lane" }

[[stages]]
name = "ready"
operator = "implementer"
lanes = "each"
gate = { kind = "external", check = "pr-checks" }   # CI on Linux is the last machine check

[[stages]]
name = "merge"
lanes = "each"
gate = { kind = "human", decision = "merge" }       # you merge and run ./scripts/bundle.sh

[policy]
slots = 1
waiting_on_me = 2
decisions = { lanes = "auto", review_objection = "ask", merge = "ask", budget = "ask" }
```

`lanes = "auto"` is safe here because there is only one lane; the
decision is trivial and asking would be noise.

### Pipeline: PTA

No repository remote, no deploy, no CI. A local git repository of
guides, templates and small scripts run through `inv`. Tickets are
lines of `task-list.md`, and most of them need you (a phone call, a
vote). Dispatch's use is the ones that are writing: a draft, a flyer, a
guide skeleton. Ready means a draft in the Drafts folder or a rendered
PDF for you to look at. Nothing is ever sent.

```toml
[project]
name = "PTA"
root = "/Users/sully/Documents/SBPTA"
switchboard_workspace = "Dispatch · PTA"

[source]
kind = "task-file"
path = "task-list.md"
marker = "@dispatch"         # a task line carrying this is handed over; Dispatch never edits the file

[[lanes]]
name = "kb"
path = "."
worktrees = ""               # no worktrees: one lane, edits in place on a branch

[operators.writer]
kind = "claude"
guidance = "Follow CLAUDE.md: extract only facts present in the sources, mark the rest _TBD_ with an Open Questions bullet, cite sources at the bottom, keep file names lowercase-hyphenated, never rename or move files."

[operators.reviewer]
kind = "codex"
definition = "PTA review"
guidance = "Check every fact against the cited source. Anything not traceable to a source is an objection. Names and contacts must match notable-people.md."

[[stages]]
name = "draft"
operator = "writer"
lanes = "each"
prompt = "Task: {task.text}\n\nContext: {task.context}\n\nProduce the draft the task asks for (a guide, a template, a flyer via `inv`, or a mail draft via `inv mail-draft`). Write what you produced and where to {notes}."
gate = { kind = "external", check = "file-settled", path = "{notes}" }

[[stages]]
name = "review"
operator = "reviewer"
lanes = "each"
gate = { kind = "external", check = "review-finalized" }

[[stages]]
name = "render"
operator = "writer"
lanes = "each"
prompt = "Apply the finalized review. If the task is a flyer or handout, render it with the matching `inv` task; if it is mail, save it with `inv mail-draft`. Never send."
gate = { kind = "command", run = "git diff --quiet || git commit -am 'Dispatch: {task.text}'", in = "lane" }

[[stages]]
name = "ready"
lanes = "each"
gate = { kind = "human", decision = "publish" }     # you send the mail or post the PDF

[policy]
slots = 1
waiting_on_me = 3
decisions = { lanes = "auto", review_objection = "ask", publish = "ask", budget = "ask" }
```

What this pipeline showed: a source that is a file rather than an
issue tracker, a project with no worktrees, and a "ready" that is a
human act with no machine gate before it. The `marker` on a task line
is the only thing Dispatch reads from the project as configuration,
which matches Switchboard's rule for `.switchboard/project.json`.

### Pipeline: Delta

The hard case, and the one the vocabulary was shaped for. Facts from
`~/code_repos/delta` (its `CLAUDE.md`, `guides/ENVIRONMENTS_GUIDE.md`,
`guides/COLLABORATION_GUIDE.md`, the backend `tasks/`):

- The root is a planning workspace (`k3systems/delta-workspace`), and
  the issues live there with a label scheme (`type:`, `area:`,
  `priority:`, `status:`). The application repositories are separate
  git repositories nested under it: `delta-backend` (integration branch
  `main`), `delta-frontend` (`dev`; `master` is production),
  `delta-snp` (`master`). Bitbucket is the origin; GitHub mirrors
  exist. PRs go through `tools/bb.py`, which squashes on merge.
- "Ready for the test environment" pre-merge means `sully-dev`: one
  personal backend stack, deployed from any branch by
  `aws-vault exec -n deltadev -- uv run inv deploy -f` from
  `delta-backend` with that env linked. Last deploy wins. The
  frontends have no per-branch environment; a branch is tried against
  `sully-dev` with `npm run link-env` and a local `npm start` (which
  the existing `webserver` service entry already runs). Shared `dev`
  deploys itself on merge to `main` and is post-merge, so it is not a
  gate.
- Machine checks: backend `uv run inv lint` and `uv run inv pytest`;
  frontend `npm ci --legacy-peer-deps` then
  `CI=true npm test -- --watchAll=false`; snp `npm test`. Bitbucket
  Pipelines runs tests on PRs but has no lint gate, and the GitHub
  mirrors have no CI, so `pr-checks` reads Bitbucket through `bb.py
  watch --branch` rather than `gh`.
- A PR is ready only with a self-contained body, an updated backend
  `CHANGELOG.md` "Unreleased" entry, a What's New entry for every
  user-visible frontend change, tests listed in the body, and no
  session-link attribution of any kind.
- Nothing in the repos locks an environment. Two tickets deploying to
  `sully-dev` would overwrite each other, which is exactly what a
  resource with `count = 1` prevents.

```toml
[project]
name = "Delta"
root = "/Users/sully/code_repos/delta"
switchboard_workspace = "Dispatch · Delta"

[source]
kind = "github"
repo = "k3systems/delta-workspace"
label = "dispatch"
# area: labels are the operator's hint for the lanes decision.
lane_hints = { "area:backend" = "backend", "area:frontend" = "frontend", "area:snp" = "snp" }

[[lanes]]
name = "backend"
path = "delta-backend"
base = "main"
worktrees = "/Users/sully/code_repos/delta-worktrees/backend"
setup = "uv sync"

[[lanes]]
name = "frontend"
path = "delta-frontend"
base = "dev"
worktrees = "/Users/sully/code_repos/delta-worktrees/frontend"
setup = "npm ci --legacy-peer-deps"

[[lanes]]
name = "snp"
path = "delta-snp"
base = "master"
worktrees = "/Users/sully/code_repos/delta-worktrees/snp"
setup = "npm ci --legacy-peer-deps"

[[resources]]
name = "sully-dev"
count = 1

[operators.investigator]
kind = "claude"
guidance = "Read the root CLAUDE.md, guides/ENVIRONMENTS_GUIDE.md and the lane's CLAUDE.md. Name the files, the endpoints and the screens the issue touches. Say which lanes it needs and why; that is a decision the human sees."

[operators.planner]
kind = "claude"
guidance = "Write the plan as plans/YYYYMMDD-name.md is written: per lane, the change, the tests, the CHANGELOG or What's New entry, and how it is tried on sully-dev."

[operators.reviewer]
kind = "codex"
definition = "Delta review"
guidance = "Hold the plan to COLLABORATION_GUIDE: branch naming, self-contained PR body, changelog entries, 100% coverage on new frontend components, no attribution lines. Check the backend and frontend halves agree on the API."

[operators.implementer]
kind = "claude"
guidance = "Implement the lane's part of the finalized plan on branch {branch}. Run the lane's checks before every commit. Never touch another lane's worktree. Never run bb.py promote or any deploy to dev, staging or production."

[operators.tester]
kind = "claude"
guidance = "Deploy the backend branch to sully-dev with the linked-env command, link the frontend to it, and exercise the change with the tools/e2e probes or curl. Write what worked and what did not to {notes}. Never deploy anywhere else."

[[stages]]
name = "investigate"
operator = "investigator"
lanes = ["backend"]           # runs once in the workspace root's context, before lanes exist
prompt = "Issue #{issue.number}: {issue.title}\n\n{issue.body}\n\nInvestigate and write findings, including the lanes needed, to {notes}."
gate = { kind = "external", check = "file-settled", path = "{notes}" }

[[stages]]
name = "lanes"
gate = { kind = "human", decision = "lanes" }   # which worktrees and branches to cut; the label hints preselect

[[stages]]
name = "plan"
operator = "planner"
lanes = "joined"
prompt = "Using {notes}, write one plan covering every lane of ticket #{issue.number} to {plan}."
gate = { kind = "external", check = "file-settled", path = "{plan}" }

[[stages]]
name = "review"
operator = "reviewer"
lanes = "joined"
gate = { kind = "external", check = "review-finalized" }

[[stages]]
name = "implement"
operator = "implementer"
lanes = "each"
prompt = "The plan at {plan} is final. Implement the {lane} part on {branch}."
gate = { kind = "command", in = "lane", run = { backend = "uv run inv lint && uv run inv pytest", frontend = "CI=true npm test -- --watchAll=false", snp = "CI=true npm test" } }

[[stages]]
name = "try"
operator = "tester"
lanes = "joined"
needs = ["sully-dev"]
prompt = "Deploy the backend lane to sully-dev and try the change end to end. Report to {notes}."
gate = { kind = "command", in = "lane:backend", run = "aws-vault exec -n deltadev -- uv run inv deploy -f" }
# The command gate deploys; the human gate after it is where you look.

[[stages]]
name = "tried"
needs = ["sully-dev"]        # still held: what you are looking at must stay deployed
gate = { kind = "human", decision = "tried" }

[[stages]]
name = "ready"
operator = "implementer"
lanes = "each"
prompt = "Open the PR with bb.py create: self-contained body, tests listed, changelog and What's New entries present."
gate = { kind = "external", check = "pr-checks", via = "bb.py watch --branch" }

[[stages]]
name = "merge"
lanes = "each"
gate = { kind = "human", decision = "merge" }   # you run bb.py merge and tools/backup.py

[policy]
slots = 2                    # two tickets in flight; only one can hold sully-dev
waiting_on_me = 2
decisions = { lanes = "recommend", review_objection = "ask", tried = "ask", merge = "ask", budget = "ask" }
```

What this pipeline showed, and what it added to the vocabulary:

- **Lanes with a `base` and a `setup`.** Each repository has its own
  integration branch and install step; the Switchboard pipeline did
  not need either.
- **`joined` stages.** Plan and review happen once per ticket across
  lanes; implementation happens per lane. That distinction did not
  exist until a multi-repo project needed it.
- **A per-lane `run` on a command gate.** The three repositories check
  themselves with three different commands.
- **`lane:backend` as a place to run.** A joined stage still has to
  deploy from one lane.
- **A resource held across two stages.** The deploy and your look at
  it must both hold `sully-dev`, or a second ticket could overwrite
  the stack while you are looking. `needs` on consecutive stages is
  one hold, not two.
- **`lane_hints`.** The issue's `area:` labels are a reasonable
  default answer to the lanes decision, so it is the one decision
  that starts at `recommend` rather than `ask`.
- **The tester operator's guidance carries the danger.** Nothing in
  Dispatch can express "never deploy to production"; the guard is the
  operator's guidance plus the fact that the only deploy command in
  the file targets whatever is linked, which is `sully-dev`. If that
  is not enough, the fix is a Switchboard command record whose
  approval hash pins the exact text, which the Run tab already does.

## Stage semantics

- A stage on `each` lane runs once per lane, in parallel, and the
  ticket advances when every lane's gate passes. A `joined` stage runs
  once with every lane at a known commit.
- Every stage's agent is a fresh Switchboard session (a cloned session
  for the review's planner, as the workflow already does), launched
  through the control port with the ticket's notes file named in the
  prompt. Fresh per stage costs more and keeps each stage honest about
  what is on disk; the notes file is the memory between stages.
- A gate is polled on Switchboard's tick through the control port:
  `file-settled` is the round-file probe the workflow already has,
  `review-finalized` is the run's state, `pr-checks` is `gh pr checks`,
  `command` is a Switchboard command record run in the lane's project
  with its exit code read back.
- Failure is a decision, never a retry. A gate that fails, an agent
  that stalls (Switchboard already marks it), a review at its cap, a
  branch that no longer merges: each becomes a decision with the
  operator's suggested next step. Nothing loops silently.
- Budget is counted per ticket from the transcripts' usage, which
  Switchboard already reads. Crossing an operator's or the project's
  budget is a decision.

## Surfacing

Everything you need to see goes through Switchboard, since the badge,
the rail count, working sets, the controller and the keys already
exist:

- One working set per project, `Dispatch · <project>`, holding the
  queue in order, one card per ticket with its stage and any pending
  decision. Reordering the cards is reordering the queue.
- A pending decision makes the ticket's current session read as
  *waiting on you*, so the Dock count and the rail include it. The
  decision text and its options go into the session's notes, and the
  answer is a line sent to the pane (or a card action once Switchboard
  has a generic "choice" card; not needed for the first slice).
- Every ticket's lanes are ordinary Switchboard projects in the
  ticket's workspace, so all the existing views work on them.

## Control port

The one thing Switchboard has to grow. A Unix socket
`<data dir>/control.sock`, newline-delimited JSON, one request per
line, one reply per line. Two request kinds:

- `{"act": <AppAction as JSON>}`: dispatched on the next frame; the
  reply is `{"ok": true}` or an error. Only a subset of actions is
  accepted at first: `AddProject`, `NewSession`, `SendInput`,
  `KillSession`, `RemoveSession`, `StartWorkflow`, `FinalizeWorkflow`,
  `NewSpace`, `MoveProjectToSpace`, `NewWorkingSet`,
  `AddToWorkingSet`, `PlacePin`.
- `{"ask": "<read model>", ...}`: answered from the core on the next
  frame: `projects`, `sessions {project}`, `session {id}` (record,
  card state, activity, host liveness, last exit), `waiting`,
  `workflow {id}`, `workflows {project}`, `usage {id}`.

Requests are actions, so they enter the deterministic core the same
way clicks do, and the script runner already shows the shape: every
line of `SWITCHBOARD_SCRIPT` is a control request in miniature. The
port needs `AppAction` and a few read models to derive `Serialize` and
`Deserialize`, which is additive.

Trust: the socket is mode 0600 in the data directory, so anything that
can write it can already write the data directory. The port launches
agents on request, which is the same trust the script runner has. The
"never resume automatically" rule holds: Dispatch only starts what it
asked for, and a restart of Switchboard reconciles and launches nothing
Dispatch did not re-request.

## The first slice

No autonomy. A command:

```
dispatch take <project> <issue-or-task>
```

that makes the worktree(s) and the Switchboard workspace and projects,
runs the `investigate` stage, then `plan`, then hands the plan into the
existing review workflow, and stops. You watch it in Switchboard and
drive the rest by hand. That exercises the control port, the pipeline
file, lanes, the notes file, and one gate of each kind except
`command`, and it tells you whether the stage boundaries and the
operator guidance are right before any policy exists.

Then, in order: the `implement` and `ready` stages with their gates;
the queue and slots; decisions surfaced as waiting sessions; resources
and the Delta environment lock; the dial.

## Decisions I made (review these)

1. **Dispatch is a separate process over a control socket**, not a
   library user of the switchboard crate and not a thread inside the
   app. Reason: one owner of the data directory and the window; the
   deterministic core makes the socket cheap.
2. **Pipelines are TOML in Dispatch's data directory**, one per
   project, never in the repository. Reason: the hard rule about
   dot-directories, and the config-editor precedent for the one
   exception.
3. **Three gate kinds** (command, external, human) and no step
   language. Same reasoning as the workflow: two concrete pipelines
   plus a third with no deploy did not need more, and a generic one
   waits for the case that shows what is shared.
4. **Fresh session per stage, notes file as the memory.** Costs more
   than continuing one session. Reason: each stage then reads what is
   on disk, not what an earlier stage remembers, and the notes file is
   auditable.
5. **The review stage is a Switchboard plan review run**, with the
   operator's guidance as a workflow definition. Reason: it exists,
   it is durable, and its verdicts are string compares.
6. **Sources are per project**: GitHub label for Switchboard and
   Delta, a marker on a `task-list.md` line for PTA, and `manual`
   always available.
7. **Decisions start at `ask` everywhere**; `auto` only where the
   choice is trivial (one lane). The dial moves per decision kind.
8. **A shared environment is a resource with a count**, and a stage
   that needs it waits. Per-ticket environments later are a count
   raise, not a new mechanism.
9. **Fresh sessions never resume across a Switchboard restart** unless
   Dispatch asks again; the "never resume automatically" rule stays.
10. **The queue is reordered by hand in a working set**, no priority
    fields in the file. Reason: you said by hand for now, and the
    grid already exists.
