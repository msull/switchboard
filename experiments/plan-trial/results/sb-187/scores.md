# Scores: sb-187

Per grader: grounding / departures caught / decisions to overturn / over-specification / hand-off.
Rounds and cap are the plan review's; minutes are investigate+plan+review for a pipeline arm and
the session's duration for a single one; cost is tokens at list price for a pipeline arm and
Claude Code's own figure for a single session. A starred value is an adjudication from
grades/overrides.txt.

| arm | plan | fable | codex | rounds | min | $ | words |
|---|---|---|---|---|---|---|---|
| pipeline/haiku-a | J | 3 / 3 / 2 / 2 / note | 3 / 3 / 0 / 1 / note | 6 cap | 14 | 0.29 | 1554 |
| pipeline/haiku-b | K | 3 / 3 / 2 / 2 / note | 3* / 2 / 0 / 1 / no | 3 | 13 | 0.24 | 1766 |
| pipeline/opus-a | L | 3 / 3 / 2 / 2 / note | 3* / 2 / 1 / 1 / no | 3 | 10 | 4.05 | 1717 |
| pipeline/opus-b | C | 3 / 3 / 1 / 2 / note | 2 / 3 / 1 / 1 / note | 2 | 11 | 4.53 | 2034 |
| pipeline/sonnet-a | G | 3 / 3 / 2 / 2 / note | 3* / 2 / 0 / 1 / no | 3 1 rerun | 5 | 2.42 | 1828 |
| pipeline/sonnet-b | H | 3 / 3 / 2 / 2 / note | 3 / 3 / 1 / 1 / note | 2 | 5 | 1.76 | 2055 |
| single/fable-a | A | 3 / 3 / 2 / 2 / note | 3 / 3 / 0 / 1 / no |  | 6 | 4.07 | 1319 |
| single/fable-b | B | 3 / 3 / 3 / 2 / note | 3 / 2 / 1 / 1 / note |  | 7 | 4.10 | 1721 |
| single/opus-a | I | 3 / 3 / 2 / 2 / note | 3 / 2 / 0 / 1 / note |  | 5 | 1.85 | 1614 |
| single/opus-b | D | 3 / 3 / 3 / 2 / yes | 3 / 3 / 1 / 1 / note |  | 5 | 1.97 | 1565 |
| single/sonnet-a | E | 3 / 3 / 2 / 2 / note | 3 / 2 / 1 / 1 / note |  | 3 | 0.76 | 1547 |
| single/sonnet-b | F | 3 / 3 / 2 / 2 / note | 3 / 3 / 2 / 1 / note |  | 2 | 0.52 | 1467 |

## Means over a and b, both graders

| arm | grounding | departures | overturn | overspec | hand-off yes/note/no | min | $ |
|---|---|---|---|---|---|---|---|
| pipeline/haiku | 3.0 | 2.8 | 1.0 | 1.5 | 0/3/1 | 14 | 0.26 |
| pipeline/opus | 2.8 | 2.8 | 1.2 | 1.5 | 0/3/1 | 10 | 4.29 |
| pipeline/sonnet | 3.0 | 2.8 | 1.2 | 1.5 | 0/3/1 | 5 | 2.09 |
| single/fable | 3.0 | 2.8 | 1.5 | 1.5 | 0/3/1 | 7 | 4.09 |
| single/opus | 3.0 | 2.8 | 1.5 | 1.5 | 1/3/0 | 5 | 1.91 |
| single/sonnet | 3.0 | 2.8 | 1.8 | 1.5 | 0/4/0 | 2 | 0.64 |

## Where the graders disagree (two or more points, or yes against no)

- pipeline/haiku-a (J): overturn 2 vs 0
- pipeline/haiku-b (K): overturn 2 vs 0
- pipeline/sonnet-a (G): overturn 2 vs 0
- single/fable-a (A): overturn 2 vs 0
- single/fable-b (B): overturn 3 vs 1
- single/opus-a (I): overturn 2 vs 0
- single/opus-b (D): overturn 3 vs 1
