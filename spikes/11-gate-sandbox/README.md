# Spike 11: confining Dispatch's pipeline commands

Question: can the commands Dispatch runs as its own children (a lane's
setup, command gates, a review round's checks, command reviewers) be
kept from writing anywhere but the ticket's tree, on macOS, without
root, and still run the real toolchains? And when a write is refused,
can the check's log say which path?

Mechanism: `/usr/bin/sandbox-exec -p <profile> -D NAME=value … -- argv`.
The profile allows everything, denies every file write, then allows
writes under the parameters it is given. Paths reach it only as `-D`
parameters, never as profile text. `profile.sb` here is the shape
`dispatch/src/confine.rs` builds, with two writable paths:

```scheme
(version 1)
(allow default)
(deny file-write* (with message (param "TOKEN")))
(allow file-write* (subpath (param "W0")) (subpath (param "W1"))
  (subpath (param "TMP")) (literal "/dev/null") (literal "/dev/tty")
  (literal "/dev/dtracehelper") (literal "/dev/ptmx")
  (regex #"^/dev/ttys[0-9]+$") (subpath "/dev/fd"))
; only with network = "deny":
(deny network-outbound (remote ip))
(allow network-outbound (remote ip "localhost:*"))
```

Measured on 2026-10-04, macOS 15 (Darwin 24.6), Apple silicon.

## The basics

```sh
W=$(pwd -P); T=$(cd "$TMPDIR" && pwd -P)
/usr/bin/sandbox-exec -f profile.sb -D W0=$W -D W1=$W -D TMP=$T -D TOKEN=dispatch-tok123 -- \
  sh -c 'touch ok; touch "$HOME/.dispatch-probe-x"; echo s=$?'
```

```
touch: /Users/…/.dispatch-probe-x: Operation not permitted
s=1
```

- Runs without root. `ok` is made; the probe is not.
- Start-up: 6.1 ms per run against 1.2 ms for a bare `/usr/bin/true`
  (mean of 50, Python `subprocess.run`), so about 5 ms.
- A refused write reaches the tool as `EPERM`.
- Seatbelt matches resolved paths: `/tmp` is `/private/tmp`, `$TMPDIR`
  is under `/private/var/folders`. Every path is canonicalized first; a
  path that does not exist yet keeps its missing tail on top of its
  longest existing ancestor, resolved.

## The deny lines, per check

The kernel logs each refused write. The `(with message …)` modifier on
the deny rule puts the check's token on the line, so a query can pick
this check's lines and no other sandbox's (Claude Code's own sandbox
logs the same kind of line):

```sh
/usr/bin/log show --start "$start" --style compact \
  --predicate 'eventMessage CONTAINS "dispatch-tok123"' | grep 'deny('
```

```
… kernel[0:6c7a9e] (Sandbox) Sandbox: touch(36332) deny(1) file-write-create /Users/…/.dispatch-probe-x
```

- No root needed.
- The token can be a parameter: `(with message (param "TOKEN"))` works,
  so the profile text is the same for every check.
- `log show` over a window of under a minute takes 0.66 to 0.69 s.
- The line was visible to a query started right after the command
  exited in 5 of 5 tries by hand, but `GitCli`'s test missed it once in
  three runs. The wrapper therefore asks again once, a second later,
  when the first query finds nothing: 10 of 10 after that. A failed
  check with no refused write pays about 2.4 s for the two queries.
- The line ends with the path, so it can be appended as is. The
  wrapper prefixes `dispatch: sandbox: `.
- A burst (Python probing every `/dev/pty??` name) logs hundreds of
  lines; the query keeps all of them.

## What real commands need

Every command below ran with only the tree, the base set and the named
paths writable, and its deny lines were read back by token.

| Command | Result | Writes outside the tree |
| --- | --- | --- |
| `cargo fetch --locked` (Switchboard) | passes even with `~/.cargo` read-only (everything cached) | `~/.cargo/.package-cache`, `.package-cache-mutate`, `.global-cache`, xattrs on `~/.cargo/registry` and `~/.cargo/git`: give the lane `~/.cargo` |
| `cargo test --locked --workspace` (Switchboard), with `~/.cargo` and `/tmp` writable | 3 failures | `/dev/ptmx` (tmux, a terminal test), fixed by the base set; `/bin/ps` cannot start (`Operation not permitted`, exit 126) because a sandboxed process cannot exec a `setuid` program; `security create-keychain` in the throwaway-keychain test fails ("A Module Directory Service error has occurred") with no deny line; `xcodebuild` deletes its plug-in cache under `$DARWIN_USER_CACHE_DIR` (harmless) |
| `cargo test -p dispatch` without `/tmp` | 2 failures | `health.rs` tests make sockets under `/tmp` on purpose (short paths): the lane needs `/tmp` |
| the client project's backend setup: `uv sync --frozen --all-groups && mkdir -p local/caches && uv run --frozen --all-groups inv link-env --env-name my-dev`, fresh clone | passes in 19 s, no deny lines, with `~/.cache/uv` writable | only `~/.cache/uv` (without it: `failed to open file ~/.cache/uv/sdists-v9/.git`). `inv link-env` writes one file, `local/caches/currently_linked_env`, inside the tree; nothing under `~` or `/tmp` |
| the client project's backend gate, `uv run inv lint && git diff --exit-code && uv run inv pytest` | first run failed: `invoke` opens a pty, and `/dev/ptmx` was not allowed (`OSError: out of pty devices`) | `/dev/ptmx` and `/dev/ttys*`, now in the base set. Not run again: see "Not measured" |
| `git add` in a linked worktree | fails on `<clone>/.git/worktrees/<name>/index.lock` unless that directory is writable; with it writable, fails writing objects into the clone's shared `objects/` | the worktree's own git directory is in the base set; the shared `.git` is not, so a gate cannot commit. `git status` and `git diff` pass |
| system Python (`/usr/bin/python3`) | runs | tries to write `.pyc` files under `~/Library/Caches/com.apple.python`; refused, harmless |

Python's `os.openpty()` works once `/dev/ptmx` and `/dev/ttys[0-9]+` are
allowed:

```sh
/usr/bin/sandbox-exec -f profile.sb … -- python3 -c 'import os; m,s=os.openpty(); print(os.ttyname(s))'
/dev/ttys009
```

## The network

```sh
… -D … -- sh -c 'curl -sS -m 3 -o /dev/null https://example.com; echo curl=$?;
  python3 -m http.server 18765 --bind 127.0.0.1 & sleep 1;
  curl -sS -m 3 -o /dev/null http://127.0.0.1:18765/; echo local=$?;
  curl -sS -m 3 -o /dev/null http://localhost:18765/; echo localname=$?'
```

```
curl: (7) Failed to connect to example.com port 443 after 18 ms: Couldn't connect to server
curl=7
local=0
localname=0
```

`(deny network-outbound (remote ip))` alone would also close loopback;
`(allow network-outbound (remote ip "localhost:*"))` keeps it, so a test
against a local database still runs. Name resolution still works (it
goes through `mDNSResponder`'s socket, not an IP connection). Unix
sockets, such as Docker's, are not affected.

## Signals

`sh -c '"$@"; …' sh <token> /usr/bin/sandbox-exec … -- argv` sees a
status over 128 for a command killed by a signal. The wrapper then
re-raises that signal on itself (`trap - $sig; kill -$sig $$`), so the
runner reads a signal death, `status.code()` is `None`, and
`poll_check` returns `-1`, the same as for the bare command. Pinned by
`a_confined_check_killed_by_a_signal_polls_as_minus_one` in
`dispatch/src/git.rs`. A `kill_check` TERM to the process group reaches
the wrapper's `sh` too, which dies of it directly.

## Not measured

- The client project's backend gate (`inv lint`, `inv pytest`) after
  the pty fix, its frontends' `npm ci --legacy-peer-deps` and
  `CI=true npm test`, and the deploy gate's `aws-vault exec`: running
  the client project's code was not approved in the session that wrote
  this. Its pipeline keeps `confine = false` until they pass; the
  setup above already has.
- The simplesingletable gate: its tests need DynamoDB Local in Docker,
  which was not running.
- Whether `(allow default)` covers what `aws-vault` needs from the
  Keychain. Keychain creation failed above, so check this before
  confining a deploy gate.

## Recommendation

Build it with the profile above, the base set (`$TMPDIR`, `/dev/null`,
`/dev/tty`, `/dev/fd`, `/dev/dtracehelper`, `/dev/ptmx`, `/dev/ttys*`, a
linked worktree's own git directory) and caches declared per lane.
Leave `confine` off by default. Switchboard's own pipeline cannot be
confined while its tests start `/bin/ps` and create a keychain; give it
`writable = ["~/.cargo", "/tmp"]` anyway so it is ready if those tests
change.
