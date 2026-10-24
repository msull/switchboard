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

## 2026-10-22: revision 4's plans graded ($34.06 of $50)

- Grades (`results/sb-230/stages/plan/v4-*/grade.*.md`): sonnet-a
  Fable 3/3/2/2/note, Codex 3/2/1/1/note; sonnet-b Fable
  3/2/2/2/note, Codex 3/2/1/1/note; haiku-a Fable 3/2/1/2/note, Codex
  3/2/0/1/no; haiku-b Fable 3/3/1/2/note, Codex 3/2/0/1/no.
- The sonnet plans are the first in the trial to get a hand-off of
  note from Codex, and both graders write that everything else
  matches the reference. The one reversal left in all four is the
  certificate reissue, with haiku-a adding a certificate download
  path and both haiku plans a third static file. So revision 4's
  credential rule removed the token-store reversal in every plan, and
  the reissue stands as the owner's judgment call.
- Compared with revision 3 on the same facts file: Fable's overturn
  rose from 1 to 2 for sonnet (one reversal instead of two), Codex's
  hand-off moved from no to note for sonnet. Length rose by about a
  fifth.
- The owner left the SameSite errata to me: the reference now says the
  finding is true and does not change decision 2, since the page is
  poll-first and the cost of a missing cookie on a restored tab is a
  socket that falls to polling, while a script-readable token is a
  standing exposure; SameSite=Lax with the Origin check is acceptable.

## 2026-10-22: the revision 4 review round, revision 5, and the reassembled pipeline ($35.67 of $50)

- Round one of revision 4's review template on the revision 4
  sonnet-b plan: sonnet-a one objection ($1.61, no search); sonnet-b
  three objections ($0.29, one search); opus-a four objections
  ($0.35, two searches). The new objection class is used: sonnet-b
  and opus-a each object that the plan changes a decision the spec
  made (the §5 pairing sentence; the card excerpt). Neither names the
  certificate reissue as one, so the reissue passes the reviewer for
  the same reason it passes the planner: the facts file's Apple rules
  make it read as required.
- One conflict found: sonnet-b objects that the cookie "rests on an
  unresolved line with no fallback", which is the default the
  planner's credential rule asks for. Revision 5 adds the converse to
  the reviewer's objection list: a safer store chosen while its
  mechanics are unresolved is right if the plan names the test.
- Reviewer cost at sonnet still varies five-fold between runs of the
  same plan and template.
- The reassembled pipeline is revision 5, run as `sbf5-*` arms in
  three scenarios: lean (haiku investigate, sonnet facts, sonnet plan,
  sonnet review), sonnet throughout, checked (sonnet with the opus
  reviewer). The scripts take, collect and blind the set; blind
  letters continue past Z.
- The respond step (the planner answering a review) run once as a
  mechanics check, on the revision 4 sonnet-b plan with opus-a's
  objections.
- The respond step works as a mechanism: $0.27 and 52 seconds on
  sonnet; the planner accepted all four of opus-a's objections after
  checking each in the tree, edited the plan copy (54 changed lines,
  4560 to 4750 words) and answered each point naming the section
  changed. In the pipeline the planner's own session is resumed
  instead, so this is a lower bound on how well it answers.
- Spend at the end of the stage-level phase: $35.94 of $50. The
  remainder is held for grading the reassembled pipeline's six plans
  when the trial Mac returns them.
- The owner added two scenarios to the revision 5 set: opus throughout
  and fable throughout, the latter to see how the strongest model
  handles each stage. Ten sbf5 arms in all.

## 2026-10-22: the revision 5 arms graded ($53.15 of $50)

- The trial Mac returned the ten `sbf5-*` arms (commit 5ccb675),
  blinded W to AF. Two arms, sonnet-a and checked-a, needed the one
  allowed rerun: Claude Code 2.1.309 auto-updated mid-batch and its
  first-run dialog ("Teach auto mode about your environment?")
  swallowed the round-2 reviewer prompt, which Dispatch's workflow
  does not detect (it fails a round only on a Stop after the ask).
  Their attempt-1 transcripts are kept beside attempt 2's. The
  tally's "3 cap" on checked-a is Dispatch's count across both
  attempts; the graded plan had two rounds.
- Both graders on all ten, $14.21 at Fable and five Codex runs
  refused by a usage limit and rerun an hour later. Spend ends at
  $53.15 against the $50, the overrun being the four opus and fable
  arms the owner added knowing the cost would stretch.
- What the plans did. Ten of ten put the token in an `HttpOnly`
  cookie, no `localStorage`, no second transport: the revision 4
  planner rule held in every arm. Ten of ten build the page to work
  without the socket; nine poll first (sonnet-b polls only and defers
  the socket, which the Fable grader reversed) and opus-a opens the
  socket first with a five-second fall back to polling (also
  reversed: five blank seconds on the device the page is for).
- The facts stage split five and five: lean-a, lean-b, sonnet-b,
  checked-a and checked-b settled the Safari question (WebKit 41419
  or Cockpit's note, two with the newer 158345); sonnet-a, opus-a,
  opus-b, fable-a and fable-b filed it unresolved with the same
  evidence attached. The plans do not show which: the "works either
  way" rule made the filing irrelevant, which is what it was for.
- The one reversal left is the certificate reissue over the same
  key, in nine of ten plans and named by the Fable grader on every
  one of them; Codex counts it in none (its overturn column is 0
  across the ten, as through the whole trial). fable-b alone keeps
  the identity as it is and lists the reissue as a follow-up; the
  Fable grader then reversed its frame-side static route and
  three-file page instead. The second reversal, in six plans, is a
  fixed cookie name without the server id. Neither is in the planner
  guidance and I am not adding either: the reissue is the owner's
  call and the cookie name is a detail the reference happens to fix.
- The fable reviewer is the first to argue with the reissue policy
  (fable-a round 1: "the dialog's trust signal is defeated by the
  plan's own reissue policy"), without reversing it. The sonnet
  reviewer approved lean-a, lean-b and sonnet-a with "No further
  feedback" after 37 to 78 tool calls each, and raised four
  objections on sonnet-b; the variance stands. Zero WebSearch calls
  in any of the ten review sessions, again.
- Scores: Fable 3 / 2-3 / 1-2 / 2 / note on all ten, Codex 3 / 2 / 0
  / 1 / no on eight and 2 and 1 on grounding for the lean pair (the
  lean-b 1 is a grader error, overridden: it marked the plan's three
  new files "absent from the tree"; lean-a's 2 is one section number
  wrong). Level with revision 4's isolated plans and with each other.
  The model choice shows in cost and time only: lean $2.07 and 18
  min, sonnet $3.69 and 19, checked $5.33 and 21, opus $9.22 and 27,
  fable $20.57 and 38. Plans run 2.4k to 4.7k words; fable-b's is the
  shortest of the set at 3.1k and the one with no reissue.
- The case for a model per stage, from everything so far: haiku
  investigate and sonnet everywhere else (lean) loses nothing a grader
  can see at a fifth of opus's cost; the opus reviewer (checked) buys
  nothing on this issue over sonnet's; opus and fable throughout
  produce the same plan at three and ten times the price. The next
  test that would move this is a fresh issue, not another #230 run.

## 2026-10-22, evening: a second $50; #187 through revision 5; the implementation test begins

- The owner granted a second $50 (`results/LEDGER.md`, "Second
  budget") for three things: #187 planned under revision 5 to compare
  with its twelve first-trial plans, grading those, and implementing
  #230 with more than one implementer. The owner also left the two
  open calls to me; both are recorded in `reference.md` under
  "Resolved": no certificate reissue, and the cookie named with the
  server id.
- The harness grew to run a whole ticket without the trial Mac:
  `review2` and `respond2` (the second round and the answer),
  `revise` (the owner's edit between rounds, the pipeline's own
  `respond_to_user` template) and `implement` (the implement stage,
  with the build, test and local git commands allowed by rule and
  the stage gate run afterwards), and `chain.sh` strings investigate
  to the second answer. One difference from the pipeline besides
  those in stage.py's doc comment: a chain runs the respond step
  even when round one said "No further feedback", where the pipeline
  would finalize; the planner then changes nothing ($0.10).
- #187 under revision 5, six chains in parallel: sonnet throughout
  (a, b, c) and lean (haiku investigate, sonnet after; a, b, c).
  $1.46 to $2.35 a plan, about twelve minutes each, no failures.
  sonnet-b converged in one round; the other five used both. The
  facts stage searched nothing on the three sonnet investigations
  and three or four times on two lean ones, for source questions the
  haiku investigator had raised about session resume behaviour; the
  haiku notes are longer (1849 to 2409 words, 10 to 15 open
  questions, against 1197 to 1539 and 8 or 9).
- Graded in place (both graders, $0.91 a Fable grade). Fable:
  sonnet 3/3/2/2, 3/3/3/2, 3/3/1/2; lean 3/3/1/3, 3/3/2/2, 3/3/2/3,
  all hand-off note. Codex: sonnet 3/3/1/1 note, 3/3/0/2 no, 2/3/0/2
  note; lean 3/2/0/2 note, 3/2/0/1 no, 3/2/0/2 no. The first trial's
  twelve #187 plans were Fable 3/3/2/2/note almost throughout and
  Codex 3/2-3/0-1/1. So: level on decisions, a point better on
  over-specification in three of six, shorter (695 to 1621 words
  against 1319 to 2055), at the first trial's sonnet-pipeline cost
  and under its opus cost. The reversals are the same ones as in
  October's first batch: `show` left alone, provenance not recorded,
  one plan gating the copy on a response file, one adding a new
  rerun answer word. None is a fact the facts stage could have found;
  #187 has no outside fact, which is why the stage ran dry on it.
- Answer to "is it better now": not on this issue, and it was not
  expected to be. The pipeline's gain is information; #187 needs
  none. Its cost is the same as the first trial's sonnet pipeline
  for seven sessions instead of three, and its plans are tighter.
- The implementation test: fable-b (the only #230 plan that keeps
  today's certificate) and lean-b (the cheapest arm) revised with the
  `revise` stage on sonnet ($0.24 and $0.38): fable-b's cookie is now
  named by server id; lean-b lost the reissue, the export and their
  tests and gained the name. Both revised plans then went to a sonnet
  implementer, in parallel, on worktrees at 7fe8949.

## 2026-10-22, late: four implementations of #230 ($32.82 of the second $50)

- The two revised plans (fable-b, lean-b) each built by a sonnet and
  a haiku implementer, reviewed by the pipeline's correctness and
  style reviewers at sonnet, and fixed by the arm's own model; every
  branch green on the implement gate after its fix round. The table
  and the reading are in `results/sb-230/implementations.md`.
- Implementing cost $2.04 to $2.36 and 18 to 25 minutes a branch,
  about the price of planning it; reviews $0.24 to $0.42; a fix round
  $0.10 (haiku) to $0.34 (sonnet).
- The Mac's disk filled during the first haiku runs and the lean-b
  sonnet fix: each implement worktree builds its own target directory
  (5 to 7 GB) and the shared one under the main checkout had grown to
  53 GB. With the owner's leave the shared cache and the failed runs'
  caches were removed; the failed runs are kept as `-attempt1`
  directories with their partial diffs, and nothing else was lost.
  This is what the trial Mac was for.
- The lean-b sonnet fix's gate failed once on a dispatch unit test
  (`a_second_kill_of_a_check_sends_nothing_and_only_escalate_does`)
  with four cargo builds running; the rerun passed. The branch does
  not touch dispatch.
- Harness additions this session: `review-code` and `fix` stages
  (the workflow templates the app holds are stood in for by one
  paragraph), `TREE` to run a stage on an existing worktree, and a
  stage run's directory accepted as `<from>`.
- Not done: no page was opened in a browser, and no second review
  round. The owner's phone against one of the four worktrees is the
  next test, and the one the whole #230 trial has been pointing at.

## 2026-10-23: the phone test

- The owner's checkout took the sonnet fable-b branch as `web-control`
  (one commit, the fixup squashed), the gate passed cold, the bundle
  was built and the live app relaunched on it (listener pid, TERM,
  open, two seconds each way). The page served with its headers on
  the first request.
- The owner paired an iPad, an iPhone and an Android tablet. On every
  one the click-through covered the page, the cookie was stored and
  sent back, and the page paired from the URL; a message sent from the
  iPad's page reached this session through the control socket. The
  Android camera offered no QR scan; the copied invite URL worked. So
  the one unresolved fact every plan built a fallback for (the cookie
  on a clicked-through origin) came out favourable on all three.
- First bug from use: the reply box on a session card loses focus on
  every poll's redraw, which makes typing nearly impossible. The
  lean-b correctness reviewer raised exactly this point on both lean-b
  branches and their fixers fixed it; the fable-b reviewers did not
  see it. Also seen in the host's log: a cookie-authenticated request
  does not update the device's `last_seen`, which the headset's bearer
  requests do.
- The owner wants to try all four. The other three are being built as
  separate bundles under `~/Applications/trial-web/<arm>/` from their
  worktrees, so a switch is a stop and an open. Within a family the
  pairing carries over (same server identity, same cookie name);
  across families (`switchboard_token_<id>` for fable-b,
  `__Host-sb-<id>` for lean-b) the phone has to pair again.

## 2026-10-23 and 24: the six builds on the phone; opus and fable; a second round ($98.22 of the second $50)

- The owner tried the four builds on an iPad, an iPhone and an
  Android tablet, swapped by stopping the listener's pid and opening
  the next bundle. Every build paired and worked on every device; the
  one bug seen was the sonnet fable-b page losing focus on each poll,
  which haiku fable-b did not share and the lean-b reviewers had
  caught. The owner liked the lean-b pair, settled on that plan, and
  asked for opus and fable implementations of it, then a second round
  of sonnet, opus and fable to test consistency. Nine implementations
  of #230 in all; the table and the reading are in
  `results/sb-230/implementations.md`.
- Each model is consistent with itself: sizes within ten percent of
  its own first run, the same tests by name, the same architecture;
  fable builds the spike as a program both times. The reviewers find
  the same family of page-script points every round. On the device
  the owner found sonnet, opus and fable indistinguishable.
- The second budget closes at $98.22: the four opus and fable
  implementations and their fix rounds ($59 of it) were the owner's
  explicit asks after the $50 was spent, and the sonnet and haiku work
  came to under $20.
- One Claude Code API outage during the opus and fable bundle builds
  was recovered without a rerun. Disk was watched: finished worktrees'
  caches removed before each new pair.
- The trial's answer on implementation: the plan decides what the
  feature does, the implementer's model decides how much is built
  around it and what it costs, and the browser is still the only test
  of the page. Next: pick sonnet or opus lean-b to finish (rebase,
  second review round, PR), lift fable's spike onto it, and run the
  spike against Chrome and Safari.
