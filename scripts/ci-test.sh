#!/bin/bash
# CI's test step: the whole workspace, and once more for just the tests
# that failed. A test that passes on the retry is reported as a warning
# on the run, so flakiness stays visible; one that fails twice fails CI.
# The floor under the tmux defect on the Linux runner, not a fix for it.
set -euo pipefail

log="$(mktemp)"
trap 'rm -f "$log"' EXIT

# With `pipefail`, the `if` sees cargo's status rather than `tee`'s.
if cargo test --locked --workspace --no-fail-fast 2>&1 | tee "$log"; then
    exit 0
fi

# libtest lists each binary's failures as indented names under a
# `failures:` line, ending at a blank line. The same heading also opens
# the block of captured output, whose lines start with `----`.
failed=$(awk '
    /^failures:$/ { listing = 1; next }
    listing && /^    [^ ]/ { print $1; next }
    listing { listing = 0 }
' "$log" | sort -u)

if [ -z "$failed" ]; then
    echo "the test run failed without naming a failed test" >&2
    exit 1
fi

# Cargo reports each failed binary once. One that crashed (an abort, a
# double panic) lists no names, and a retry by name would skip it.
listings=$(awk '
    /^failures:$/ { listing = 1; next }
    listing && /^    [^ ]/ { count++; listing = 0; next }
    listing { listing = 0 }
    END { print count + 0 }
' "$log")
targets=$(grep -cE '^error: (test|doctest) failed, to rerun pass' "$log" || true)
if grep -q '^error: could not compile' "$log" || [ "$targets" -gt "$listings" ]; then
    echo "a test binary failed without naming its failed tests" >&2
    exit 1
fi

echo "retrying once:"
echo "$failed" | sed 's/^/  /'
# A name that matches nothing in another binary runs nothing there.
# shellcheck disable=SC2086
cargo test --locked --workspace -- --exact $failed

for name in $failed; do
    echo "::warning::flaky: $name"
done
