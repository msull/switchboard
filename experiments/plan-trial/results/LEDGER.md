# Iteration budget

Budget: $50, granted 2026-10-22 after revision 2's runs, for refining the
stage guidance and testing it in isolation. Counted: every stage run
from the first label counted: `v3` onward, from each run's own
total_cost_usd, and every grader run written after the grant (grades
after epoch ms 1792707427227); a Codex grader run reports no cost and is counted
at a flat $0.30. Not counted: anything before the grant, and runs on
the trial Mac, which are the pipeline's cost rather than the
iteration's. `spend.py` prints the current total; the figure in each
journal entry is from it.

Closed 2026-10-22 at $53.15: the revision 5 arms' twenty grades took
the total past $50, by the four opus and fable arms the owner added
with that cost in view. No further stage runs or grades are charged
to this budget.

# Second budget

Budget 2: $50, granted 2026-10-22 evening for running #187 through the
revision 5 pipeline (chain.sh: labels `v5` and `v5lean`), grading those
plans, and implementation runs on #230 (the `revise` and `implement`
stages). Counted: every stage run and grader run written after epoch ms
1792724823766, by file time; the same flat $0.30 for a Codex run.
`spend.py` prints both budgets.

Closed 2026-10-24 at $98.22: the opus and fable implementations and
their fix rounds, asked for after the $50 was spent, are the overrun.
