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

## 2026-10-22: facts batch two, transcripts, and what they show

- The haiku and cheap arms planned on the trial Mac (a3abdcd), blinded
  as S to V, graded here: `results/sb-230/scores.md` has all
  twenty-two plans. haiku throughout: departures 1.8, overturn 0.0,
  $0.48, 29 minutes. cheap (haiku writing, sonnet checking):
  departures 2.0, $1.12, 16 minutes. Neither found the WebKit bug;
  all four built socket-first pages.
- Every session's transcript and launch record collected (5ecb607),
  after the trial Mac fixed collect.sh to match sessions by the
  Switchboard record id Dispatch names rather than the Claude Code id
  inside it. 79 MB, no file over 15 MB.
- Fidelity of `stage.sh` checked against opus-a's real facts session:
  the first message is identical after path normalization, and the
  launch argv matches rule for rule.
- From the transcripts: no reviewer in any of the ten facts arms made a
  WebSearch call (three WebFetch calls in total), so the reviewer's
  tools went unused. Facts-stage searches per arm: sonnet 0 and 1,
  cheap 0 and 0, opus 8 and 2, mixed 4 and 5, haiku 10 and 11. Sonnet
  fetches pages it already knows and does not search; haiku searches
  most and still did not find the bug.
- The next guidance revision, to be tried with `stage.sh` before any
  pipeline run: a [source] question requires a search with its own
  terms and the facts file lists the queries tried; the evidence
  standard names what settles an engineering decision (a vendor's open
  bug plus a project's documented workaround counts); an unfavourable
  unresolved fact makes the robust path the default; the reviewer's
  instructions move into the review template, since its guidance field
  is never sent.

## 2026-10-22: facts guidance revision 2, in isolation

- `pipelines/switchboard-facts-v2.toml`: the facts operator must search
  before fetching, record its queries, and treat a vendor's open bug
  plus a project's documented workaround as settling; an Unresolved
  line carries both branches and what the plan builds anyway; the
  planner's default under an unresolved fact is the design that works
  either way; the reviewer's instructions move into the review
  templates, since its guidance field is never sent; the investigator
  phrases outside questions in search terms.
- First four runs of the facts stage alone, from opus-a's
  investigation (`results/sb-230/stages/facts/v2-*`): sonnet a and b,
  haiku a and b. Searches per run went from 0 to 1 under revision 1 to
  10 to 16; every run lists its queries. Cost $0.19 to $0.56, two to
  four minutes, no spikes, every worktree clean.
- On the central fact: haiku-b filed "a click-through does not carry
  wss" under Settled with the WebKit bug and the list thread, the
  reference's position. sonnet-a put the two sources under Settled and
  the question under Unresolved with the right build line (socket
  optional, refetch on failure). sonnet-b filed it Unresolved with the
  right build line. haiku-a did not find the bug and gave no build
  line. So two of four reached the reference's design, one more
  reached its sources, and one missed.
- A wrong Settled line: sonnet-b says the Stop hook's last message is a
  field on the session record, citing the hook binary and a core test;
  the base tree has it only on the event. The kind of error a
  correctness pass over the facts file is for.
- New and true, not in the reference: haiku-b found WebKit bugs 200345
  and 232666, that a SameSite=Strict cookie can be missing from a
  WebSocket in a restored Safari tab.
- Length: sonnet 1907 and 2072 words, over the 1500 asked; haiku 1325
  and 1469.
