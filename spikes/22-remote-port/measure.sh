#!/bin/sh
# Spike 22 measurements. Starts the server on a test port, runs the
# bench, kills the server by its pid, and repeats for the hidden-window
# frame rate and for Bonjour.
set -u
BIN=../../target/release
PORT=43199
run_server() {
  $BIN/spike-remote-port --port $PORT --pin-file pin.txt "$@" > server.log 2>&1 &
  echo $! > server.pid
  sleep 1
}
stop_server() {
  kill "$(cat server.pid)" 2>/dev/null
  sleep 0.3
  echo "server frame stats: $(grep '^frame:' server.log | tail -1)"
  rm -f server.pid
}
echo "== frame 16 ms, all phases"
run_server --frame-ms 16
$BIN/bench --port $PORT --pin-file pin.txt --phase all
stop_server
echo
echo "== frame 100 ms (hidden window), latency"
run_server --frame-ms 100
$BIN/bench --port $PORT --pin-file pin.txt --phase latency
stop_server
echo
echo "== bonjour"
run_server --frame-ms 16 --bonjour
head -3 server.log
(dns-sd -B _switchboard._tcp local. > dnssd.log 2>&1 & echo $! > dnssd.pid); sleep 3; kill "$(cat dnssd.pid)" 2>/dev/null; rm -f dnssd.pid
cat dnssd.log
stop_server
