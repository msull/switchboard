#!/usr/bin/env python3 -I
"""The owner's iteration budget, computed from the runs' own cost reports.

usage: spend.py [budget_usd]

Sums total_cost_usd over every stage run (results/*/stages/*/*/run.json)
whose label is in the budget (results/LEDGER.md names the first label
counted), and every grader run (results/*/grades/*.fable.json) written
after the budget began. Codex grader runs report no cost and are
counted at a flat estimate. Prints a table by label and the total
against the budget.
"""
import json, pathlib, re, sys

HERE = pathlib.Path(__file__).resolve().parent
LEDGER = HERE / "results/LEDGER.md"
CODEX_ESTIMATE = 0.30   # a grading run of ~40 s on a 20k-token packet, not reported by codex exec

text = LEDGER.read_text()
budget = float(sys.argv[1]) if len(sys.argv) > 1 else float(re.search(r"Budget: \$(\d+)", text).group(1))
first = re.search(r"first label counted: `(\S+)`", text).group(1)
since = float(re.search(r"grades\s+after epoch ms (\d+)", text).group(1))

def cost_of(run_json):
    try:
        r = json.loads(run_json.read_text())
        r = r[-1] if isinstance(r, list) else r
        return float(r.get("total_cost_usd") or 0)
    except (ValueError, OSError):
        return 0.0

def revision(label):
    m = re.match(r"v(\d+)", label)
    return int(m.group(1)) if m else None

rows = {}
for f in sorted(HERE.glob("results/*/stages/*/*/run.json")):
    label = f.parent.name.rsplit("-", 2)[0]
    n = revision(label)
    if n is None or n < revision(first):
        continue   # before the grant, or not a revision label
    key = f"{f.parents[3].name} {f.parents[1].name} {label}"
    rows.setdefault(key, [0, 0.0])
    rows[key][0] += 1
    rows[key][1] += cost_of(f)
for f in sorted(HERE.glob("results/*/grades/*.fable.json")):
    if f.stat().st_mtime * 1000 >= since:
        key = f"{f.parents[1].name} grading fable"
        rows.setdefault(key, [0, 0.0]); rows[key][0] += 1; rows[key][1] += cost_of(f)
for f in sorted(HERE.glob("results/*/grades/*.codex.md")):
    if f.stat().st_mtime * 1000 >= since:
        key = f"{f.parents[1].name} grading codex (est.)"
        rows.setdefault(key, [0, 0.0]); rows[key][0] += 1; rows[key][1] += CODEX_ESTIMATE
total = 0.0
print(f"{'what':40} {'runs':>4} {'$':>7}")
for k, (n, c) in rows.items():
    print(f"{k:40} {n:4} {c:7.2f}"); total += c
print(f"{'total':40} {'':4} {total:7.2f}  of ${budget:.0f} ({100*total/budget:.0f}%)")
