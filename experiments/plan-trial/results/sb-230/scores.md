# Scores: sb-230

Per grader: grounding / departures caught / decisions to overturn / over-specification / hand-off.
Rounds and cap are the plan review's; minutes are investigate+plan+review for a pipeline arm and
the session's duration for a single one; cost is tokens at list price for a pipeline arm and
Claude Code's own figure for a single session. A starred value is an adjudication from
grades/overrides.txt.

| arm | plan | fable | codex | rounds | min | $ | words |
|---|---|---|---|---|---|---|---|
| pipeline/haiku-a | H | 3 / 3 / 1 / 2 / note | 3 / 1 / 0 / 1 / no | 6 cap | 20 | 0.40 | 4713 |
| pipeline/haiku-b | A | 3 / 2 / 1 / 2 / no | 3 / 1 / 0 / 1 / no | 5 | 19 | 0.33 | 3675 |
| pipeline/opus-a | K | 3 / 2 / 1 / 2 / no | 2 / 1 / 0 / 0 / no | 3 | 20 | 7.61 | 5586 |
| pipeline/opus-b | G | 3 / 3 / 1 / 2 / note | 3 / 2 / 0 / 1 / no | 2 | 13 | 5.65 | 3635 |
| pipeline/sonnet-a | J | 3 / 2 / 2 / 2 / note | 3 / 2 / 0 / 1 / no | 2 | 10 | 3.13 | 3848 |
| pipeline/sonnet-b | I | 3 / 2 / 1 / 1 / note | 3 / 1 / 0 / 0 / no | 4 | 12 | 3.79 | 5558 |
| single/fable-a | C | 3 / 2 / 1 / 2 / note | 3 / 1 / 0 / 2 / no |  | 6 | 4.68 | 2618 |
| single/fable-b | B | 3 / 1 / 1 / 2 / note | 3* / 1 / 0 / 2 / no |  | 7 | 4.73 | 2401 |
| single/opus-a | E | 3 / 2 / 1 / 2 / note | 3 / 1 / 0 / 1 / no |  | 7 | 2.86 | 3585 |
| single/opus-b | L | 3 / 2 / 1 / 2 / note | 3 / 1 / 0 / 1 / no |  | 10 | 3.51 | 4086 |
| single/sonnet-a | D | 3 / 3 / 2 / 2 / note | 3 / 2 / 0 / 1 / no |  | 7 | 1.22 | 3402 |
| single/sonnet-b | F | 3 / 2 / 1 / 2 / note | 3 / 1 / 0 / 1 / no |  | 5 | 1.09 | 3484 |

## Means over a and b, both graders

| arm | grounding | departures | overturn | overspec | hand-off yes/note/no | min | $ |
|---|---|---|---|---|---|---|---|
| pipeline/haiku | 3.0 | 1.8 | 0.5 | 1.5 | 0/1/3 | 19 | 0.37 |
| pipeline/opus | 2.8 | 2.0 | 0.5 | 1.2 | 0/1/3 | 16 | 6.63 |
| pipeline/sonnet | 3.0 | 1.8 | 0.8 | 1.0 | 0/2/2 | 11 | 3.46 |
| single/fable | 3.0 | 1.2 | 0.5 | 2.0 | 0/2/2 | 6 | 4.70 |
| single/opus | 3.0 | 1.5 | 0.5 | 1.5 | 0/2/2 | 9 | 3.18 |
| single/sonnet | 3.0 | 2.0 | 0.8 | 1.5 | 0/2/2 | 6 | 1.15 |

## Where the graders disagree (two or more points, or yes against no)

- pipeline/haiku-a (H): departures 3 vs 1
- pipeline/opus-a (K): overspec 2 vs 0
- pipeline/sonnet-a (J): overturn 2 vs 0
- single/sonnet-a (D): overturn 2 vs 0
