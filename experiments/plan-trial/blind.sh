#!/bin/sh
# Shuffles an issue's plans into results/<short>-<issue>/blind/ with the
# key in key.txt and a scores.md to fill in. The first call takes the
# twelve first-trial plans as A to L. A later call with `facts` adds the
# facts-arm plans not yet in the key as the next free letters, shuffled
# among themselves, appending to the key and the score sheet; the
# earlier letters keep their grades. So each batch of facts arms is
# blinded once, when its plans are collected. Score before opening the
# key. A pipeline arm's plan is the reviewed copy.
# usage: blind.sh <switchboard|simplesingletable> <issue> [facts]
set -eu
here=$(cd "$(dirname "$0")" && pwd)
. "$here/lib.sh"
repo=$1; issue=$2; set_=${3:-}
repo_vars "$repo"
python3 -I - "$here/results/$short-$issue" "$set_" <<'PY'
import pathlib, random, string, sys
res = pathlib.Path(sys.argv[1])
facts = sys.argv[2] == "facts"
blind = res / "blind"
keyfile = blind / "key.txt"
if facts:
    if not keyfile.exists():
        sys.exit("blind/key.txt missing: run blind.sh for the first-trial plans first")
    entries = [line.split() for line in keyfile.read_text().splitlines() if line]
    taken = [e[0] for e in entries]
    done = {e[1] for e in entries}
    plans = [p for p in sorted(res.glob("facts*/*/plan.md"))
             if f"{p.parent.parent.name}/{p.parent.name}" not in done]
    if not plans:
        sys.exit("every collected facts plan is already in blind/key.txt")
    if len(plans) % 2:
        sys.exit(f"{len(plans)} new facts plans: a scenario is missing its a or b")
    # A to Z, then AA, AB, ...: the trial outgrew the alphabet.
    pool = list(string.ascii_uppercase) + [a + b for a in string.ascii_uppercase for b in string.ascii_uppercase]
    letters = [c for c in pool if c not in taken][:len(plans)]
else:
    plans = sorted(res.glob("pipeline/*/plan.md")) + sorted(res.glob("single/*/plan.md"))
    if len(plans) != 12:
        sys.exit(f"{len(plans)} plans found, expected 12 (6 pipeline + 6 single)")
    blind.mkdir(exist_ok=True)
    if keyfile.exists():
        sys.exit("blind/key.txt exists; remove the blind directory to reshuffle")
    letters = list("ABCDEFGHIJKL")
random.shuffle(plans)
key, rows = [], []
for letter, p in zip(letters, plans):
    (blind / f"{letter}.md").write_text(p.read_text())
    key.append(f"{letter} {p.parent.parent.name}/{p.parent.name}")
    rows.append(f"| {letter} | | | | | | |")
with keyfile.open("a") as f:
    f.write("\n".join(key) + "\n")
scores = blind / "scores.md"
if scores.exists() and facts:
    with scores.open("a") as f:
        f.write("\n".join(rows) + "\n")
else:
    scores.write_text(
        "# Scores\n\nSee rubric.md. 0 to 3 per column; hand-off is yes, note or no.\n\n"
        "| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off | notes |\n"
        "|---|---|---|---|---|---|---|\n" + "\n".join(rows) + "\n")
print(f"{blind}: {letters[0]}..{letters[-1]} written, key in key.txt, scores in scores.md")
PY
