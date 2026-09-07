#!/bin/sh
# Runs the keychain experiments. `probe BIN NAME` reports whether a get
# returned within 8 s (no prompt) or blocked on a Keychain prompt.
set -u
probe() {
  "$1" get "$2" > /tmp/spike-keychain.out 2>&1 &
  pid=$!
  for i in 1 2 3 4 5 6 7 8; do
    sleep 1
    kill -0 $pid 2>/dev/null || break
  done
  if kill -0 $pid 2>/dev/null; then kill $pid; echo "  BLOCKED on a prompt: $(tail -1 /tmp/spike-keychain.out)"; else echo "  $(tail -1 /tmp/spike-keychain.out)"; fi
}
B=target/release/spike-keychain
cp $B /tmp/spike-a; cp $B /tmp/spike-b
echo "A. ad-hoc binary creates and reads its own item"
codesign -s - -f /tmp/spike-a >/dev/null 2>&1
/tmp/spike-a set ALPHA one; probe /tmp/spike-a ALPHA
echo "B. a different ad-hoc signed copy reads the item"
codesign -s - -f -i spike.other /tmp/spike-b >/dev/null 2>&1
probe /tmp/spike-b ALPHA
/tmp/spike-a delete ALPHA
echo "C. signed with the Prompt Box Dev identity: first build stores, a rebuilt copy reads"
cp $B /tmp/spike-c1; cp $B /tmp/spike-c2
codesign -s "Prompt Box Dev" -f -i com.sadburger.switchboard.spike /tmp/spike-c1 >/dev/null 2>&1
/tmp/spike-c1 set BETA two; probe /tmp/spike-c1 BETA
# c2 is byte-identical here; make it differ so its cdhash differs.
printf '\0' >> /tmp/spike-c2 2>/dev/null; codesign -s "Prompt Box Dev" -f -i com.sadburger.switchboard.spike /tmp/spike-c2 >/dev/null 2>&1
codesign -dvv /tmp/spike-c1 2>&1 | grep -E 'CDHash' ; codesign -dvv /tmp/spike-c2 2>&1 | grep -E 'CDHash'
probe /tmp/spike-c2 BETA
echo "D. same identity, different bundle identifier"
cp $B /tmp/spike-d; codesign -s "Prompt Box Dev" -f -i com.sadburger.other /tmp/spike-d >/dev/null 2>&1
probe /tmp/spike-d BETA
/tmp/spike-c1 delete BETA
rm -f /tmp/spike-a /tmp/spike-b /tmp/spike-c1 /tmp/spike-c2 /tmp/spike-d
