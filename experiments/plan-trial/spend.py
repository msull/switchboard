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
# A second grant: everything written after its epoch is its, not the first's.
m2 = re.search(r"Budget 2: \$(\d+).*?after epoch ms\s+(\d+)", text, re.S)
budget2, since2 = (float(m2.group(1)), float(m2.group(2))) if m2 else (0.0, float("inf"))

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
rows2 = {}
def which(f):
    return rows2 if f.stat().st_mtime * 1000 >= since2 else rows
for f in sorted(HERE.glob("results/*/stages/*/*/run.json")):
    label = f.parent.name.rsplit("-", 2)[0]
    n = revision(label)
    if n is None or n < revision(first):
        continue   # before the grant, or not a revision label
    key = f"{f.parents[3].name} {f.parents[1].name} {label}"
    r = which(f)
    r.setdefault(key, [0, 0.0])
    r[key][0] += 1
    r[key][1] += cost_of(f)
# Grader runs: on blind letters under grades/, and on stage runs in place.
for f in sorted(HERE.glob("results/*/grades/*.fable.json")) + sorted(HERE.glob("results/*/stages/*/*/grade.fable.json")):
    if f.stat().st_mtime * 1000 >= since:
        issue = f.parents[1].name if f.parent.name == "grades" else f.parents[3].name
        key = f"{issue} grading fable"
        r = which(f); r.setdefault(key, [0, 0.0]); r[key][0] += 1; r[key][1] += cost_of(f)
for f in sorted(HERE.glob("results/*/grades/*.codex.md")) + sorted(HERE.glob("results/*/stages/*/*/grade.codex.md")):
    if f.stat().st_mtime * 1000 >= since:
        issue = f.parents[1].name if f.parent.name == "grades" else f.parents[3].name
        key = f"{issue} grading codex (est.)"
        r = which(f); r.setdefault(key, [0, 0.0]); r[key][0] += 1; r[key][1] += CODEX_ESTIMATE
def table(rows, budget, title):
    total = 0.0
    print(f"{title:40} {'runs':>4} {'$':>7}")
    for k, (n, c) in rows.items():
        print(f"{k:40} {n:4} {c:7.2f}"); total += c
    print(f"{'total':40} {'':4} {total:7.2f}  of ${budget:.0f} ({100*total/budget:.0f}%)")
table(rows, budget, "budget 1")
if m2:
    print()
    table(rows2, budget2, "budget 2")
