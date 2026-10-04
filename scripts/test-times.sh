#!/bin/bash
# Per-test times for the whole workspace, run serially (parallel times
# on macOS are mostly contention), with the slowest ten, each binary's
# total, and any test over the budget. Exits non-zero if a test failed
# or went over the budget, so it can serve as a check.
#
# libtest's `--report-time` is unstable; `RUSTC_BOOTSTRAP=1` on each
# test binary, never on cargo, allows it on the stable toolchain without
# changing cargo's fingerprints and forcing a rebuild.
set -euo pipefail

# A test that runs git in a fixture must not find the repository a hook
# is committing to.
for var in $(env | sed -n 's/^\(GIT_[A-Za-z0-9_]*\)=.*/\1/p'); do
    unset "$var"
done

cd "$(dirname "$0")/.."

cargo test --locked --workspace --no-run --message-format=json \
    | python3 -c '
import json, os, sys
root = os.getcwd()
for line in sys.stdin:
    try:
        msg = json.loads(line)
    except ValueError:
        continue
    if msg.get("reason") == "compiler-artifact" and msg.get("profile", {}).get("test") and msg.get("executable"):
        dir = os.path.dirname(msg["manifest_path"])
        # Package and target, so the two `live` binaries read apart.
        package = os.path.relpath(dir, root)
        target = msg["target"]["name"] + ("" if "test" in msg["target"]["kind"] else " (" + msg["target"]["kind"][0] + ")")
        label = target if package == "." else package + "/" + target
        print(msg["executable"] + "\t" + dir + "\t" + label)
' \
    | while IFS=$'\t' read -r bin dir label; do
        echo "#binary $label"
        # As under `cargo test`, relative paths start at the package.
        status=0
        (cd "$dir" && RUSTC_BOOTSTRAP=1 "$bin" --test-threads=1 \
            -Z unstable-options --report-time --format json) || status=$?
        echo "#exit $status"
    done \
    | python3 -c '
import json, sys
budget = 5.0  # seconds a test; the README states it
binary, times, totals, failed = "?", [], {}, []
binary_failed = False
for line in sys.stdin:
    if line.startswith("#binary "):
        binary = line.split(" ", 1)[1].strip()
        totals.setdefault(binary, 0.0)
        binary_failed = False
        continue
    if line.startswith("#exit "):
        # A binary that crashed or never started names no failed test.
        status = line.split(" ", 1)[1].strip()
        if status != "0" and not binary_failed:
            failed.append(binary + " exited with status " + status)
        continue
    try:
        ev = json.loads(line)
    except ValueError:
        continue
    if ev.get("type") != "test" or ev.get("event") not in ("ok", "failed"):
        continue
    secs = float(ev.get("exec_time", 0.0))
    times.append((secs, binary, ev["name"]))
    totals[binary] += secs
    if ev["event"] == "failed":
        failed.append(binary + " " + ev["name"])
        binary_failed = True
times.sort(reverse=True)
print("slowest ten:")
for secs, b, name in times[:10]:
    print(f"  {secs:8.2f} s  {b}  {name}")
print("each binary, serially:")
for b, secs in sorted(totals.items(), key=lambda kv: -kv[1]):
    print(f"  {secs:8.2f} s  {b}")
print(f"  {sum(totals.values()):8.2f} s  in all")
over = [t for t in times if t[0] > budget]
for secs, b, name in over:
    print(f"over the {budget:g} s budget: {b} {name} ({secs:.2f} s)")
for name in failed:
    print(f"failed: {name}")
sys.exit(1 if over or failed else 0)
'
