# Writing a pipeline file

This guide builds a Dispatch pipeline file one step at a time. It
starts with a single repository and ends with a project of several
repositories that share a deploy stack. It is written for the owner
and for a supervisor the owner asks to manage a pipeline file. Each
step brings in options where they first pay off. For each option it
says what the option buys, what it costs, and what goes wrong without
it. The design record for every topic is in
[`dispatch.md`](dispatch.md); this guide links to it rather than
repeating it. The key-by-key reference is
[The pipeline file](dispatch.md#the-pipeline-file).

**Where the file lives.** It is `pipelines/<project>.toml` in
Dispatch's data directory, never in a repository. A project that also
reviews other people's pull requests has a second file,
`pipelines/<project>.pr.toml` (step 6). When a ticket is taken, it
keeps a frozen copy of the file and runs from that copy on every pass.
An edit to the live file therefore reaches tickets taken afterwards,
and a running ticket only after `dispatch restart <ticket>`, which
swaps a fresh copy in. Step 7 lists the few keys that are read live.

**Checking a file.** Nothing runs a file until a `take`. A file that
does not parse or validate is refused at the next `take` with the
parser's reason, so read the file back after every edit. Every block
in this guide is parsed and validated by a test
(`every_block_in_the_pipeline_guide_is_a_pipeline_the_parser_knows`
in `dispatch/src/pipeline.rs`), which also refuses a key the parser
would silently ignore. A block marked `toml` is a whole file. A block
marked `toml fragment` is an excerpt, and the test merges it into the
whole file above it.

## 1. The minimum

One repository, issues from GitHub, and the stages from an issue to a
merged pull request: investigate, plan, review the plan, implement,
open the pull request, wait for CI, wait for the merge.

```toml
version = 1

[project]
name = "Switchboard"
repo = "git@github.com:msull/switchboard.git"
base = "main"
space = "Dispatch · Switchboard"

[source]
kind = "github"
repo = "msull/switchboard"
label = "dispatch"

[[lanes]]
name = "repo"
path = "."
setup = ["cargo", "fetch", "--locked"]

[operators.investigator]
kind = "claude"
guidance = "Read CLAUDE.md first. Name the files the issue touches, with line numbers. Write nothing but your notes."

[operators.planner]
kind = "claude"
guidance = "Write the plan as docs/design.md sections are written: the record, the actions and effects, the UI, the tests."

[operators.reviewer]
kind = "claude"
guidance = "Hold the plan to the hard rules in CLAUDE.md."

[operators.reviewer.review]
reviewer = "claude"
review_first = "Review the plan at {plan} for the repository at {worktree}. Write numbered objections to {feedback}. If there are none, write exactly this line alone: {no_feedback}"
review_round = "The planner answered at {response} and updated the plan at {plan}. Reply to each point at {feedback}. If you are satisfied, write exactly this line alone: {no_feedback}"
respond = "Feedback on your plan is at {feedback}. Edit the copy at {plan} and only that file, then answer each point at {response}."
respond_to_user = "The user says: {text}. Edit the copy at {plan} and only that file, then answer at {response}."
handoff = "The plan at {plan} was reviewed and is final."
no_feedback = "No further feedback."
cap = 4

[operators.implementer]
kind = "claude"
guidance = "Implement the finalized plan on the branch. Commit messages carry no tool attribution."

[[stages]]
name = "investigate"
operator = "investigator"
context = "root"
writes = ["notes"]
prompt = "Issue #{issue.number}: {issue.title}\n\n{issue.body}\n\nInvestigate and write your findings to {notes}."

[[stages]]
name = "lanes"
gate = { kind = "human", decision = "lanes" }

[[stages]]
name = "plan"
operator = "planner"
context = "each"
writes = ["plan"]
prompt = "Using the investigation at {inputs.notes}, write a plan for issue #{issue.number} to {plan}."

[[stages]]
name = "review"
review = "reviewer"
context = "each"
subject = "plan"
gate = { kind = "external", check = "review-finalized" }

[[stages]]
name = "implement"
operator = "implementer"
context = "each"
writes = ["notes"]
prompt = "The plan at {inputs.plan} is final. Implement it on branch {branch}, commit, and write your notes to {notes}. Do not push."
gate = { kind = "command", in = "lane", argv = ["sh", "-c", "cargo fmt --all -- --check && cargo clippy --locked --workspace --all-targets -- -D warnings && cargo test --locked --workspace"] }

[[stages]]
name = "pr"
operator = "implementer"
context = "each"
writes = ["notes"]
prompt = "Push {branch} and open a pull request against main with gh. Write its number to {notes}."
gate = { kind = "command", in = "lane", argv = ["sh", "-c", "test -z \"$(git status --porcelain)\" && git fetch -q origin && test \"$(git rev-parse HEAD)\" = \"$(git rev-parse \"origin/$DISPATCH_BRANCH\")\""] }

[[stages]]
name = "ready"
context = "each"
gate = { kind = "external", check = "pr-checks" }

[[stages]]
name = "merge"
context = "each"
gate = { kind = "external", check = "pr-merged", decision = "merge" }
```

**`version = 1`** is the only version there is. Any other number is
refused.

**`[project]`.**

- `name` is the project's name on the command line
  (`dispatch take Switchboard 42`).
- `repo` is the repository's URL. Dispatch keeps its own clone of it
  under its data directory and fetches before every cut. Each ticket
  works in a worktree of that clone, so your own checkout is never
  touched.
- `base` is the branch tickets branch from (default `main`), and
  `remote` names the remote (default `origin`).
- `space` is the Switchboard workspace every ticket's project goes
  in. It is required.

**`[source]`** says where tickets come from. `kind = "github"` with
`repo` reads issues from GitHub. `label` is required: it names the
label that marks an issue as handed over, for you and for a
supervisor. Dispatch itself takes an issue only when `dispatch take`
names it, and it never edits labels.

**`[[lanes]]`** are the places a ticket works. A single repository has
one lane at `path = "."`. `setup` runs once in each new worktree,
before the lane's first agent. It buys a tree that is ready to build
when the agent starts. Without it, the first agent spends its turn
fetching dependencies, and the gate may fail on a cold cache. Make it
safe to run twice: a restart that changed it runs it again.

**`[operators.<name>]`** are the agents. `kind = "claude"` is Claude
Code, whose Stop is the signal that an attempt finished. `guidance`
opens every prompt the operator gets. It is the place for what holds
across the operator's stages, such as which files to read first and
what not to do. Guidance is advisory: nothing stops an agent from
ignoring it.

**The plan review** is a workflow stage. It names a `review` operator,
not an `operator`. That operator carries a `[operators.<name>.review]`
table, which is a whole Switchboard review definition: the reviewer's
kind, the templates for each turn, and a `cap` on rounds. The stage's
`subject` is the artifact under review, and some earlier stage must
write it. The `review-finalized` gate passes when the review run is
finalized, which is the `finalize` decision unless its dial says
`auto` (step 7). See
[Stage semantics](dispatch.md#stage-semantics) for why a review works
on a copy.

**Contexts.** `context = "root"` runs once for the ticket, in the root
of its tree. `context = "each"` runs once in every lane the ticket
chose. With one lane the two run in the same directory, but they are
not the same. A stage that reads `{branch}` or holds a lane's commits
belongs in `each`, so that a second lane needs no rewrite. A stage that
looks across the whole project, such as an investigation, belongs in
`root`.

**The `lanes` stage** asks which lanes the ticket needs. With one lane
it passes without a question. It costs nothing here, and keeping it
means a second lane needs no new stage.

**Artifacts.** `writes = ["notes"]` names files the stage leaves. Each
name expands in the prompt to a path (`{notes}`), fresh for every
attempt. A later stage reads one as `{inputs.notes}`. Two stages may
write the same name. The reader gets the newest, from its own lane or
from a stage that runs in one context.

**Gates.** An agent stage without a `gate` is done when the agent
stops and its artifacts have settled. A command gate adds a check:
`in = "lane"` runs the argv in the lane's tree after the agent stops,
and only exit 0 completes the stage. Without one, an implementer that
stops with failing tests completes the stage anyway. The command sees
`DISPATCH_BRANCH`, `DISPATCH_TREE` and the rest listed under
[The pipeline file](dispatch.md#the-pipeline-file). The `pr` stage's
gate checks that the branch was pushed as it stands, so `ready` never
reads a pull request whose head is older than the tree.

**The external gates.** `pr-checks` waits for the pull request's checks
at the branch head. Pending is waiting, not failure. Green at an older
head is not green. `pr-merged` waits for the provider to report the
merge. It never merges anything itself. While it waits, its decision
(`merge` here) is pending, so the ticket reads as waiting on you. See
[PR checks and merges](dispatch.md#pr-checks-and-merges).

**A project that is not a repository.** `[project] root = "/path"`
in place of `repo` works in that directory as it is, with no clone and
no branch per ticket. `[source] kind = "task-file"` with `path` and
`marker` takes tickets from the lines of a file that carry the marker.
That shape gives up code review (step 2) and `without_lane` (step 4),
both of which need a clone.
[Pipeline: PTA](dispatch.md#pipeline-pta) is the worked example.

## 2. Code review

The plan was reviewed, but the code was not. A code review stage
between `implement` and `pr` puts reviewers on the branch, at once, in
rounds. A fresh agent addresses their points after each round, until
no point is open or the cap is reached.

A reviewer may be an agent or local tooling. A `kind = "command"`
operator runs its argv in the lane. Exit 0 means nothing to report.
Exit 1 with output means findings, and the output is the feedback. Any
other exit is a failure.

```toml fragment
[operators.lint]
kind = "command"
argv = ["sh", "-c", "cargo clippy --locked --workspace --all-targets --message-format short -- -D warnings 2>&1 || exit 1"]
```

A command reviewer costs no tokens and never misses what it checks.
It is only worth having when its findings are something the implement
gate does not already fail on, such as a stricter lint run as advice.

The stage, with the operators it names, is in the whole file at the
end of this step:

```toml fragment
[operators.correctness]
kind = "claude"
guidance = "Review the branch for correctness only: logic that does not do what the plan says, an effect that runs twice or never."

[operators.style]
kind = "claude"
guidance = "Review the branch against CLAUDE.md's Style section. Start every point with style:."

[operators.fixer]
kind = "claude"
guidance = "Address one round of review points on the branch. For each point, fix it or say in your response why not."
```

- **`reviewers`** lists the operators that review each round, at once.
  Each reviewer you add is another session per round. A reviewer
  named `style`, or a point that starts with `style:`, counts as
  wording. That matters for `style_rounds`.
- **`implementer`** is the operator that addresses a round's points. It
  is a fresh session each round, and it must be `claude`.
- **`cap`** is the number of review passes (default 3) before the
  points left become a `review-cap` question: accept, one more pass,
  or park. A higher cap spends more rounds before you are asked.
- **`style_rounds`** (default 2) is the round from which a round whose
  open points are all wording converges without a fix pass. Without
  it, a reviewer that always finds a word to change keeps the stage
  going until the cap. It is read from the live file every round, so
  an edit reaches a running review without a restart. Every other key
  of the stage, `commits` included, comes from the ticket's copy.
- **`commits`** is what happens to the branch's commits as the stage
  completes. `keep` (the default) leaves them as made. `fold` folds
  each fix round into the commit it amends. `one` squashes the branch
  into one commit. The tree is unchanged either way, so the checks do
  not run again. A pipeline of other people's pull requests refuses
  anything but `keep`.
- **`gate = { kind = "command", like = "implement" }`** runs
  `implement`'s checks again at the head the review accepted. A result
  of that command at the same clean head is reused rather than run
  again.
- **`review-code = "auto"`** in `[policy] decisions` starts each fix
  pass without asking. With the default `ask`, every round with open
  points is a question. `auto` pairs with `style_rounds`: rounds go
  on unasked until only wording is left, and then they stop.

**`budget_usd`** on an operator is meant as a budget per ticket across
that operator's attempts, priced by `[policy] rates` (step 7).
Budget reporting is not built yet: both keys parse and nothing reads
them, so neither stops or asks anything today. Set them only as a
record of intent. [Budget](dispatch.md#budget) is the design.

```toml
version = 1

[project]
name = "Switchboard"
repo = "git@github.com:msull/switchboard.git"
base = "main"
space = "Dispatch · Switchboard"

[source]
kind = "github"
repo = "msull/switchboard"
label = "dispatch"

[[lanes]]
name = "repo"
path = "."
setup = ["cargo", "fetch", "--locked"]

[operators.investigator]
kind = "claude"
guidance = "Read CLAUDE.md first. Name the files the issue touches, with line numbers. Write nothing but your notes."

[operators.planner]
kind = "claude"
guidance = "Write the plan as docs/design.md sections are written: the record, the actions and effects, the UI, the tests."

[operators.reviewer]
kind = "claude"
guidance = "Hold the plan to the hard rules in CLAUDE.md."

[operators.reviewer.review]
reviewer = "claude"
review_first = "Review the plan at {plan} for the repository at {worktree}. Write numbered objections to {feedback}. If there are none, write exactly this line alone: {no_feedback}"
review_round = "The planner answered at {response} and updated the plan at {plan}. Reply to each point at {feedback}. If you are satisfied, write exactly this line alone: {no_feedback}"
respond = "Feedback on your plan is at {feedback}. Edit the copy at {plan} and only that file, then answer each point at {response}."
respond_to_user = "The user says: {text}. Edit the copy at {plan} and only that file, then answer at {response}."
handoff = "The plan at {plan} was reviewed and is final."
no_feedback = "No further feedback."
cap = 4

[operators.implementer]
kind = "claude"
guidance = "Implement the finalized plan on the branch. Commit messages carry no tool attribution."
budget_usd = 20.0

[operators.correctness]
kind = "claude"
guidance = "Review the branch for correctness only: logic that does not do what the plan says, an effect that runs twice or never."

[operators.style]
kind = "claude"
guidance = "Review the branch against CLAUDE.md's Style section. Start every point with style:."

[operators.lint]
kind = "command"
argv = ["sh", "-c", "cargo clippy --locked --workspace --all-targets --message-format short -- -D warnings 2>&1 || exit 1"]

[operators.fixer]
kind = "claude"
guidance = "Address one round of review points on the branch. For each point, fix it or say in your response why not."

[[stages]]
name = "investigate"
operator = "investigator"
context = "root"
writes = ["notes"]
prompt = "Issue #{issue.number}: {issue.title}\n\n{issue.body}\n\nInvestigate and write your findings to {notes}."

[[stages]]
name = "lanes"
gate = { kind = "human", decision = "lanes" }

[[stages]]
name = "plan"
operator = "planner"
context = "each"
writes = ["plan"]
prompt = "Using the investigation at {inputs.notes}, write a plan for issue #{issue.number} to {plan}."

[[stages]]
name = "review"
review = "reviewer"
context = "each"
subject = "plan"
gate = { kind = "external", check = "review-finalized" }

[[stages]]
name = "implement"
operator = "implementer"
context = "each"
writes = ["notes"]
prompt = "The plan at {inputs.plan} is final. Implement it on branch {branch}, commit, and write your notes to {notes}. Do not push."
gate = { kind = "command", in = "lane", argv = ["sh", "-c", "cargo fmt --all -- --check && cargo clippy --locked --workspace --all-targets -- -D warnings && cargo test --locked --workspace"] }

[[stages]]
name = "review-code"
context = "each"
reviewers = ["correctness", "style", "lint"]
implementer = "fixer"
cap = 3
style_rounds = 2
commits = "fold"
gate = { kind = "command", like = "implement" }

[[stages]]
name = "pr"
operator = "implementer"
context = "each"
writes = ["notes"]
prompt = "Push {branch} and open a pull request against main with gh. Write its number to {notes}."
gate = { kind = "command", in = "lane", argv = ["sh", "-c", "test -z \"$(git status --porcelain)\" && git fetch -q origin && test \"$(git rev-parse HEAD)\" = \"$(git rev-parse \"origin/$DISPATCH_BRANCH\")\""] }

[[stages]]
name = "ready"
context = "each"
gate = { kind = "external", check = "pr-checks" }

[[stages]]
name = "merge"
context = "each"
gate = { kind = "external", check = "pr-merged", decision = "merge" }

[policy]
rates = { "claude-sonnet-5" = [3.0, 15.0], "claude-opus-5-5" = [15.0, 75.0] }
decisions = { lanes = "auto", finalize = "ask", review-code = "auto" }
```

Publishing only after review (`pr` after `review-code`) buys one CI run
per ticket, and no pull request ever shows unreviewed code. See
[The code review stage](dispatch.md#the-code-review-stage) for rounds,
carried points and the resolution review.

## 3. Several repositories

From here on the example is Orchard: a planning workspace whose issues
live in `example-org/orchard-workspace`, with three application
repositories nested under it. They are a backend (integration branch
`main`), a frontend (`dev`) and an admin frontend (`master`).
[Pipeline: Orchard](dispatch.md#pipeline-orchard) has the facts it was
shaped by.

```toml
version = 1

[project]
name = "Orchard"
repo = "git@github.com:example-org/orchard-workspace.git"
space = "Dispatch · Orchard"

[source]
kind = "github"
repo = "example-org/orchard-workspace"
label = "dispatch"
lane_hints = { "area:backend" = "backend", "area:frontend" = "frontend", "area:admin" = "admin" }

[[lanes]]
name = "backend"
path = "orchard-backend"
repo = "git@bitbucket.org:example-co/orchard-backend.git"
base = "main"
setup = ["uv", "sync"]

[[lanes]]
name = "frontend"
path = "orchard-frontend"
repo = "git@bitbucket.org:example-co/orchard-frontend.git"
base = "dev"
setup = ["npm", "ci", "--legacy-peer-deps"]

[[lanes]]
name = "admin"
path = "orchard-admin"
repo = "git@bitbucket.org:example-co/orchard-admin.git"
base = "master"
setup = ["npm", "ci", "--legacy-peer-deps"]

[operators.investigator]
kind = "claude"
guidance = "Read the root CLAUDE.md and each repository's CLAUDE.md. Name the files, endpoints and screens the issue touches, and say which lanes it needs and why. Write nothing but your notes."

[operators.planner]
kind = "claude"
guidance = "Plan per lane: the change, the tests, and the changelog entry."

[operators.reviewer]
kind = "codex"
guidance = "Hold the plan to guides/COLLABORATION_GUIDE.md."

[operators.reviewer.review]
reviewer = "codex"
review_first = "The plan at {plan} is for the repository at {worktree}. Check that the backend and frontend halves agree on the API. Write objections to {feedback}. If there are none, write exactly this line alone: {no_feedback}"
review_round = "The planner answered at {response} and updated the plan at {plan}. Reply at {feedback}. If you are satisfied, write exactly this line alone: {no_feedback}"
respond = "Feedback on your plan is at {feedback}. Edit the copy at {plan} and only that file, then answer each point at {response}."
respond_to_user = "The user says: {text}. Edit the copy at {plan} and only that file, then answer at {response}."
handoff = "The plan at {plan} was reviewed and is final."
no_feedback = "No further feedback."
cap = 4

[operators.implementer]
kind = "claude"
guidance = "Implement the lane's part of the finalized plan on branch {branch}, in this worktree only. Run the lane's checks before every commit."

[operators.code-reviewer]
kind = "claude"
guidance = "Hold the lane's branch to its repository's guides/COLLABORATION_GUIDE.md and to the finalized plan."

[operators.fixer]
kind = "claude"
guidance = "Address one round of review points on the branch. For each point, fix it or say in your response why not."

[[stages]]
name = "investigate"
operator = "investigator"
context = "root"
writes = ["notes"]
prompt = "Issue #{issue.number}: {issue.title}\n\n{issue.body}\n\nThe lanes are {lanes.all}. Investigate and write your findings, including the lanes needed, to {notes}."

[[stages]]
name = "lanes"
gate = { kind = "human", decision = "lanes" }

[[stages]]
name = "plan"
operator = "planner"
context = "joined"
writes = ["plan"]
prompt = "Using {inputs.investigate.notes}, write one plan covering the lanes {lanes} of issue #{issue.number} to {plan}."

[[stages]]
name = "review"
review = "reviewer"
context = "joined"
subject = "plan"
gate = { kind = "external", check = "review-finalized" }

[[stages]]
name = "implement"
operator = "implementer"
context = "each"
prompt = "The plan at {inputs.plan} is final. Implement the {lane} part on {branch}, and commit."
gate = { kind = "command", in = "lane", per_lane = { backend = ["sh", "-c", "uv run inv lint && uv run inv pytest"], frontend = ["sh", "-c", "CI=true npm test -- --watchAll=false"], admin = ["sh", "-c", "CI=true npm test"] } }

[[stages]]
name = "review-code"
context = "each"
reviewers = ["code-reviewer"]
implementer = "fixer"
gate = { kind = "command", like = "implement" }

[[stages]]
name = "pr"
operator = "implementer"
context = "each"
writes = ["notes"]
prompt = "Push {branch} and open a pull request against the lane's base with a self-contained body. Write its id to {notes}."
gate = { kind = "command", in = "lane", argv = ["sh", "-c", "test -z \"$(git status --porcelain)\" && git fetch -q origin && test \"$(git rev-parse HEAD)\" = \"$(git rev-parse \"origin/$DISPATCH_BRANCH\")\""] }

[[stages]]
name = "ready"
context = "each"
gate = { kind = "external", check = "pr-checks" }

[[stages]]
name = "merge"
context = "each"
gate = { kind = "external", check = "pr-merged", decision = "merge" }

[policy]
slots = 2
decisions = { lanes = "ask", finalize = "ask", review-code = "auto" }
```

**Lanes with their own repositories.** A lane with `repo` is cloned by
Dispatch too, and cut as a worktree at its `path` inside the ticket's
tree, so the layout matches a checkout of the workspace. `base` is the
lane's own integration branch. Without it the lane branches from the
project's `base`, which for the frontend is the wrong branch. `setup`
is per lane, because each repository installs differently. A lane can
also take `remote` and `remotes` (mirrors a pull request may be taken
from) as the project does.

**The `lanes` decision** now matters. Every lane is cut before the
first stage, and this gate chooses which ones the ticket's work runs
in. `lane_hints` maps issue labels to lanes. They are the suggested
answer, shown with the investigator's notes. With the `lanes` dial at
`ask`, the hint is only a suggestion. At `auto`, a hint answers the
question, and an issue with no hinted label is still asked. Without
`lane_hints`, every multi-lane ticket asks with no suggestion.

**Which context each stage wants.**

- `root` runs once, before lanes exist or across them. Use it for the
  investigation, which must see every repository to say which lanes
  are needed. `{lanes.all}` names every lane of the file.
- `joined` runs once in the root, after every lane's agent has
  stopped, with every chosen lane named. Use it for the plan and its
  review, so that the backend and frontend halves are planned and
  reviewed together. As `each`, the halves would be planned apart and
  could disagree on the API. `{lanes}` names the lanes the ticket chose.
- `each` runs once per chosen lane, in parallel, and the ticket
  advances when every lane's gate passes. Use it for anything that
  works on a branch.
- `lane:<name>` runs in that one lane, and is skipped when the ticket
  did not choose it. Step 4 uses it for a deploy. A list of lane names
  runs in just those lanes.

**`per_lane`** gives one gate a command per lane, run in the lane
(`in = "lane"`). A lane missing from `per_lane` has no command to run,
so name every lane the stage can run in.

**`{inputs.*}`, both forms.** `{inputs.notes}` is the newest `notes`
from this lane's attempts or from a stage that runs in one context.
`{inputs.investigate.notes}` names the stage. Use the stage form
whenever two stages write the same name and the reader wants a
particular one. A `root` or `joined` stage may not read a file that
only a per-lane stage writes, in either form, because which lane's
copy it got would be a guess. Such a file is refused when it loads.
The agent also gets each input as an environment variable,
`DISPATCH_INPUT_NOTES` or `DISPATCH_INPUT_INVESTIGATE_NOTES`, set to the
path.

## 4. A deploy stack

Orchard's test environment is one personal backend stack, `my-dev`,
deployed from any branch. Two tickets deploying to it would overwrite
each other, and nothing in the repositories stops that. A resource with
`count = 1` does.

```toml fragment
[[resources]]
name = "my-dev"
count = 1
```

A resource is a lock, not an environment. Raising `count` to 2 does not
create a second stack. It lets two tickets deploy to the same one. See
[Resources and slots](dispatch.md#resources-and-slots).

**`needs`** on a stage holds the resource while that stage runs. A run
of consecutive stages that all name it is one hold, from the first
stage of the run to the last. The hold is released at the end of a run
and taken again at the start of the next. Holds are on the ticket
record, so a runner restart does not drop one in the middle of a run.

- A code review stage between `implement` and the deploy splits the
  hold into two runs. That is deliberate: review rounds take hours, and
  every other ticket would wait through them. A code review stage that
  names a resource is refused at `take` and at `dispatch restart`.
- A workflow stage (a plan review) may stand only last in a run, since
  a hold retaken partway through a run sends the ticket back to the
  run's first stage, and a review cannot be sent back that way.

Here `implement` holds `my-dev` in a run of its own, so the
implementers may deploy their lane while they work. Its operator gets
`args`, extra flags for the agent's command line, that allow the
deploy. Nothing they deploy is kept, because the pipeline deploys again.

**The deploy that counts** is a gate-only stage: no operator, just a
command gate. It runs once, and the head it ran at is recorded on the
attempt, so a later stage reads it as `{inputs.deploy.commit}`. A
gate-only command in `in = "lane:backend"` must have
`context = "lane:backend"`.

**`without_lane = "base"`.** A `lane:backend` stage is skipped when the
ticket did not choose the backend lane. For a deploy that is wrong. A
frontend-only ticket would then be tried against whatever the stack
last held, which may be another ticket's half-finished backend. With
`without_lane = "base"`, the stage deploys the backend's base branch
from the project's shared base tree instead, holds the resource as any
deploy does, and records that commit. The key is valid only on a
gate-only command stage in `lane:<name>`, in a project with `repo`.
The deploy must exit 0 when the stack already holds the commit, so make
the deploy task idempotent.

**A check of the deploy.** A deploy that exits 0 has not proved the
stack answers. A gate-only `deploy-check` stage in the same run probes
it, and fails the attempt into a `rerun` question before anyone looks.

**A human look.** `inspect` is a human gate in the same run, so the
stack stays deployed while you look. `confirm = true` makes it a
"you did this" gate: the answer is `done` (you looked) or `park`, and
it is never answered automatically, whatever the `decisions` dial
says. It offers `rerun` as well only when the nearest earlier agent
stage holds the same resource in the same run, which is true of step
5's `tried` and not of `inspect`, whose stages before it are commands.

```toml
version = 1

[project]
name = "Orchard"
repo = "git@github.com:example-org/orchard-workspace.git"
space = "Dispatch · Orchard"

[source]
kind = "github"
repo = "example-org/orchard-workspace"
label = "dispatch"
lane_hints = { "area:backend" = "backend", "area:frontend" = "frontend", "area:admin" = "admin" }

[[lanes]]
name = "backend"
path = "orchard-backend"
repo = "git@bitbucket.org:example-co/orchard-backend.git"
base = "main"
setup = ["sh", "-c", "uv sync && uv run inv link-env --env-name my-dev"]

[[lanes]]
name = "frontend"
path = "orchard-frontend"
repo = "git@bitbucket.org:example-co/orchard-frontend.git"
base = "dev"
setup = ["npm", "ci", "--legacy-peer-deps"]

[[lanes]]
name = "admin"
path = "orchard-admin"
repo = "git@bitbucket.org:example-co/orchard-admin.git"
base = "master"
setup = ["npm", "ci", "--legacy-peer-deps"]

[[resources]]
name = "my-dev"
count = 1

[operators.investigator]
kind = "claude"
guidance = "Read the root CLAUDE.md and each repository's CLAUDE.md. Name the files, endpoints and screens the issue touches, and say which lanes it needs and why. Write nothing but your notes."

[operators.planner]
kind = "claude"
guidance = "Plan per lane: the change, the tests, the changelog entry, and how it is tried on my-dev."

[operators.reviewer]
kind = "codex"
guidance = "Hold the plan to guides/COLLABORATION_GUIDE.md."

[operators.reviewer.review]
reviewer = "codex"
review_first = "The plan at {plan} is for the repository at {worktree}. Check that the backend and frontend halves agree on the API. Write objections to {feedback}. If there are none, write exactly this line alone: {no_feedback}"
review_round = "The planner answered at {response} and updated the plan at {plan}. Reply at {feedback}. If you are satisfied, write exactly this line alone: {no_feedback}"
respond = "Feedback on your plan is at {feedback}. Edit the copy at {plan} and only that file, then answer each point at {response}."
respond_to_user = "The user says: {text}. Edit the copy at {plan} and only that file, then answer at {response}."
handoff = "The plan at {plan} was reviewed and is final."
no_feedback = "No further feedback."
cap = 4

[operators.implementer]
kind = "claude"
guidance = "Implement the lane's part of the finalized plan on branch {branch}, in this worktree only. Run the lane's checks before every commit. Never run a deploy."

[operators.lane-implementer]
kind = "claude"
args = ["--allowedTools", "Bash(uv run inv deploy:*)"]
guidance = "Implement the lane's part of the finalized plan on branch {branch}, in this worktree only. Run the lane's checks before every commit. You may deploy your lane to my-dev while you work; nothing you deploy is kept."

[operators.code-reviewer]
kind = "claude"
guidance = "Hold the lane's branch to its repository's guides/COLLABORATION_GUIDE.md and to the finalized plan."

[operators.fixer]
kind = "claude"
guidance = "Address one round of review points on the branch. For each point, fix it or say in your response why not. Never run a deploy."

[[stages]]
name = "investigate"
operator = "investigator"
context = "root"
writes = ["notes"]
prompt = "Issue #{issue.number}: {issue.title}\n\n{issue.body}\n\nThe lanes are {lanes.all}. Investigate and write your findings, including the lanes needed, to {notes}."

[[stages]]
name = "lanes"
gate = { kind = "human", decision = "lanes" }

[[stages]]
name = "plan"
operator = "planner"
context = "joined"
writes = ["plan"]
prompt = "Using {inputs.investigate.notes}, write one plan covering the lanes {lanes} of issue #{issue.number} to {plan}."

[[stages]]
name = "review"
review = "reviewer"
context = "joined"
subject = "plan"
gate = { kind = "external", check = "review-finalized" }

[[stages]]
name = "implement"
operator = "lane-implementer"
context = "each"
needs = ["my-dev"]
prompt = "The plan at {inputs.plan} is final. Implement the {lane} part on {branch}, and commit. You hold my-dev while you work."
gate = { kind = "command", in = "lane", per_lane = { backend = ["sh", "-c", "uv run inv lint && uv run inv pytest"], frontend = ["sh", "-c", "CI=true npm test -- --watchAll=false"], admin = ["sh", "-c", "CI=true npm test"] } }

[[stages]]
name = "review-code"
context = "each"
reviewers = ["code-reviewer"]
implementer = "fixer"
gate = { kind = "command", like = "implement" }

[[stages]]
name = "deploy"
context = "lane:backend"
without_lane = "base"
needs = ["my-dev"]
gate = { kind = "command", in = "lane:backend", argv = ["sh", "-c", "uv run inv link-env --env-name my-dev && uv run inv deploy -f"] }

[[stages]]
name = "deploy-check"
context = "root"
needs = ["my-dev"]
gate = { kind = "command", in = "root", argv = ["sh", "-c", "curl -fsS --retry 5 --retry-delay 10 https://api.my-dev.example.com/health"] }

[[stages]]
name = "inspect"
needs = ["my-dev"]
gate = { kind = "human", decision = "inspect", confirm = true }

[[stages]]
name = "pr"
operator = "implementer"
context = "each"
writes = ["notes"]
prompt = "Push {branch} and open a pull request against the lane's base with a self-contained body. Write its id to {notes}."
gate = { kind = "command", in = "lane", argv = ["sh", "-c", "test -z \"$(git status --porcelain)\" && git fetch -q origin && test \"$(git rev-parse HEAD)\" = \"$(git rev-parse \"origin/$DISPATCH_BRANCH\")\""] }

[[stages]]
name = "ready"
context = "each"
gate = { kind = "external", check = "pr-checks" }

[[stages]]
name = "merge"
context = "each"
gate = { kind = "external", check = "pr-merged", decision = "merge" }

[policy]
slots = 2
decisions = { lanes = "ask", finalize = "ask", review-code = "auto" }
```

The deploy links the tree to `my-dev` again right before deploying, so
the target is never inherited from another checkout. The backend's
`setup` links it too, for the implementer's own deploys. `slots = 2`
lets two tickets run while only one holds `my-dev`; the other works
through its plan and review and waits at `implement`.

## 5. Trying it

`inspect` asks you to look, but the frontends are not running anywhere.
A tester agent with the frontends served against `my-dev` can try the
change end to end first, and the human gate becomes a look at its
evidence.

**`serve`** on a lane says how to run it. `argv` is the command,
`env` its environment, `url` the address the tester is told, and
`ready` a probe (`http`, a path, answering within `within_secs`).
`{port}` is a port Dispatch allocates per ticket. `serve.env` travels
on the service's command line, so it may hold only literals and
`{port}`, and a key that looks like a secret is refused.

**`services`** on a stage names lanes to serve while it runs. Each is
started only if the ticket cut that lane; for one it did not,
`{services.<lane>}` renders `not served (no <lane> lane)`. A stage
with `services` must hold a `[[resources]]` entry, since the services
live until that run's last stage ends. The file must have
`[policy] ports`, the range ports are allocated from. Each port is
tested free before use, so a server you started by hand is simply
not chosen.

**`before`** maps a served lane to a command run before its service
starts, such as linking the frontend to `my-dev`. Its keys must be
among the stage's `services`. It must be safe to run again: a retry,
a send-back or a runner restart runs it anew. A `before` that fails,
no free port, or a probe that never answers is a decision before the
tester is launched.

**`env` sets** are named groups of variables and secrets the owner
keeps in Switchboard, never in the pipeline file. On an operator or an
agent stage, they are granted to the agent's sessions, and the prompt
ends with one sentence on running commands through
`switchboard-env exec -- <command>`. Only `claude` operators take them.
On a command gate, `env` makes the gate run under
`switchboard-env exec --` with the runner's own grants, which the owner
makes with `switchboard-env grant --runner <set>`. Without a runner
grant the gate gets nothing from the set. A runner started by hand,
rather than from the Dispatch overview, fails such a gate. Here the
deploy gets AWS credentials from the `aws-dev` set rather than from
the runner's environment.

**The `tried` decision** replaces `inspect`. It is a human gate in the
same run as the tester, so it shows the first line of the tester's
notes and the deployed commit. What you look at is what was tested. A
tester that could not test anything should say so on that first line.
Answering `rerun` tests again on the same deploy, with the services
restarted.

**An evidence directory** keeps what the tester produced besides its
notes: screenshots, a rendered PDF, an exported CSV. A stage's `writes`
may name one directory as `{ name = "evidence", dir = true }`. Dispatch
makes it inside the attempt directory before the launch and hands it
to the agent as `{evidence}` and `$DISPATCH_WRITES_EVIDENCE`; a
gate-only command gets the variable too. When the attempt completes,
the files are listed on the attempt (path, size, time; a file still
being written holds completion), the ticket page shows them under the
attempt with Open and Reveal, the `tried` question carries the count,
and `dispatch evidence <ticket>` prints their paths. The contents are
never read into the app: Open hands the path to the system opener, and
only for a list of safe extensions. Without the directory, a screenshot
lands in the worktree or `/tmp`, dirties the tree or is lost when the
tree is removed, and the notes can only describe it.

What reaches the directory without a permission prompt is measured in
`spikes/21-evidence-writes`: the Write tool does, and so does any
gate-only command; Bash `cp`, `mv` and `mkdir` ask, so the tester is
told to write files there rather than copy them. A Playwright MCP
screenshot reaches it only when the tester's server runs with
`--allow-unrestricted-file-access` in the operator's `--mcp-config`
and the tester passes `{evidence}/<file>` as `filename`; that flag also
lets the browser read any local file, so it is your choice per
operator. An empty directory never fails the attempt. Codex writes only
in its cwd, so `dir = true` on a Codex stage is refused.

```toml
version = 1

[project]
name = "Orchard"
repo = "git@github.com:example-org/orchard-workspace.git"
space = "Dispatch · Orchard"

[source]
kind = "github"
repo = "example-org/orchard-workspace"
label = "dispatch"
lane_hints = { "area:backend" = "backend", "area:frontend" = "frontend", "area:admin" = "admin" }

[[lanes]]
name = "backend"
path = "orchard-backend"
repo = "git@bitbucket.org:example-co/orchard-backend.git"
base = "main"
setup = ["sh", "-c", "uv sync && uv run inv link-env --env-name my-dev"]

[[lanes]]
name = "frontend"
path = "orchard-frontend"
repo = "git@bitbucket.org:example-co/orchard-frontend.git"
base = "dev"
setup = ["npm", "ci", "--legacy-peer-deps"]
serve = { argv = ["npm", "start"], env = { BROWSER = "none", PORT = "{port}" }, url = "http://localhost:{port}", ready = { http = "/", within_secs = 120 } }

[[lanes]]
name = "admin"
path = "orchard-admin"
repo = "git@bitbucket.org:example-co/orchard-admin.git"
base = "master"
setup = ["npm", "ci", "--legacy-peer-deps"]
serve = { argv = ["npm", "start"], env = { BROWSER = "none", PORT = "{port}" }, url = "http://localhost:{port}", ready = { http = "/", within_secs = 120 } }

[[resources]]
name = "my-dev"
count = 1

[operators.investigator]
kind = "claude"
guidance = "Read the root CLAUDE.md and each repository's CLAUDE.md. Name the files, endpoints and screens the issue touches, and say which lanes it needs and why. Write nothing but your notes."

[operators.planner]
kind = "claude"
guidance = "Plan per lane: the change, the tests, the changelog entry, and how it is tried on my-dev."

[operators.reviewer]
kind = "codex"
guidance = "Hold the plan to guides/COLLABORATION_GUIDE.md."

[operators.reviewer.review]
reviewer = "codex"
review_first = "The plan at {plan} is for the repository at {worktree}. Check that the backend and frontend halves agree on the API. Write objections to {feedback}. If there are none, write exactly this line alone: {no_feedback}"
review_round = "The planner answered at {response} and updated the plan at {plan}. Reply at {feedback}. If you are satisfied, write exactly this line alone: {no_feedback}"
respond = "Feedback on your plan is at {feedback}. Edit the copy at {plan} and only that file, then answer each point at {response}."
respond_to_user = "The user says: {text}. Edit the copy at {plan} and only that file, then answer at {response}."
handoff = "The plan at {plan} was reviewed and is final."
no_feedback = "No further feedback."
cap = 4

[operators.implementer]
kind = "claude"
guidance = "Implement the lane's part of the finalized plan on branch {branch}, in this worktree only. Run the lane's checks before every commit. Never run a deploy."

[operators.lane-implementer]
kind = "claude"
args = ["--allowedTools", "Bash(uv run inv deploy:*)"]
guidance = "Implement the lane's part of the finalized plan on branch {branch}, in this worktree only. Run the lane's checks before every commit. You may deploy your lane to my-dev while you work; nothing you deploy is kept."

[operators.code-reviewer]
kind = "claude"
guidance = "Hold the lane's branch to its repository's guides/COLLABORATION_GUIDE.md and to the finalized plan."

[operators.fixer]
kind = "claude"
guidance = "Address one round of review points on the branch. For each point, fix it or say in your response why not. Never run a deploy."

[operators.tester]
kind = "claude"
env = ["aws-dev"]
guidance = "The backend is already deployed to my-dev and the frontends are already served; do not deploy or start a server. Never print a credential."

[[stages]]
name = "investigate"
operator = "investigator"
context = "root"
writes = ["notes"]
prompt = "Issue #{issue.number}: {issue.title}\n\n{issue.body}\n\nThe lanes are {lanes.all}. Investigate and write your findings, including the lanes needed, to {notes}."

[[stages]]
name = "lanes"
gate = { kind = "human", decision = "lanes" }

[[stages]]
name = "plan"
operator = "planner"
context = "joined"
writes = ["plan"]
prompt = "Using {inputs.investigate.notes}, write one plan covering the lanes {lanes} of issue #{issue.number} to {plan}."

[[stages]]
name = "review"
review = "reviewer"
context = "joined"
subject = "plan"
gate = { kind = "external", check = "review-finalized" }

[[stages]]
name = "implement"
operator = "lane-implementer"
context = "each"
needs = ["my-dev"]
prompt = "The plan at {inputs.plan} is final. Implement the {lane} part on {branch}, and commit. You hold my-dev while you work."
gate = { kind = "command", in = "lane", per_lane = { backend = ["sh", "-c", "uv run inv lint && uv run inv pytest"], frontend = ["sh", "-c", "CI=true npm test -- --watchAll=false"], admin = ["sh", "-c", "CI=true npm test"] } }

[[stages]]
name = "review-code"
context = "each"
reviewers = ["code-reviewer"]
implementer = "fixer"
gate = { kind = "command", like = "implement" }

[[stages]]
name = "deploy"
context = "lane:backend"
without_lane = "base"
needs = ["my-dev"]
env = ["aws-dev"]
gate = { kind = "command", in = "lane:backend", argv = ["sh", "-c", "uv run inv link-env --env-name my-dev && uv run inv deploy -f"] }

[[stages]]
name = "deploy-check"
context = "root"
needs = ["my-dev"]
gate = { kind = "command", in = "root", argv = ["sh", "-c", "curl -fsS --retry 5 --retry-delay 10 https://api.my-dev.example.com/health"] }

[[stages]]
name = "try"
operator = "tester"
context = "joined"
needs = ["my-dev"]
services = ["frontend", "admin"]
before = { frontend = ["npm", "run", "link-env"], admin = ["npm", "run", "link-env"] }
writes = ["notes", { name = "evidence", dir = true }]
prompt = "my-dev runs backend commit {inputs.deploy.commit}. The frontend: {services.frontend}. The admin frontend: {services.admin}. The plan is at {inputs.plan}. Try issue #{issue.number} end to end. Write to {notes}, first line the result, then the commands and their output. Save screenshots and any file you produce under {evidence} with the Write tool, not cp or mv."

[[stages]]
name = "tried"
needs = ["my-dev"]
gate = { kind = "human", decision = "tried", confirm = true }

[[stages]]
name = "pr"
operator = "implementer"
context = "each"
writes = ["notes"]
prompt = "Push {branch} and open a pull request against the lane's base with a self-contained body. Write its id to {notes}."
gate = { kind = "command", in = "lane", argv = ["sh", "-c", "test -z \"$(git status --porcelain)\" && git fetch -q origin && test \"$(git rev-parse HEAD)\" = \"$(git rev-parse \"origin/$DISPATCH_BRANCH\")\""] }

[[stages]]
name = "ready"
context = "each"
gate = { kind = "external", check = "pr-checks" }

[[stages]]
name = "merge"
context = "each"
gate = { kind = "external", check = "pr-merged", decision = "merge" }

[policy]
slots = 2
ports = [3100, 3199]
decisions = { lanes = "ask", finalize = "ask", review-code = "auto" }
```

Two traps read as plain test failures:

- Gates and `setup` run with the runner's `PATH`. A tool installed
  globally can stand in for one the lane's `setup` did not install, so
  a lint passes from a global copy while a test runner fails with
  `command not found`. When a Python lane keeps its tools in a
  dependency group, both `setup` and the gate need that group
  (`uv sync --all-groups`, `uv run --all-groups ...`).
- A gate that exits 127 is that case, and the decision's text says so.
  `rerun` and `check` cannot help. Fix the pipeline, then restart the
  tickets taken under the broken one.

## 6. Pull requests

**`provider`** on an external gate says which host the pull request is
on. Without it, the provider is guessed from the lane's remote:
`github.com` is read through `gh`, `bitbucket.org` through Bitbucket's
API. Name it when the remote does not say, or to make the file say so
outright. On a repository that runs no CI, `checks = "none"` on the
`pr-checks` gate passes on a pull request at the head alone. Without
it, every reading of "no checks" is a question.

This step's changes to the main file are fragments, merged in step 7's
whole file.

```toml fragment
[[stages]]
name = "ready"
context = "each"
gate = { kind = "external", check = "pr-checks", provider = "bitbucket" }

[[stages]]
name = "merge"
context = "each"
gate = { kind = "external", check = "pr-merged", provider = "bitbucket" }
```

A `pr-merged` gate with no `decision` asks `merge`, which is what the
earlier steps named. That name is what a supervisor's `decides` lists
(step 8).

**A conflict or red checks.** The base moves while a pull request
waits. A `rebaser` is an operator that rebases a conflicting branch,
continued from the lane's implementer, up to `max_rebases` times per
pull request (default 2). A `fixer` fixes red checks at the tree's
head, up to `max_fixes` (default 2). Without them, each is a question
for you. After the last code review stage, a conflict's resolution is
reviewed once by `resolution_reviewer`. Absent, that is the stage's
first reviewer that is not `style`. See
[PR checks and merges](dispatch.md#pr-checks-and-merges).

```toml fragment
[operators.rebaser]
kind = "claude"
guidance = "Rebase the branch onto its moved base. Keep the change's intent on every conflict; when the base changed the same code for its own reasons, keep both. Run the lane's checks, then push."

[policy]
rebaser = "rebaser"
max_rebases = 2
fixer = "fixer"
max_fixes = 2
resolution_reviewer = "code-reviewer"
```

**Merge order.** The frontend calls an endpoint the backend adds, so
the frontend must not merge first. `merge_after = ["backend"]` on the
frontend lane holds its `merge` question until the backend's pull
request has merged. It needs a per-lane `pr-merged` stage that watches
both lanes, which `context = "each"` does. A lane the ticket did not
choose holds nothing. `{lane.merge_after}` in a prompt tells the
lane's agent about the dependency.
`merge_after_deploy = true` also waits for the base's pipeline on the
merge commit, and a step's name waits for that step alone. It is
refused wherever the file names Bitbucket, as Orchard's does, because
Bitbucket's pipeline reads have not been verified (spike 16). See
[Merge order](dispatch.md#merge-order).

```toml fragment
[[lanes]]
name = "frontend"
path = "orchard-frontend"
repo = "git@bitbucket.org:example-co/orchard-frontend.git"
base = "dev"
setup = ["npm", "ci", "--legacy-peer-deps"]
serve = { argv = ["npm", "start"], env = { BROWSER = "none", PORT = "{port}" }, url = "http://localhost:{port}", ready = { http = "/", within_secs = 120 } }
merge_after = ["backend"]
```

**Someone else's pull request.** A second file,
`pipelines/<project>.pr.toml`, takes tickets from pull requests named
on the command line (`dispatch take Orchard pr backend/123`). Its
`[source]` is `kind = "pull-request"` and its lanes are the project's.
It is the tail of the issue pipeline: the checks, your sign-off, and
the merge. Nothing in it pushes, so it refuses a `rebaser` or `fixer`,
and a code review stage there keeps the commits as they are. The
project's `.toml` governs the queue, `slots` and `waiting_on_me` for
both files. See
[Tickets from pull requests](dispatch.md#tickets-from-pull-requests).

```toml
version = 1

[project]
name = "Orchard"
repo = "git@github.com:example-org/orchard-workspace.git"
space = "Dispatch · Orchard"

[source]
kind = "pull-request"

[[lanes]]
name = "backend"
path = "orchard-backend"
repo = "git@bitbucket.org:example-co/orchard-backend.git"
base = "main"
setup = ["uv", "sync"]

[[lanes]]
name = "frontend"
path = "orchard-frontend"
repo = "git@bitbucket.org:example-co/orchard-frontend.git"
base = "dev"
setup = ["npm", "ci", "--legacy-peer-deps"]

[[lanes]]
name = "admin"
path = "orchard-admin"
repo = "git@bitbucket.org:example-co/orchard-admin.git"
base = "master"
setup = ["npm", "ci", "--legacy-peer-deps"]

[[stages]]
name = "ready"
context = "each"
gate = { kind = "external", check = "pr-checks", provider = "bitbucket" }

[[stages]]
name = "inspect"
context = "each"
gate = { kind = "human", decision = "inspect" }

[[stages]]
name = "merge"
context = "each"
gate = { kind = "external", check = "pr-merged", provider = "bitbucket" }

[policy]
decisions = { merge = "ask" }
```

## 7. Policy

`[policy]` holds the project-wide knobs. This is the main Orchard file
with step 6 merged in and every policy key below set.

- **`slots`** (default 1) is the number of tickets with a running
  attempt or a held resource. A ticket waiting at a human gate with
  no hold costs no slot. More slots run more tickets at once, and
  spend more at once.
- **`waiting_on_me`** (default 2) is the number of pending decisions
  across the project before nothing new starts. Running attempts
  finish either way. Raise it and more work piles up unanswered. Lower
  it and the project stops sooner when you are away.
- **`decisions`** sets each decision's dial: `ask`, `recommend` or
  `auto`. Unset is `ask`, and `recommend` is not built yet, so it
  behaves as `ask`. Use `auto` only where the answer is never in
  doubt: `lanes` with one lane, `review-code` with `style_rounds`.
- **`max_reruns`** (default 3) is the number of failed attempts a stage
  may collect in one context before the ticket parks instead of asking
  again. It stops a broken gate from spending all night.
- **`on_dirty`** is what an agent gets when it stops with its tree not
  clean: `{ nudge = N }` (the default is one nudge) types a line into
  its session up to N times before the question, and `"ask"` asks at
  once. A stage may set its own on an agent stage with a command gate
  or on a code review stage.
- **`min_free_gb`** (default 10) is the free space on the worktrees'
  volume below which nothing new starts. A build on a full disk fails
  for nothing and costs the run.
- **`evidence_file_mb`** (default 25), **`evidence_attempt_mb`**
  (default 200) and **`evidence_keep_days`** (default 30) govern
  evidence directories (section 5). A file over the first, or the files
  past the second, stay on disk but are not listed. The runner removes a
  closed ticket's evidence after the third, once a day, and logs
  `evidence-swept`; `dispatch close <ticket> --drop-evidence` removes it
  at once. All three are read from the ticket's copy.
- **`refresh`** (default true) brings each lane's branch up to its base
  when a stage begins, so a plan that sat is not implemented on stale
  code. A clean rebase is mechanical, and a conflict goes to the
  `rebaser`. Turning it off saves the rebases but merges conflicts
  later.
- **`trust_folders`** answers Claude Code's folder trust question,
  which every fresh worktree asks once. Without it, each new tree's
  first agent waits on that question.
- **`confine`** runs `setup`, command gates, review checks and command
  reviewers sandboxed to the ticket's trees, the attempt directory and
  each lane's `writable` paths, the caches its tools fill. `network`
  (`allow` or `deny`) says whether a confined command may reach off the
  machine; loopback stays open. A command gate's own `network`
  overrides it, as the deploy's does here, since it must reach AWS. It
  costs setup work: every cache a lane writes must be listed, and
  some suites cannot run confined at all. Switchboard's own tests start
  `/bin/ps` and make a keychain, which the sandbox refuses, so its
  file keeps `confine = false`.
- **`rates`** are dollars per million input and output tokens, per
  model, for budget reporting. Like `budget_usd`, they parse but
  nothing reads them yet.
- **`ports`** is the range services are allocated from (step 5).
- **`rebaser`**, **`max_rebases`**, **`fixer`**, **`max_fixes`** and
  **`resolution_reviewer`** are step 6's.

```toml
version = 1

[project]
name = "Orchard"
repo = "git@github.com:example-org/orchard-workspace.git"
space = "Dispatch · Orchard"

[source]
kind = "github"
repo = "example-org/orchard-workspace"
label = "dispatch"
lane_hints = { "area:backend" = "backend", "area:frontend" = "frontend", "area:admin" = "admin" }

[[lanes]]
name = "backend"
path = "orchard-backend"
repo = "git@bitbucket.org:example-co/orchard-backend.git"
base = "main"
setup = ["sh", "-c", "uv sync && uv run inv link-env --env-name my-dev"]
writable = ["~/.cache/uv"]

[[lanes]]
name = "frontend"
path = "orchard-frontend"
repo = "git@bitbucket.org:example-co/orchard-frontend.git"
base = "dev"
setup = ["npm", "ci", "--legacy-peer-deps"]
writable = ["~/.npm"]
serve = { argv = ["npm", "start"], env = { BROWSER = "none", PORT = "{port}" }, url = "http://localhost:{port}", ready = { http = "/", within_secs = 120 } }
merge_after = ["backend"]

[[lanes]]
name = "admin"
path = "orchard-admin"
repo = "git@bitbucket.org:example-co/orchard-admin.git"
base = "master"
setup = ["npm", "ci", "--legacy-peer-deps"]
writable = ["~/.npm"]
serve = { argv = ["npm", "start"], env = { BROWSER = "none", PORT = "{port}" }, url = "http://localhost:{port}", ready = { http = "/", within_secs = 120 } }

[[resources]]
name = "my-dev"
count = 1

[operators.investigator]
kind = "claude"
guidance = "Read the root CLAUDE.md and each repository's CLAUDE.md. Name the files, endpoints and screens the issue touches, and say which lanes it needs and why. Write nothing but your notes."

[operators.planner]
kind = "claude"
guidance = "Plan per lane: the change, the tests, the changelog entry, and how it is tried on my-dev."

[operators.reviewer]
kind = "codex"
guidance = "Hold the plan to guides/COLLABORATION_GUIDE.md."

[operators.reviewer.review]
reviewer = "codex"
review_first = "The plan at {plan} is for the repository at {worktree}. Check that the backend and frontend halves agree on the API. Write objections to {feedback}. If there are none, write exactly this line alone: {no_feedback}"
review_round = "The planner answered at {response} and updated the plan at {plan}. Reply at {feedback}. If you are satisfied, write exactly this line alone: {no_feedback}"
respond = "Feedback on your plan is at {feedback}. Edit the copy at {plan} and only that file, then answer each point at {response}."
respond_to_user = "The user says: {text}. Edit the copy at {plan} and only that file, then answer at {response}."
handoff = "The plan at {plan} was reviewed and is final."
no_feedback = "No further feedback."
cap = 4

[operators.implementer]
kind = "claude"
guidance = "Implement the lane's part of the finalized plan on branch {branch}, in this worktree only. Run the lane's checks before every commit. Never run a deploy."

[operators.lane-implementer]
kind = "claude"
args = ["--allowedTools", "Bash(uv run inv deploy:*)"]
guidance = "Implement the lane's part of the finalized plan on branch {branch}, in this worktree only. Run the lane's checks before every commit. You may deploy your lane to my-dev while you work; nothing you deploy is kept."
budget_usd = 30.0

[operators.code-reviewer]
kind = "claude"
guidance = "Hold the lane's branch to its repository's guides/COLLABORATION_GUIDE.md and to the finalized plan."

[operators.fixer]
kind = "claude"
guidance = "Address one round of review points on the branch. For each point, fix it or say in your response why not. Never run a deploy."

[operators.rebaser]
kind = "claude"
guidance = "Rebase the branch onto its moved base. Keep the change's intent on every conflict; when the base changed the same code for its own reasons, keep both. Run the lane's checks, then push."

[operators.tester]
kind = "claude"
env = ["aws-dev"]
guidance = "The backend is already deployed to my-dev and the frontends are already served; do not deploy or start a server. Never print a credential."

[[stages]]
name = "investigate"
operator = "investigator"
context = "root"
writes = ["notes"]
prompt = "Issue #{issue.number}: {issue.title}\n\n{issue.body}\n\nThe lanes are {lanes.all}. Investigate and write your findings, including the lanes needed, to {notes}."

[[stages]]
name = "lanes"
gate = { kind = "human", decision = "lanes" }

[[stages]]
name = "plan"
operator = "planner"
context = "joined"
writes = ["plan"]
prompt = "Using {inputs.investigate.notes}, write one plan covering the lanes {lanes} of issue #{issue.number} to {plan}."

[[stages]]
name = "review"
review = "reviewer"
context = "joined"
subject = "plan"
gate = { kind = "external", check = "review-finalized" }

[[stages]]
name = "implement"
operator = "lane-implementer"
context = "each"
needs = ["my-dev"]
prompt = "The plan at {inputs.plan} is final. Implement the {lane} part on {branch}, and commit. You hold my-dev while you work."
gate = { kind = "command", in = "lane", per_lane = { backend = ["sh", "-c", "uv run inv lint && uv run inv pytest"], frontend = ["sh", "-c", "CI=true npm test -- --watchAll=false"], admin = ["sh", "-c", "CI=true npm test"] } }

[[stages]]
name = "review-code"
context = "each"
reviewers = ["code-reviewer"]
implementer = "fixer"
on_dirty = "ask"
gate = { kind = "command", like = "implement" }

[[stages]]
name = "deploy"
context = "lane:backend"
without_lane = "base"
needs = ["my-dev"]
env = ["aws-dev"]
gate = { kind = "command", in = "lane:backend", network = "allow", argv = ["sh", "-c", "uv run inv link-env --env-name my-dev && uv run inv deploy -f"] }

[[stages]]
name = "deploy-check"
context = "root"
needs = ["my-dev"]
gate = { kind = "command", in = "root", network = "allow", argv = ["sh", "-c", "curl -fsS --retry 5 --retry-delay 10 https://api.my-dev.example.com/health"] }

[[stages]]
name = "try"
operator = "tester"
context = "joined"
needs = ["my-dev"]
services = ["frontend", "admin"]
before = { frontend = ["npm", "run", "link-env"], admin = ["npm", "run", "link-env"] }
writes = ["notes", { name = "evidence", dir = true }]
prompt = "my-dev runs backend commit {inputs.deploy.commit}. The frontend: {services.frontend}. The admin frontend: {services.admin}. The plan is at {inputs.plan}. Try issue #{issue.number} end to end. Write to {notes}, first line the result, then the commands and their output. Save screenshots and any file you produce under {evidence} with the Write tool, not cp or mv."

[[stages]]
name = "tried"
needs = ["my-dev"]
gate = { kind = "human", decision = "tried", confirm = true }

[[stages]]
name = "pr"
operator = "implementer"
context = "each"
writes = ["notes"]
prompt = "Push {branch} and open a pull request against the lane's base with a self-contained body. {lane.merge_after} Write its id to {notes}."
gate = { kind = "command", in = "lane", argv = ["sh", "-c", "test -z \"$(git status --porcelain)\" && git fetch -q origin && test \"$(git rev-parse HEAD)\" = \"$(git rev-parse \"origin/$DISPATCH_BRANCH\")\""] }

[[stages]]
name = "ready"
context = "each"
gate = { kind = "external", check = "pr-checks", provider = "bitbucket" }

[[stages]]
name = "merge"
context = "each"
gate = { kind = "external", check = "pr-merged", provider = "bitbucket" }

[policy]
slots = 2
waiting_on_me = 2
decisions = { lanes = "ask", finalize = "ask", review-code = "auto" }
max_reruns = 3
on_dirty = { nudge = 1 }
min_free_gb = 10
evidence_keep_days = 30
refresh = true
trust_folders = true
confine = true
network = "deny"
ports = [3100, 3199]
rates = { "claude-sonnet-5" = [3.0, 15.0], "claude-opus-5-5" = [15.0, 75.0], "gpt-5-codex" = [1.25, 10.0] }
rebaser = "rebaser"
max_rebases = 2
fixer = "fixer"
max_fixes = 2
resolution_reviewer = "code-reviewer"
```

**What is read live and what is copied at take.**

| Key | Read from | An edit reaches |
|---|---|---|
| `[policy] slots`, `waiting_on_me`, `min_free_gb` | the live `<project>.toml`, for the whole project, on every pass | every ticket, at the next pass |
| `[supervisor] decides`, `may` | only the live `<project>.toml`, on every supervisor command | the running supervisor, at its next command |
| the rest of `[supervisor]` | only the live `<project>.toml`, when the seed and settings are written | the supervisor, when it is next started fresh |
| a code review stage's `style_rounds` | the live file (`<project>.pr.toml` for a ticket from a pull request), every round | a running review, at its next round |
| everything else, `refresh` included | the ticket's copy, made at take | tickets taken afterwards, and a running ticket after `dispatch restart <ticket>` |

To apply a fixed gate to a ticket that already failed on the old one,
run `dispatch restart <ticket>`, then answer its `rerun` question
`check` to run the new checks on the same work. Answering `check`
without the restart runs the old copy.

## 8. The supervisor block

A project may have one supervisor: a long-lived Claude Code session
that watches the project's tickets and answers the decisions the owner
lets it. Its table is read only from the live `<project>.toml`, never
from a ticket's copy or the `.pr.toml`. See
[Supervisor](dispatch.md#supervisor).

```toml fragment
[supervisor]
guidance = "Keep Orchard's queue moving. The owner reviews every plan that touches the backend's API and merges every pull request."
read = ["CLAUDE.md", "guides/COLLABORATION_GUIDE.md"]
setup = ["git", "clone", "git@github.com:example-org/orchard-workspace.git", "."]
model = "sonnet"
decides = ["finalize", "rerun", "merge", "lanes", "tried"]
merges = false
may = ["restart"]
```

- **`guidance`** is what the supervisor is for, in your words. The seed
  opens with it. A running supervisor picks up a change to it only
  when it is started fresh. It must not be empty.
- **`read`** lists files to read first, relative to its workspace. An
  absolute path is refused.
- **`setup`** makes the workspace: one argv, or `[[supervisor.setup]]`
  entries with an `argv` each for several. Absent, it is
  `git clone <repo> .` for a project with a `repo`.
- **`model`** is the session's `--model`.
- **`decides`** lists the decisions the supervisor may answer. Every
  other decision is yours. A name is either one Dispatch asks of its
  own accord (`finalize`, `paused`, `rerun`, `pr`, `branch`, `lanes`,
  `refresh`, `review-cap`, `review-code`, `message`, `resolution`,
  `lost-send`) or one a gate of this file asks. That is a human or
  external gate's `decision`, such as `tried`, or `merge` for a
  `pr-merged` gate that names none. A gate's decision is valid only
  while the file has that gate, so this fragment needs step 7's
  `merge` and `tried` stages. `decides` never takes an answer:
  `keep` or `check` in place of `rerun`, `recheck` in place of `pr`,
  `refresh` or `merge`, and `proceed` in place of a human gate's
  decision are refused, naming the decision to use.
- **`merges`** (default false) lets the supervisor merge a pull request
  itself once its checks pass and its body is clean, by giving it
  allow rules for `gh pr` and `git pull`. Orchard's pull requests are
  on Bitbucket and merged by the owner, so it stays false.
- **`may`** lists capabilities beyond decisions. `runner` lets it stop,
  start and restart the runner. `restart` lets it run
  `dispatch restart <ticket> [<stage>]` on its own project's tickets.
  Omit the key rather than write it empty.

## 9. Review checklist

Each row is a key from the steps above, and what goes wrong without it.

| Key | Without it |
|---|---|
| `[project] base`, a lane's `base` | the lane branches from `main` (or the project's `base`), which may not be its integration branch |
| `[source] lane_hints` | every multi-lane ticket asks which lanes, with no suggestion |
| a lane's `setup` | the first agent installs dependencies itself, and gates fail on a cold tree |
| a lane's `serve` | a stage cannot serve that lane; `services` naming it is refused |
| a lane's `writable` | under `confine`, the lane's tools fail writing their caches |
| a lane's `merge_after` | lanes merge in any order, and a frontend can land before the endpoint it calls |
| `[[resources]]` with `count = 1` | two tickets deploy to one stack and overwrite each other |
| an operator's `guidance` | every prompt starts cold, with no project rules |
| an operator's `args` | the agent runs with default flags and asks before every tool its rules do not allow |
| an operator's `env` | the agent has no credentials, and looks for them on disk |
| an operator's `budget_usd` | nothing today: budget reporting is not built |
| a stage's `writes` | later stages have no `{inputs.<name>}` to read |
| `writes = [{ name = "evidence", dir = true }]` | screenshots and rendered files land in the worktree or `/tmp`, and are gone with the tree |
| `evidence_keep_days` | a closed ticket's evidence is removed after 30 days |
| a command `gate` on an agent stage | the stage completes when the agent stops, tests failing or not |
| `per_lane` | one command must suit every lane's toolchain |
| `like = "implement"` | the review's accepted head is never checked |
| `reviewers`, `implementer` | the branch is published unreviewed |
| `cap` | three passes, then a `review-cap` question |
| `style_rounds` | wording points keep the review going until the cap |
| `commits` | the fix rounds stay as separate commits |
| `review-code = "auto"` | every round with open points is a question |
| `needs` | the stage runs while another ticket owns the stack |
| `without_lane = "base"` | a ticket without that lane is tried against whatever the stack last held |
| a deploy-check stage | a deploy that exited 0 against a stack that does not answer goes to the tester |
| `services`, `before` | the tester starts its own servers, on ports and links of its guessing |
| a stage's `env` on a command gate | the gate runs with only the runner's environment |
| `[policy] ports` | `services` is refused |
| `provider` | the provider is guessed from the lane's remote |
| `checks = "none"` | a repository without CI asks about missing checks every time |
| `rebaser` | every conflicting pull request is a question |
| `fixer` | every red pull request is a question |
| `resolution_reviewer` | the last code review stage's first non-`style` reviewer checks a resolution |
| `slots` | one ticket at a time |
| `waiting_on_me` | the project stops starting work at two pending decisions |
| `max_reruns` | three failures, then the ticket parks |
| `on_dirty` | one nudge, then a question |
| `min_free_gb` | nothing new starts below 10 GB free |
| `refresh` | (default on) turning it off implements plans on stale branches |
| `trust_folders` | each fresh tree's first agent waits on the folder trust question |
| `confine`, `network` | setup and gates can write anywhere the runner can and reach anything |
| `rates` | nothing today: budget reporting is not built |
| `[supervisor] decides` | every decision is the owner's |
| `[supervisor] merges` | the supervisor reports a green pull request and stops |
| `[supervisor] may` | runner and ticket restarts stay the owner's |
