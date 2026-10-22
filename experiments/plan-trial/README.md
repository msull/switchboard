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
| facts | investigate, facts (web search, fetch, a spike when needed), plan, review rounds (cap 2), stops at `finalize` | three scenarios, below |

Every arm runs twice (`a` and `b`), so the two repetitions of one arm
measure consistency. That is 12 Dispatch tickets and 12 single sessions
for the first two arms, and 6 more tickets for the facts arm, which was
added after #230 was scored (see "The facts pipeline" below). Implementation is a later phase and continues from these
tickets, so nothing here is closed when planning is done.

What stays fixed across the twelve plans of an issue: the base commit
(`take.sh` records it and `single.sh` checks it out), the planner's
guidance (`prompts/<repo>.md` is the pipeline file's text, verbatim), and
the issue text as `gh issue view` returns it. The pipeline arms differ
from the live pipeline only as the comment at the top of
`pipelines/<repo>.toml` says: no supervisor, two slots, a cap of six
review rounds, the model set on the investigator, planner and reviewer,
and `trust_folders` on. The `--model` flag reaches the review's reviewer
session through the operator's `args`, and a resumed Claude Code session
keeps the model it was started with, so the planner answers feedback at
its arm's model too.

Issues so far: Switchboard #187 (a small Dispatch bug; the first run,
scored under `results/sb-187/`), simplesingletable #13 (spoiled: the
issue was already fixed on main before the trial's base commit, and
every plan says so; its tickets and results stay as a record and are
not scored), and Switchboard #230 (a feature, the web control surface,
chosen because #187 was too small to separate the arms). Any other open
issue works the same way.

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

4. Switchboard on this Mac is 0.4.1 or newer (the Settings menu shows
   the version; the app updates itself from GitHub releases). That
   release's `dispatch` takes an issue once per project; 0.4.0 refused
   an issue any project already held.

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

Then the second Switchboard issue. The #187 tickets hold at `finalize`
and each keeps its arm's slot, so the trial file now says `slots = 2`;
write the arm files again first so the live copies pick it up (a ticket
already taken keeps its own copy, and the scheduler reads `slots` from
the live file):

```sh
./make-arms.sh
./take.sh switchboard 230
for m in opus sonnet fable; do for r in a b; do ./single.sh switchboard 230 $m $r; done; done
```

and the same collect, blind, commit when its arms are done, under
`results/sb-230/`. #230 is a feature, so expect longer plans and more
review rounds than #187 took; the cap of six still applies. Leave the
simplesingletable arms alone.

## The facts pipeline

What the first two issues showed: review rounds added words but never
a fact, no stage had web access, and the plans that scored best on
#230 were the ones that hedged the one external question (whether a
browser keeps a WebSocket to a click-through certificate) with a
fallback. The owner's reference settled that question with two
searches. So `pipelines/switchboard-facts.toml` is the live pipeline
with one stage added and the review changed, as its header comment
says: the investigator ends with tagged open questions; a `facts`
stage between `lanes` and `plan` settles them and the issue's claims
about the world outside the tree, with `WebSearch` and `WebFetch` in
its `args`, a closer read, or a spike under `spikes/` in the shape of
the ones there, committed on the branch; the planner treats the facts
file as the yardstick and writes a fallback beside any decision that
rests on an unresolved line; and the review has a cap of two rounds
with web tools, premises in round one and cuts in round two.

Three scenarios, each run twice, so six more plans of the same issue
against the same base commit:

| scenario | investigate | facts | plan | review | compares with |
|---|---|---|---|---|---|
| `sonnet` | sonnet | sonnet | sonnet | sonnet | pipeline sonnet, single sonnet |
| `opus` | opus | opus | opus | opus | pipeline opus |
| `mixed` | sonnet | opus | sonnet | opus | both: the money goes to the two stages that gather and check information |

The projects are `sbf-<scenario>-<a|b>` in their own space,
`Plan trial · Switchboard facts`. Running them, on the trial Mac with
the branch pulled:

```sh
./make-arms.sh                      # writes the six sbf-* files beside the twelve
./take.sh switchboard 230 facts     # refuses if main has moved off results/sb-230/base.txt
```

The same decisions rules apply: never answer `finalize`, one `rerun`
per ticket. Two things are new to watch:

- **The facts stage may spike.** It runs in the lane and may build a
  throwaway crate under `spikes/`; its gate is a clean tree, so an
  uncommitted spike gets the dirty-tree nudge and then a question.
  `dispatch tail` shows what it is doing. A facts stage that writes a
  one-paragraph facts file because nothing was open is a valid result,
  not a failure.
- **Permission prompts.** The facts operator's `args` allow `WebSearch`,
  `WebFetch`, `cargo` and `git add`/`commit`; the reviewer's allow the
  two web tools. Anything else an agent reaches for asks in its pane,
  as in the first trial. If a facts session sits at a prompt, answer it
  and note which tool in the results commit message.

When all six wait at `finalize`:

```sh
./collect.sh switchboard 230 facts   # results/sb-230/facts/<scenario>-<rep>/, with facts.md and spikes.txt
./blind.sh switchboard 230 facts     # adds the six as M..R to blind/, appending the key
git add experiments/plan-trial/results
git commit -m "Plan trial: switchboard #230 planned in the facts arms"
git push
```

Scoring is then `grade-all.sh` as before: it grades M to R against the
same reference and skips A to L, whose grades exist, and `tally.py`
puts the facts arms in the same table with their rounds (cap 2) and a
`spike` mark where one was committed. The reference's errata section
stays as it is: the facts arms are graded by the same yardstick as the
twelve, including its one known gap.

Not in this cut: a human gate after `facts` for the questions only the
owner can answer. The trial runs unattended, so the facts file's "For
the owner" section is read by the planner as unresolved, with the
fallback it names. The live pipeline can add the gate afterwards.

## Scoring

First the owner writes `results/<short>-<issue>/reference.md` from the code at base, before reading any plan: departures from the issue's framing, the decisions a plan must get right, and the anchors it should cite. Then `grade-all.sh` scores every blind plan with two graders and `tally.py` writes `scores.md`; the owner adjudicates where the graders disagree, recording any correction in `grades/overrides.txt`.

The manual alternative. Pull the branch, open `results/<short>-<issue>/blind/`
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
| `pipelines/switchboard-facts.toml` | the Switchboard pipeline with the facts stage and the two-round review, with `PROJECT` and a model per planning stage to fill in |
| `prompts/<repo>.md` | the planner's guidance, verbatim from the base file, for the single arm |
| `make-arms.sh` | writes the twelve arm projects and the six facts arms into Dispatch's `pipelines/` |
| `take.sh` | records the base commit and takes an issue in a repository's six arms, or with `facts` in the six facts arms |
| `single.sh` | one headless Claude Code plan at the base commit |
| `collect.sh` | copies an issue's pipeline arms, or with `facts` its facts arms, out of Dispatch's data directory |
| `blind.sh` | shuffles the twelve plans into A to L with a key and a score sheet; with `facts`, adds the six facts plans as M to R |
| `rubric.md` | how to score a plan, and what to put beside the scores |
| `base-tree.sh` | a worktree at an issue's base commit under `~/code_repos/plan-trial`, made on first use |
| `grade.sh` | scores one blind plan with one grader (`fable` or `codex`) from one packet: issue, rubric, reference, plan, base worktree; writes `grades/<letter>.<grader>.md` |
| `grade-all.sh` | every plan of an issue through both graders, four at a time, skipping grades already written |
| `tally.py` | joins grades to the key and the runs' measurements into `scores.md`; applies `grades/overrides.txt` |
| `lib.sh` | shared by the scripts: the `dispatch` binary, the data directory, the repositories, the facts scenarios |
| `results/` | one directory per issue, committed to this branch |
