# Plan trial journal

One entry per thing that happened, newest last, so the write-up can be
assembled from this file and the directories it names. Dates are when
the step ran. Costs are list price from the runs' own reports.

## 2026-10-21: design and the first issue

- The question: does Dispatch's planning pipeline (investigate, plan,
  plan review by rounds) produce a better plan than one Claude session
  asked for a plan, and how much of that holds at a cheaper model?
- Design: an issue planned twelve times. Pipeline arms at opus, sonnet
  and haiku; single `claude -p` sessions at opus, sonnet and fable;
  every arm twice (a, b). Commit 92a90e3 holds the runbook and scripts.
- Dispatch's `take` had to learn to take one issue once per project
  rather than once per data directory, so six projects could hold the
  same issue (495852c; shipped as Switchboard 0.4.1).
- Switchboard #187 (a small Dispatch bug) planned in every arm
  (4f07630) and graded (e7f6676): `results/sb-187/`. Every plan found
  the bug; the pipeline did not beat a single session and cost two to
  four times as much. Separation came only from decisions and
  over-specification.
- simplesingletable #13 planned in every arm (a18f2d1): spoiled, the
  issue was already fixed on main before the base commit and every
  plan says so. Kept unscored under `results/sst-13/`.

## 2026-10-22: the second issue, #230

- Issue #230, the web control surface on the remote port, written as a
  larger feature with six decisions a plan must settle (f062730).
- Planned in every arm (dcf60c7), graded (c71e289):
  `results/sb-230/scores.md`. Again no arm separated. No plan knew
  that WebKit refuses a WebSocket to a click-through certificate, the
  reference's central fact; the plans that hedged with polling scored
  best. Review rounds added 20 to 50 percent length and never a fact.
  Eleven plans found a gap in the reference (the card excerpt is not on
  the wire), recorded as errata in `reference.md`.
- Grading method, used for both issues: the owner writes
  `reference.md` from the base commit before reading any plan; a Fable
  grader and a Codex grader score each blind plan from one packet
  (issue, rubric, reference, plan, base worktree); `tally.py` joins
  grades to cost and time; disputes are adjudicated in
  `grades/overrides.txt`.
- Diagnosis: no stage had web access, and iteration without
  information cannot add a fact. Decision: a facts stage.

## 2026-10-22: the facts pipeline, batch one

- `pipelines/switchboard-facts.toml` (6728f63): the investigator ends
  with tagged open questions; a facts stage with web search, fetch and
  a spike settles them; the planner carries a fallback for unresolved
  lines; the review is two rounds, premises then cuts. Three
  scenarios (sonnet, opus, mixed) twice each.
- Planned on the trial Mac (a31dbcc): `results/sb-230/facts/`. One arm,
  mixed-b, committed a spike (`spikes/23-browser-cert`) measuring
  Apple's trust evaluation against four certificate shapes.
- Graded (33d9fb1). The first pass's packet lacked the facts file and
  Codex marked every citation into it wrong; kept under
  `grades/v1-without-facts/`, regraded with the file in the packet.
  Departures improved for opus and mixed; nothing else moved; cost
  about twice the first trial at the same model.
- Finding: four of six facts stages found the WebKit bug and filed it
  as unresolved ("not from Apple"); the planner's fallback rule then
  produced socket-first designs. The evidence standard and the
  fallback default are the two guidance changes to make. The reviewer
  never used its web tools.
- Batch two, haiku and cheap (haiku writing, sonnet checking), taken
  on the trial Mac (8a6812a).

## 2026-10-22: stage-level iteration

- Decision: iterate on one stage at a time rather than rerun the
  pipeline. `stage.sh` reproduces what Dispatch hands a stage session
  (guidance plus rendered prompt, operator args plus an edit rule,
  the lane worktree, input paths in the environment) and runs it as
  `claude -p` from a finished run's inputs. Runs land under
  `results/sb-230/stages/<stage>/<label>-<model>-<rep>/` with the
  prompt, argv, artifacts, transcript and cost.
- Noted while building it: the reviewer operator's `guidance` is never
  sent to the reviewer; only the review templates are.
