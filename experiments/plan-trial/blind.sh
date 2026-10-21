#!/bin/sh
# Shuffles an issue's twelve plans into results/<short>-<issue>/blind/ as
# A.md .. L.md with the key in key.txt, and writes a scores.md to fill in.
# Score before opening the key. The pipeline arm's plan is the reviewed copy.
# usage: blind.sh <switchboard|simplesingletable> <issue>
set -eu
here=$(cd "$(dirname "$0")" && pwd)
. "$here/lib.sh"
repo=$1; issue=$2
repo_vars "$repo"
python3 - "$here/results/$short-$issue" <<'PY'
import pathlib, random, sys
res = pathlib.Path(sys.argv[1])
plans = sorted(res.glob("pipeline/*/plan.md")) + sorted(res.glob("single/*/plan.md"))
if len(plans) != 12:
    sys.exit(f"{len(plans)} plans found, expected 12 (6 pipeline + 6 single)")
blind = res / "blind"
blind.mkdir(exist_ok=True)
if (blind / "key.txt").exists():
    sys.exit("blind/key.txt exists; remove the blind directory to reshuffle")
random.shuffle(plans)
key, rows = [], []
for letter, p in zip("ABCDEFGHIJKL", plans):
    (blind / f"{letter}.md").write_text(p.read_text())
    key.append(f"{letter} {p.parent.parent.name}/{p.parent.name}")
    rows.append(f"| {letter} | | | | | | |")
(blind / "key.txt").write_text("\n".join(key) + "\n")
(blind / "scores.md").write_text(
    "# Scores\n\nSee rubric.md. 0 to 3 per column; hand-off is yes, note or no.\n\n"
    "| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off | notes |\n"
    "|---|---|---|---|---|---|---|\n" + "\n".join(rows) + "\n")
print(f"{blind}: A..L written, key in key.txt, scores in scores.md")
PY
