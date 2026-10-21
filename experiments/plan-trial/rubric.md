# Scoring a plan

Score blind: the twelve plans of an issue as A to L, with the key closed
until every row is filled. Length reads as thoroughness, so score each
column on its own and never the whole impression.

Each column is 0 to 3 except the last.

- **Grounding.** Open five file or line references the plan makes, in a
  worktree at the issue's base commit (`results/<short>-<issue>/base.txt`).
  3: all real and relevant. 2: one wrong. 1: several wrong. 0: the plan
  cites nothing checkable, or mostly misses.
- **Departures caught.** Where the issue's claims do not match the code,
  did the plan say so and say what it does instead? 3: every departure,
  including ones you had not noticed. 0: it planned the issue as written
  where the code says otherwise.
- **Decisions to overturn.** Count the calls you would reverse before
  implementing. 3: none. 2: one. 1: two. 0: three or more.
- **Over-specification.** How much of the plan an implementer would not
  need: restated code, sections with nothing to say, steps the test
  suite would force anyway. 3: nothing to cut. 0: half or more.
- **Hand-off.** Would you give this plan to an implementer as it is?
  `yes`, `note` (with one line of guidance), `no`.
- **Notes.** One line: the plan's one best and one worst thing.

After the key is opened, put beside each arm's scores what the run
measured: plan review rounds and whether the cap was hit (from
`report.json`), wall time, tokens by model, and for a single session the
`total_cost_usd` and `duration_ms` in `run.json`. Two repetitions per
arm: note where they disagree, since that is the consistency result.

Write the result per issue as the table and a paragraph of judgment, not
an average over two issues.
