# Plan trial

Does Dispatch's planning process (investigate, plan, a plan review by
rounds) produce a better plan than one Claude session asked for a plan,
and how much of that holds at a cheaper model? This directory runs the
trial on a second Mac that already runs Switchboard. It is written as a
runbook for whoever operates that Mac, agent or owner.

## The design

Two public repositories, one issue each, twelve plans per issue:

| arm | what runs | models |
|---|---|---|
| pipeline | investigate, plan, review rounds (cap 6), stops at `finalize` | opus, sonnet, haiku |
| single | one headless Claude Code session, the issue plus the planner's guidance, no review | opus, sonnet, fable |

Every arm runs twice (`a` and `b`), so the two repetitions of one arm
measure consistency. That is 12 Dispatch tickets and 12 single sessions
in all. Implementation is a later phase and continues from these
tickets, so nothing here is closed when planning is done.

What stays fixed across the twelve plans of an issue: the base commit
(`take.sh` records it and `single.sh` checks it out), the planner's
guidance (`prompts/<repo>.md` is the pipeline file's text, verbatim), and
the issue text as `gh issue view` returns it. The pipeline arms differ
from the live pipeline only as the comment at the top of
`pipelines/<repo>.toml` says: no supervisor, one slot, a cap of six
review rounds, the model set on the investigator, planner and reviewer,
and `trust_folders` on. The `--model` flag reaches the review's reviewer
session through the operator's `args`, and a resumed Claude Code session
keeps the model it was started with, so the planner answers feedback at
its arm's model too.

Default issues, chosen for being medium sized and self-contained:
Switchboard #187 and simplesingletable #13. Any other open issue works
the same way.

## Before the first issue

On the Mac that runs the trial:

1. Switchboard is running and its Dispatch page's runner is started.
   `dispatch health` answers. The `dispatch` binary is the one in
   `~/Applications/Switchboard.app/Contents/MacOS`; the scripts find it
   there when it is not on `PATH`.
2. `claude` is logged in, `gh auth status` is good, `tmux`, `cargo` and
   `uv` are installed. Docker is not needed: nothing here runs tests.
3. Clone this repository on branch `plan-trial` and work from this
   directory:

   ```sh
   git clone -b plan-trial https://github.com/msull/switchboard.git ~/code_repos/personal/switchboard
   cd ~/code_repos/personal/switchboard/experiments/plan-trial
   ```

4. Build the `dispatch` CLI from this branch, which takes an issue once
   per project; the 0.4.0 release refuses an issue any project already
   holds. `take` runs in the CLI process, so the app's bundled runner
   drives the tickets unchanged:

   ```sh
   (cd ../.. && cargo build --locked --release -p dispatch)
   ```

   The scripts use `target/release/dispatch` when it exists.

5. Write the twelve trial projects into Dispatch's data directory:

   ```sh
   ./make-arms.sh
   ```

   It prints the twelve files. Their names are `sb-<model>-<a|b>` and
   `sst-<model>-<a|b>`. All six of a repository's projects share one
   Switchboard space, `Plan trial · Switchboard` or
   `Plan trial · simplesingletable`.

## Running one issue

Take the issue in all six pipeline arms at once. The script records the
base commit first, so the single sessions later plan against the same
main:

```sh
./take.sh switchboard 187
```

Each project then clones the repository, cuts a worktree, runs the lane
setup and starts the investigator. Follow along with `dispatch status`
and `dispatch decisions`; the Switchboard window shows the sessions
under the trial space. A ticket's path is investigate, lanes (answered
automatically), plan, review rounds, then a `finalize` question.

Decisions:

- **`finalize`**: never answer it. The ticket holding there is the end
  state of this phase; its plan and rounds stay on disk.
- **`rerun`**, asked when an agent's turn ended without its notes or
  plan file: answer `rerun` once per ticket, with the note
  `plan trial: one rerun allowed`, by copying the line
  `dispatch decisions` prints. If it fails again, leave the question
  pending and record the arm as failed to produce.
- Anything else (`lost-send`, `refresh`): answer what the question
  offers to keep the ticket moving, with a note saying why, and mention
  it in the results commit message.
- Never park, close or restart a trial ticket, never answer `finalize`,
  and do not push to `main` of either repository while an issue's arms
  are running: `refresh` rebases a lane onto its base at each stage
  start, and a moved main would put the arms on different code.

While the tickets run, the six single sessions, in any order and as
many at once as the Mac is comfortable with:

```sh
for m in opus sonnet fable; do for r in a b; do ./single.sh switchboard 187 $m $r; done; done
```

Each clones the repository once under `~/code_repos/plan-trial`, cuts a
detached worktree at the base commit, and runs `claude -p` there with
the prompt in `results/sb-187/single/<model>-<rep>/prompt.md`. The run's
usage and cost land in `run.json` beside the plan. If a run exits at
once with a folder-trust message, open `claude` interactively in that
worktree once, accept, and run the script again.

When `dispatch decisions` lists a `finalize` question for all six arms
of the issue (or an arm is recorded as failed), gather everything:

```sh
./collect.sh switchboard 187
./blind.sh switchboard 187
```

`collect.sh` copies each arm's investigation notes, first plan, reviewed
plan with its feedback and response rounds, and the ticket's `report`
and `show` JSON into `results/sb-187/pipeline/<model>-<rep>/`.
`blind.sh` shuffles the twelve plans into `results/sb-187/blind/A.md`
to `L.md`, with the mapping in `key.txt` and a `scores.md` to fill in.

Commit the results to this branch and push, so the owner can score on
another machine. The pre-commit hook runs the test suite, which takes a
few minutes:

```sh
git add experiments/plan-trial/results
git commit -m "Plan trial: switchboard #187 planned in every arm"
git push
```

No tool attribution in that commit message, the same as everywhere else
in this repository.

Then the second issue:

```sh
./take.sh simplesingletable 13
for m in opus sonnet fable; do for r in a b; do ./single.sh simplesingletable 13 $m $r; done; done
```

and the same collect, blind, commit when its arms are done.

## Scoring

The owner's part. Pull the branch, open `results/<short>-<issue>/blind/`
and fill `scores.md` by `rubric.md` with `key.txt` unopened. Then open
the key and put each arm's measurements beside its scores: rounds and
whether the cap was hit from `report.json`, wall time, tokens by model,
and a single session's `total_cost_usd` and `duration_ms` from
`run.json`. Where the `a` and `b` repetitions of an arm disagree, say
so: that is the consistency result.

## Afterwards

The tickets stay as they are: implementation is phase two, and it starts
from these plans. Which implementer model runs which plan is decided
then; a ticket's pipeline copy is made at take, so cross combinations
(a strong plan with a weak implementer) need either an edit to the live
file before a `restart` or a new project whose plan stage copies a
chosen plan in. When the whole trial is over, `dispatch close` each
ticket, delete the twelve files under the data directory's `pipelines/`,
and remove `~/code_repos/plan-trial`.

## Files

| file | what it is |
|---|---|
| `pipelines/switchboard.toml`, `pipelines/simplesingletable.toml` | the live pipelines as trial bases, with `PROJECT` and `MODEL` to fill in |
| `prompts/<repo>.md` | the planner's guidance, verbatim from the base file, for the single arm |
| `make-arms.sh` | writes the twelve arm projects into Dispatch's `pipelines/` |
| `take.sh` | records the base commit and takes an issue in a repository's six arms |
| `single.sh` | one headless Claude Code plan at the base commit |
| `collect.sh` | copies an issue's pipeline arms out of Dispatch's data directory |
| `blind.sh` | shuffles the twelve plans into A to L with a key and a score sheet |
| `rubric.md` | how to score a plan, and what to put beside the scores |
| `lib.sh` | shared by the scripts: the `dispatch` binary, the data directory, the repositories |
| `results/` | one directory per issue, committed to this branch |
