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

## 2026-10-22: revision 2, eight runs and a transfer check

- Four more isolated facts runs on #230 (sonnet c, d; haiku c, d). Over
  all eight: seven found the WebKit bug, seven wrote a build line the
  planner can use (six name polling), sonnet four of four on both at
  $0.48 to $0.56, haiku three of four at $0.19 to $0.35 with one miss
  (haiku-a) and one weak line (haiku-c). Two sonnet runs filed the
  sources under Settled and the question under Unresolved. Sonnet runs
  1907 to 2072 words, over the cap; haiku 1325 to 1671.
- Transfer check on #187 with revision 2, from the same base commit:
  the investigator wrote seventeen open questions, seven tree, two
  mislabelled source (in-tree Switchboard code), three run, five owner
  that were all design choices. The facts stage then ran no search,
  said so in its first paragraph, settled the tree questions with line
  references that match the reference (the Complete filter in
  review_subject, nothing removes the failed copy, no carry for a
  workflow stage), and cost $0.27 in 61 seconds. It restated the
  issue's design decisions under For the owner, as every #230 run did.
- stage.py named its branch by stage and label only, so the #187 run
  collided with the #230 run's branch; the branch now carries the
  issue.
- Revision 3 (`pipelines/switchboard-facts-v3.toml`): the two phrases
  naming a page and a browser are general; an owner question is one
  only the owner can observe and a design choice is excluded, in the
  investigator's tag and the facts file; an empty list is a fine
  answer; a settled question is not listed under Unresolved as well.
  Not yet run.

## 2026-10-22: revision 3, first runs ($2.08 of the $50 budget so far)

- The owner granted a $50 budget for stage-level iteration after
  revision 2; `results/LEDGER.md` says what is counted and `spend.py`
  sums it from the runs' own reports.
- Four facts runs on #230 (sonnet a, b; haiku a, b; $0.19 to $0.41):
  all four found the WebKit bug. sonnet-b and haiku-b filed it under
  Settled cleanly; sonnet-a filed it Settled as "may not cover" and
  again as unresolved for current iOS; haiku-a misread the bug as
  about logging and filed it a lead. Build lines: polling fallback in
  three, a SAN certificate "works either way" in haiku-a. Owner
  sections fell to 150 to 310 words from 400 to 600; files 1126 to
  1785 words, one still over the cap.
- #187 under revision 3: the investigator wrote ten open questions,
  six tree, one source, one run, two owner, both observations (is the
  revised file still on disk; what text the owner saw). The facts stage
  ran no search, settled the tree questions in 68 seconds for $0.30,
  and its owner section holds those two questions with both branches.
  The design-choice drift is gone on this issue.
- Considered, not taken yet: a second facts finder on another model
  with a resolver folding the two files. Held until the single sonnet
  run's consistency is known over four repetitions; the trap test
  (revision 2 sonnet-b's false claim against haiku-b's true one) is
  the thing that would decide it.

## 2026-10-22: revision 3's plans, graded ($12.5 of $50 so far)

- Two more sonnet facts runs (c, d): both found and settled the WebKit
  bug, so sonnet is four of four on revision 3; 1303 and 2044 words.
- Four plan-stage runs from the sonnet-b facts file and opus-a's notes
  (sonnet a, b at about $1.00 and five minutes; haiku a, b at about
  $0.40 and seven minutes; 3481 to 4181 words). Every one is
  poll-first with the socket as the upgrade, which none of the
  twenty-two earlier plans was. sonnet-b's plan opened by contradicting
  the facts file's false Settled line (the Stop hook's message on the
  record) with the tree; haiku-a corrected two facts line numbers;
  sonnet-a believed the false line.
- Graded by both graders against the same reference
  (`results/sb-230/stages/plan/v3-*/grade.*.md`): Fable 3/3/1/2/note
  for three and 3/2/1/2/note for sonnet-b; Codex 3/2/0/1/no for all
  four. That equals the best rows of the first trial (pipeline opus-b,
  single sonnet-a) and the four are near-identical to each other,
  which the first trial's a and b pairs never were.
- The two decisions every plan lost are the same two: a certificate
  reissue with SAN, EKU and a bounded validity plus identity fields
  and renewal (the reference keeps the identity model), and a token in
  localStorage with a subprotocol carrier because the cookie's
  behaviour on a WebSocket had an unresolved line (the reference
  takes the cookie). Both follow from the planner's rules as written:
  the facts files said the reissue "works either way", and the
  robust-default rule read an unresolved cookie mechanic as a reason
  to pick the weaker store.
- Revision 4 (`pipelines/switchboard-facts-v4.toml`, planner only): a
  decision the spec already made stands unless the issue asks to
  change it; where a credential lives is a security decision, the
  safer store wins, and its unresolved mechanic is the implementer's
  first test, not a reason to ship the weaker one. Not yet run.
- The resolver idea (two facts finders, one resolver): not taken for
  now. The single sonnet run is four of four on the central fact, so
  recall is not the problem; the one facts error that reached a plan
  was caught by two planners of three. Revisit if an error reaches a
  graded plan.
- Review stage, round one of revision 3's template, running on the
  sonnet-b plan with sonnet twice and opus once.

## 2026-10-22: the review round under revision 3

- Round one of the revision 3 review template on the sonnet-b plan,
  with sonnet twice and opus once
  (`results/sb-230/stages/review/v3-*`). sonnet-a: six objections,
  1035 words, $1.50, no search; sonnet-b: one objection, 228 words,
  $0.47, no search; opus-a: three objections, 574 words, $0.27 at the
  reported rate, two searches and three fetches.
- All three caught the same false premise, that load_identity refuses
  a newer file, which the plan used to justify its identity schema
  bump; opus-a also caught that not_after alone does not bound
  validity, and the .local name with no source. Every objection is
  inside the certificate reissue's details; none objects to the
  reissue as a decision the spec's identity model rules out, and none
  mentions the token store. So the review corrects premises but has
  no yardstick for a decision the spec already made, and the two
  reference overturns pass through it untouched.
- Revision 4 now carries the planner's two limits into the review's
  first round as objection classes: a spec decision changed without
  the issue asking, and a credential kept where script can read it.
- Reviewer consistency is poor at sonnet: one run wrote six
  objections and another one, from the same plan and template, at
  three times the cost. Worth a second look once revision 4 runs.
- A first attempt at these runs launched nothing: a zsh loop did not
  split its model and repetition pairs. No cost.

## 2026-10-22: does a better facts file make a better plan? ($23.88 of $50)

- The revision 3 planner on sonnet, held fixed, five times from each
  of two revision 2 haiku facts files: haiku-a, which never found the
  WebKit bug and wrote no build line, and haiku-b, which filed it as
  settled and also found the SameSite cookie bugs
  (`results/sb-230/stages/plan/v3fa-*` and `v3fb-*`; $0.67 to $1.27 a
  plan, 3134 to 4123 words).
- From the file that missed the fact: zero of five plans are
  poll-first. All five open the socket and wait for hello as §4.4
  says; three add a polling fallback for a refused socket, two have no
  polling at all. None cites the WebKit bug.
- From the file that found it: four of five are poll-first with the
  socket as the enhancement, and the fifth opens the socket and falls
  to polling after five seconds. Four of five cite the bug and four
  carry the SameSite finding, which every one used to argue for the
  subprotocol token over the cookie.
- So the facts file drives the plan: the planner does not hedge on
  its own, and one settled line in the file is the difference between
  a page that works on the phone and one that does not. A second facts
  finder or a rectifier is therefore worth exactly what it adds to
  recall. With sonnet four of four on revision 3 that is little; with
  haiku three of four it is one plan in four. The cheap rectifier is
  running the facts stage on sonnet.
- A true finding that cuts against the reference: the SameSite cookie
  bugs in restored Safari tabs are real, and four plans took them as a
  reason to keep the token out of a cookie, which the reference calls
  the decision to reverse. The reference's cookie decision may deserve
  an errata note weighing that bug; the owner's call.
- Revision 4 planner, four runs from the revision 3 sonnet-b facts
  file, launched for comparison with the four revision 3 plans.

## 2026-10-22: revision 4's plans

- Four plans from the revision 3 sonnet-b facts file under the
  revision 4 planner (sonnet $0.94 and $1.17; haiku $0.38 and $0.38;
  4405 to 4909 words, up from 3481 to 4181 under revision 3).
- The credential rule held in all four: the token lives in an
  HttpOnly; Secure; SameSite=Strict cookie named by server id, no
  localStorage, and no second transport built; sonnet-b and haiku-b
  say in so many words that none is added. All four stay poll-first.
- The certificate reissue stayed in all four. The planners do not read
  it as changing a spec decision: they read the facts file's Apple
  rules as "the certificate as built cannot be trusted by a browser"
  and the reissue over the same key as the default that works either
  way, with the SPKI pin unchanged. Whether that is over-building (the
  reference's view) or the identity model kept with a renewed
  certificate is a judgment for the owner, and the guidance should not
  be tuned further toward the reference on it until the owner says.
- Graded by both graders, and the revision 4 review round run on the
  sonnet-b plan with sonnet twice and opus once, for comparison with
  revision 3's.
