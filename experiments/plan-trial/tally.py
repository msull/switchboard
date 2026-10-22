#!/usr/bin/env python3 -I
"""Joins an issue's grades to its key and measurements.

usage: tally.py results/<short>-<issue>

Reads blind/key.txt, grades/<letter>.<grader>.md (the score table each
grader wrote), the pipeline and facts arms' report.json and the single
arms' run.json, and writes scores.md: one row per arm with both graders'
columns and the run's measurements, the per-arm means over a and b, and
where the graders disagree. Pipeline cost is tokens at list price; a
single session's cost is what Claude Code reported.
"""
import json, pathlib, re, statistics, sys

# $ per million tokens: input, output, cache write (1h), cache read.
PRICE = {
    "claude-opus-5-5": (4, 20, 8, 0.20),
    "claude-sonnet-5-5": (2, 10, 4, 0.20),
    "claude-haiku-5-5": (0.10, 0.50, 0.20, 0.01),
    "claude-fable-5-1": (10, 50, 20, 0.25),
}
COLS = ["grounding", "departures", "overturn", "overspec", "handoff"]
GRADERS = ["fable", "codex"]

res = pathlib.Path(sys.argv[1])
key = dict(line.split() for line in (res / "blind/key.txt").read_text().split("\n") if line)

def grade(letter, grader):
    f = res / f"grades/{letter}.{grader}.md"
    if not f.exists():
        return None
    for line in f.read_text().splitlines():
        m = re.match(r"\|\s*([A-Z]+)\s*\|(.*)\|\s*$", line)
        if m and m.group(1) == letter:
            cells = [c.strip() for c in m.group(2).split("|")]
            if len(cells) >= 5:
                return dict(zip(COLS, cells[:5]))
    return None

def cost(tokens):
    usd = 0.0
    for model, t in tokens.items():
        i, o, w, r = PRICE.get(model, (0, 0, 0, 0))
        usd += (t.get("input", 0) * i + t.get("output", 0) * o
                + t.get("cache_write", 0) * w + t.get("cache_read", 0) * r) / 1e6
    return usd

def measure(arm):
    d = res / arm
    if arm.startswith(("pipeline/", "facts")):
        r = json.loads((d / "report.json").read_text())
        show = json.loads((d / "show.json").read_text())
        # The review stage is still open (finalize is never answered in
        # the trial), so the report clocks it to collection time; its
        # real end is when finalize was asked.
        ms = sum(s["ms"] for s in r["stages"] if s["stage"] in ("investigate", "facts", "plan"))
        reviews = [a for a in show["attempts"] if a["stage"] == "review"]
        asked = [dc["made_ms"] for dc in show["decisions"] if dc["name"] == "finalize"]
        if reviews and asked:
            ms += max(asked) - reviews[-1]["started_ms"]
        failed = sum(a["state"] == "failed" for a in reviews)
        rounds = r["plan_rounds"]
        # The first trial's review cap is six rounds; the facts pipeline's is two.
        cap = "cap" if rounds >= (2 if arm.startswith("facts") else 6) else ""
        if failed:
            cap = (cap + f" {failed} rerun").strip()
        spike = (d / "spikes.txt").exists() and (d / "spikes.txt").read_text().strip() != ""
        if spike:
            cap = (cap + " spike").strip()
        return {"rounds": rounds, "cap": cap, "min": ms / 60000,
                "usd": cost(r["tokens"]), "words": len((d / "plan.md").read_text().split())}
    r = json.loads((d / "run.json").read_text())
    r = r[-1] if isinstance(r, list) else r
    return {"rounds": "", "cap": "", "min": r["duration_ms"] / 60000,
            "usd": r["total_cost_usd"], "words": len((d / "plan.md").read_text().split())}

# grades/overrides.txt: "<letter> <grader> <column> <value> — <reason>",
# an adjudication of a grader's error, kept beside the grades it corrects.
overrides = []
ov = res / "grades/overrides.txt"
if ov.exists():
    for line in ov.read_text().splitlines():
        if line and not line.startswith("#"):
            letter, grader, col, value = line.split()[:4]
            overrides.append((letter, grader, col, value))

rows = []
for letter, arm in sorted(key.items(), key=lambda kv: kv[1]):
    g = {gr: grade(letter, gr) for gr in GRADERS}
    for l, gr, col, value in overrides:
        if l == letter and g.get(gr):
            g[gr][col] = value + "*"
    rows.append((letter, arm, g, measure(arm)))

def num(x):
    try:
        return float(str(x).rstrip("*"))
    except (TypeError, ValueError):
        return None

out = ["# Scores: " + res.name, "",
       "Per grader: grounding / departures caught / decisions to overturn / over-specification / hand-off.",
       "Rounds and cap are the plan review's (cap six in the first trial, two in the facts pipeline; spike marks",
       "a facts arm that committed one); minutes are investigate+(facts)+plan+review for a pipeline arm and",
       "the session's duration for a single one; cost is tokens at list price for a pipeline arm and",
       "Claude Code's own figure for a single session. A starred value is an adjudication from",
       "grades/overrides.txt.", "",
       "| arm | plan | fable | codex | rounds | min | $ | words |", "|---|---|---|---|---|---|---|---|"]
for letter, arm, g, m in rows:
    cells = []
    for grader in GRADERS:
        s = g[grader]
        cells.append(" / ".join(s[c] for c in COLS) if s else "-")
    rounds = f"{m['rounds']} {m['cap']}".strip()
    out.append(f"| {arm} | {letter} | {cells[0]} | {cells[1]} | {rounds} | {m['min']:.0f} | {m['usd']:.2f} | {m['words']} |")

out += ["", "## Means over a and b, both graders", "",
        "| arm | grounding | departures | overturn | overspec | hand-off yes/note/no | min | $ |",
        "|---|---|---|---|---|---|---|---|"]
groups = {}
for letter, arm, g, m in rows:
    groups.setdefault(arm.rsplit("-", 1)[0], []).append((g, m))
for name, items in groups.items():
    means = []
    for c in COLS[:4]:
        vals = [num(g[gr][c]) for g, _ in items for gr in GRADERS if g[gr] and num(g[gr][c]) is not None]
        means.append(f"{statistics.mean(vals):.1f}" if vals else "-")
    hand = [g[gr]["handoff"].lower() for g, _ in items for gr in GRADERS if g[gr]]
    h = f"{hand.count('yes')}/{hand.count('note')}/{hand.count('no')}"
    mins = statistics.mean(m["min"] for _, m in items)
    usd = statistics.mean(m["usd"] for _, m in items)
    out.append(f"| {name} | {' | '.join(means)} | {h} | {mins:.0f} | {usd:.2f} |")

out += ["", "## Where the graders disagree (two or more points, or yes against no)", ""]
for letter, arm, g, m in rows:
    a, b = g["fable"], g["codex"]
    if not a or not b:
        continue
    diffs = []
    for c in COLS[:4]:
        x, y = num(a[c]), num(b[c])
        if x is not None and y is not None and abs(x - y) >= 2:
            diffs.append(f"{c} {a[c]} vs {b[c]}")
    if {a["handoff"].lower(), b["handoff"].lower()} == {"yes", "no"}:
        diffs.append("hand-off yes vs no")
    if diffs:
        out.append(f"- {arm} ({letter}): " + "; ".join(diffs))

(res / "scores.md").write_text("\n".join(out) + "\n")
print("\n".join(out))
