# Spike 22: the remote port over std

The questions from `docs/remote-client.md` §10 that the host side can
answer without a client: can a pinned-TLS HTTP and WebSocket port run on
std threads with every request answered by the frame thread, what the
handoff costs, whether the bounds of §4.1 hold under abuse, whether
Bonjour works from Rust, what a transcript costs to parse, and whether
the content-version signals of §4.4 see the changes that defeat weaker
ones. The visionOS half (a `URLSession` pinning delegate accepting the
certificate and refusing a substitute) is not here: it needs an Xcode
project and the simulator.

Standalone crate, not built by the app. Builds into the repo's shared
`target/` like every crate under this tree.

## Crates

`rustls` 0.23 on `ring` (already in the app's tree), `rcgen` 0.14 for
the self-signed certificate, `tungstenite` 0.30 over the TLS stream
(`WebSocket::from_raw_socket` after a hand-written 101, since the
request was already read), `mdns-sd` 0.21 for Bonjour with no runtime,
`sha2` and `sha1` and `base64` for the pin and the accept key. No Tokio.
The whole thing compiled clean under the app's pedantic clippy on the
first try bar lints.

## What it does

`src/main.rs` is the server: one accept thread, one thread per
connection, TLS handshake and the first request line within 5 s, writes
within 10 s, a bounded channel of 64 to a stand-in frame thread that
sleeps `--frame-ms` and then drains at most 8 requests, bumps a
revision, and gives every WebSocket subscriber one event of
`--event-bytes` from a queue of 256; a full queue drops the subscriber
with its overflow flag set, and its writer sends `resync_required` and
closes. Two connection pools: 4 unauthenticated, 12 paired (a bearer
token moves a connection over); a further accept is dropped at once.
`src/bin/bench.rs` is a pinned client (a `ServerCertVerifier` that
checks one SHA-256 and nothing else) with four phases. `tests/versions.rs`
is the content-version experiment.

## Commands

```sh
cd spikes/22-remote-port
cargo test --release                 # the pin and request parsing; the version experiment
./measure.sh                         # the three server runs below; kills the server by pid
SWITCHBOARD_SPIKE_TRANSCRIPT=<path> cargo test --release --lib parse_timing -- --ignored --nocapture   # from the repo root
```

## Results (2026-10-12, M-series MacBook Pro, loopback)

**Handoff.** 300 `GET /api/v1/info` on one kept connection, then 50
each with a fresh TLS handshake, the client sending the next request as
soon as the reply lands:

| frame | kept connection p50 / p99 | fresh TLS p50 / p99 | frame's pass cost avg / max |
|---|---|---|---|
| 16 ms | 22.0 / 24.2 ms | 21.3 / 24.2 ms | 20 µs / 184 µs |
| 100 ms (hidden window) | 107.8 / 110.2 ms | 107.4 / 110.2 ms | 28 µs / 225 µs |

The latency is one pass interval, not half: a client that sends on the
reply synchronizes with the frame. The interval is longer than the
sleep (22 ms for a 16 ms sleep, 108 for 100) because `thread::sleep`
overshoots by 6 to 8 ms on this Mac; eframe paces its own loop, so the
app will not. `TCP_NODELAY` and a one-record reply changed nothing,
which is how the overshoot was found. **The frame's share is under
0.25 ms per pass at the worst, against the 1 ms target.** A fresh TLS
handshake costs nothing visible beside the frame interval.

**Overflow.** A subscriber that stops reading while 4 KiB events flow
at the frame rate: after 12 s unread, the drain gave 391 frames
(the kernel's buffers and the 256-deep queue), then `resync_required`,
then the close. A healthy paired client asking every 50 ms meanwhile:
p50 10.1 ms, p99 23.5 ms, no failures, which is random arrival against
a 22 ms interval. The frame's pass cost did not move.

**Stall.** 20 raw TCP connections held open without a handshake:
the already-paired connection answered in 24 ms as before; a new
connection was refused in 0.6 ms (the pool of 4 was full and the
accept loop dropped it); a new connection was served again 4.85 s
later, when the 5 s handshake timeout cleared the four held slots.
So an unauthenticated flood costs a new device up to 5 s and costs an
established device nothing, which is what §4.1's separate pools are
for.

**Pin.** A client pinned to another hash fails at the handshake with
"certificate is not the pinned one"; no request is sent.

**Bonjour.** `mdns-sd` registered `_switchboard._tcp` with TXT `v=1`
and the display name, and `dns-sd -B _switchboard._tcp local.` listed
"Spike 22" within the second. Note that two instances on one machine
need distinct instance names, which the display name gives.

**Transcript parse** (the app's own parser, release build, the
largest transcripts in this machine's data; peak RSS is the test
process):

| file | read | parse | turns | kept text | peak RSS |
|---|---|---|---|---|---|
| 146 MB | 55 ms | 237 ms | 402 | 1.5 MB | 575 MB |
| 80 MB | 20 ms | 84 ms | 176 | 0.46 MB | 260 MB |

So the app's comment that "several megabytes costs tens of
milliseconds" holds per megabyte, and the data directory holds
sessions a hundred times that size: **290 ms per read, or eighteen
frames, which settles §4.1: content work runs on a connection thread,
never the frame.** Memory is the whole file plus its JSON, four times
the file; the parsed conversation the cache keeps is 1.5 MB. Two of
these in flight at once is over a gigabyte of transient memory, which
argues for one content worker, not one per connection, or for a
parser that streams lines (out of scope; `§2`).

**Content versions** (`tests/versions.rs`, all pass):

- a same-size rewrite with mtime restored by `touch -r` leaves (mtime,
  size) unchanged and moves ctime;
- a replace-by-rename moves the inode;
- a log truncated and regrown to the same length moves ctime;
- an appended transcript is strictly larger after each append, so size
  alone versions it.

## Recommendation

Build the port as `docs/remote-client.md` says, with three amendments
from the numbers: the content reads go to **one worker thread** with a
queue, not to each connection thread, so that at most one large parse
is in memory at a time; the latency the client should expect is one
frame interval, so a visible view's periodic refetch need not be faster
than a few seconds; and the Bonjour instance name is the display name
plus a short id suffix when two instances share a machine.

## Left to the owner

- The `URLSession` and `URLSessionWebSocketTask` pinning delegate on
  visionOS against this server (`--bonjour` to discover it); the
  server's pin is in `pin.txt` after a run.
- The read-model diff cost over a few hundred records, which needs the
  app's views; the serialization of a few hundred small structs is
  well under the pass budget by the parse numbers above, but it is not
  measured.
