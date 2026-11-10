#!/bin/zsh
# Does mDNSResponder drop a dns-sd -R registration when its client dies by SIGTERM or SIGKILL?
# dns-sd is a DNSServiceRegister client, so its exit is the same event as the host's.
# Browse is local (same mDNSResponder), so this shows deregistration, not wire packets.
out=${1:-/tmp}
for sig in TERM KILL; do
  name="sbspike-$sig"
  dns-sd -R "$name" _sbspike._tcp . 59999 v=1 >$out/reg-$sig.log 2>&1 &
  reg=$!
  dns-sd -B _sbspike._tcp . >$out/br-$sig.log 2>&1 &
  br=$!
  sleep 4
  kill -$sig $reg; t0=$(date +%s.%N)
  sleep 4
  kill $br
  echo "== SIG$sig: killed at $t0"; grep -E "Add|Rmv" $out/br-$sig.log
done
