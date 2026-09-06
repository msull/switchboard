# Spike 02: what hosts session processes?

Question 2 of Spike 0 in `docs/design.md`: should Switchboard's sessions run
under a **tmux server** (a warm cache that outlives app restarts) or under
**PTYs the app owns directly** (dead when the app is)?

Answer, backed by the experiments below: **tmux as the process host**, with
app-owned PTYs kept only as a fallback adapter behind the same port. The
details, numbers, and a proposed `ProcessHost` trait are at the end.

Everything here was found by running the programs in `examples/`, not by
reading about it. Outputs are pasted verbatim except where cut for length.

## Running it

```sh
cd spikes/02-process-host
cargo build
cargo run --example tmux_host        # Q2, Q5, Q6: spawn, capture, liveness, latency, pipe-pane
cargo run --example tmux_reattach    # Q2: a NEW process finds the session and kills the server
cargo run --example tmux_control     # Q3: control mode over pipes
cargo run --example tmux_env         # Q5: the three ways env reaches a tmux pane
cargo run --example pty_host         # Q4, Q5, Q6: portable-pty spawn, resize, exit, tee, orphans
```

The crate is standalone (its own `[workspace]` table), edition 2024, and
forbids `unsafe` (`portable-pty` uses it internally; that is fine, the ban is
on our code). `cargo clippy --all-targets -- -D warnings` is clean with the
same pedantic lint set as the app.

Requires `brew install tmux` (>= 3.2; tested with 3.7c). `SWITCHBOARD_TMUX=<path>`
optionally points the examples at a different binary, e.g. a bundled one.

Every tmux server the spike starts is on a socket named `switchboard-spike-*`
(`tmux -L`) with `-f /dev/null`, so it never sees the user's sessions or
config. `tmux_reattach`, `tmux_control`, and `tmux_env` kill their own
servers at the end. Set `KEEP=1` on `tmux_reattach` to leave the host server
running for a look with `tmux -L switchboard-spike-host attach`.

## 1. tmux availability

Tested against **tmux 3.7c from Homebrew** (`brew install tmux`), on macOS
15 / Darwin 24.6, Apple Silicon:

```
$ which tmux
/opt/homebrew/bin/tmux
$ tmux -V
tmux 3.7c
```

tmux was **not installed when this spike started**; macOS does not ship
it. The first pass ran against a Homebrew bottle unpacked into a scratch
directory with its dylib paths rewritten (`install_name_tool` on the
`@@HOMEBREW_PREFIX@@` placeholders for libevent, ncurses, utf8proc, and
jemalloc, then `codesign -s -`). After `brew install tmux` every example
was rerun against the real binary and all findings and numbers below are
from that rerun; they matched the relocated-bottle run within noise.

Two things worth keeping from that detour:

- **tmux is a hard external dependency** the app must detect (`tmux -V`,
  need >= 3.2 for `new-session -e` and `pane_dead_status`) and explain how
  to get: `brew install tmux`.
- **Bundling is feasible.** The relocation exercise is exactly what
  shipping tmux inside the app bundle would take: the binary plus four
  dylibs (or a static build), re-signed under the app's identity. tmux is
  ISC licensed. The examples honor an optional `SWITCHBOARD_TMUX=<path>`
  override for that case; unset, they use `tmux` from `PATH`.

## 2. tmux as the host, driven from Rust (`tmux_host`, `tmux_reattach`)

Each tmux command is `std::process::Command` running `tmux -L <socket> -f
/dev/null <args>`; see `src/lib.rs::Tmux`. Everything worked first time
except one gotcha noted below.

**Spawn with cwd and env, then check the pane:**

```
$ tmux -L switchboard-spike-host new-session -d -s sess -c /var/folders/.../T/ -e SWITCHBOARD_TEST=hello-from-tmux -x 120 -y 40
  (ok, no output)
$ tmux -L switchboard-spike-host list-panes -t sess -F pid=#{pane_pid} dead=#{pane_dead} cmd=#{pane_current_command} path=#{pane_current_path}
  pid=99806 dead=0 cmd=zsh path=/private/var/folders/.../T
```

`-e` (tmux >= 3.2) sets the variable in the session's environment; `-c` is
the cwd; `pane_pid` is the shell's pid, `pane_current_command` the
foreground process name, `pane_current_path` its cwd. That is the "is it
alive, what is it doing, where" line the board needs, from one call.

**send-keys and capture-pane:**

```
$ tmux -L switchboard-spike-host send-keys -t sess 'echo VAR=$SWITCHBOARD_TEST; echo CWD=$PWD; [[ -o login ]] && echo LOGIN=yes; [[ -o interactive ]] && echo INTERACTIVE=yes; echo ZSHRC_MARKER=$SAM_CLI_TELEMETRY; echo SHELL_PID=$$' Enter
$ tmux -L switchboard-spike-host capture-pane -p -S - -t sess
  ...
  ❯ echo VAR=$SWITCHBOARD_TEST; echo CWD=$PWD; ...
  VAR=hello-from-tmux
  CWD=/var/folders/yc/w_bz19lj32g4rv_3773ck1480000gn/T/
  LOGIN=yes
  INTERACTIVE=yes
  ZSHRC_MARKER=0
  SHELL_PID=99806
  ❯
```

`capture-pane -p -S -` returns the pane's text from the top of history,
already rendered (no escape codes; add `-e` to keep colors).

**Exit detection** needs `remain-on-exit` or the window disappears the
instant its command ends. Gotcha: it is a *window* option, and `set-option
-t sess remain-on-exit on` only touches the session's current window.
Setting it globally on the private server (`-gw`) is the reliable form.

```
$ tmux -L switchboard-spike-host set-option -gw remain-on-exit on
$ tmux -L switchboard-spike-host new-window -d -t sess -n job sh -c 'echo working; sleep 1; exit 3'
$ tmux -L switchboard-spike-host list-panes -s -t sess -F win=#{window_name} pid=#{pane_pid} dead=#{pane_dead} status=#{pane_dead_status} cmd=#{pane_current_command}
  win=zsh pid=99806 dead=0 status= cmd=zsh
  win=job pid=1107 dead=0 status= cmd=bash
(1.2 s later)
  win=zsh pid=99806 dead=0 status= cmd=zsh
  win=job pid=1107 dead=1 status=3 cmd=sh
```

`pane_dead=1` plus `pane_dead_status=3` is exactly the "exited (code 3)"
card state in the design. A `pane-died` hook (`set-hook -g pane-died
'run-shell ...'`) can push that instead of polling; not exercised here.

**Survives the app process.** `tmux_host` exits leaving the session; a
separate program then finds it, sends more keys into the *same shell pid*,
and reads the whole history:

```
$ cargo run --example tmux_reattach
$ tmux -L switchboard-spike-host has-session -t sess
  (ok, no output)
  session 'sess' is alive from a previous process
$ tmux -L switchboard-spike-host list-panes -s -t sess -F ...
  win=zsh pid=99806 dead=0 cmd=zsh
  win=job pid=1107 dead=1 cmd=sh
$ tmux -L switchboard-spike-host send-keys -t sess 'echo REATTACHED_PID=$$' Enter
$ tmux -L switchboard-spike-host capture-pane -p -S - -t sess
  ...
  SHELL_PID=99806
  ❯ echo REATTACHED_PID=$$
  REATTACHED_PID=99806
$ tmux -L switchboard-spike-host list-sessions -F '#{session_name} created=#{session_created} attached=#{session_attached}'
  sess created=1788675092 attached=0
$ tmux -L switchboard-spike-host kill-server
$ tmux -L switchboard-spike-host has-session -t sess
  ERROR: no server running on /private/tmp/tmux-501/switchboard-spike-host
```

The dead `job` pane, its exit status, and the scrollback all survived too.
`list-sessions` is what the app runs at startup to see which records are
warm. `session_created` lets it reconcile against the workspace record.

**Latency** (debug build, M-series Mac, Homebrew tmux 3.7c, 50 runs each):

```
capture-pane -p -S -:                          min=2.70ms mean=2.91ms max=3.48ms
list-panes -F:                                 min=2.72ms mean=2.96ms max=3.42ms
send-keys (no wait for output):                min=2.45ms mean=2.91ms max=3.93ms
send-keys + poll capture-pane until visible:   min=4.94ms mean=5.75ms max=17.78ms
```

The 2.5 ms floor is `fork`+`exec` of the tmux client plus its socket round
trip; the actual server work is microseconds (see control mode). It is
cheap enough to poll a board at 1 to 2 Hz, and one `list-panes -a -F` call
covers every session on the server, so the cost does not scale with the
number of cards.

## 3. tmux control mode from Rust (`tmux_control`)

Practical, yes, with three quirks that each cost a debugging round.

Setup: create the session detached, then spawn `tmux -C attach-session -t
ctl` with `Stdio::piped()` on stdin and stdout. No tty is needed; `-C`
(one C) is right for pipes, `-CC` only adds terminal echo handling for
interactive use. A reader thread turns stdout into lines on a channel; the
main thread writes tmux commands to stdin as text lines.

```
-- greeting on attach
  +  2.80ms raw: %begin 1788675098 284 0
  +  2.81ms raw: %end 1788675098 284 0
  +  2.82ms raw: %session-changed $0 ctl

-- after writing "send-keys -t ctl 'echo hello; echo VAR=$SWITCHBOARD_TEST' Enter"
  +176.83µs raw: %begin 1788675772 289 1
  +194.33µs raw: %end 1788675772 289 1
  raw: %output %0 echo hello; echo VAR=$SWITCHBOARD_TEST\015\012
  raw: %output %0 \033[1m\033[7m%\033[27m\033[1m\033[0m   ...
  raw: %output %0 he
  raw: %output %0 ll
  raw: %output %0 o
  ...
  raw: %output %0 \033]133;C\007\033]2;echo hello; ...\007\033[0 qhello\015\012VAR=hello-from-control\015\012...

-- after writing "list-panes -t ctl -F 'pid=#{pane_pid} dead=#{pane_dead} cmd=#{pane_current_command}'"
  +161.46µs raw: %begin 1788675099 293 1
  +189.79µs raw: pid=1192 dead=0 cmd=zsh
  +195.54µs raw: %end 1788675099 293 1

-- after writing "no-such-command"
  +293.38µs raw: %begin 1788675099 294 1
  +429.67µs raw: parse error: unknown command: no-such-command
  +487.25µs raw: %error 1788675099 294 1

-- decoded %output stream contains:
  hello
  VAR=hello-from-control
```

Command replies arrive in **150 to 450 µs** on the persistent connection
(same range on the real install as on the relocated bottle),
versus 2.5 ms per spawned client. Pane output is **pushed** as `%output`
the moment it happens; nothing to poll.

Protocol notes, all observed:

- **Framing.** Each command's reply is `%begin <time> <n> <flags>`, zero or
  more body lines, then `%end` (success) or `%error` (failure) with the same
  `<n>`. Notifications (`%output`, `%session-changed`, `%exit`, ...) are
  single lines outside any block and can interleave anywhere.
- **Escaping.** `%output %<pane-id> <bytes>`: bytes below 0x20 and the
  backslash are written as three-digit octal (`\033`, `\015`, `\012`,
  `\007`). Bytes 0x80 and above pass through **unescaped**.
- **Chunking is byte-arbitrary.** A chunk ends wherever the pty read did,
  including in the middle of a multibyte character. In one run a
  Powerline glyph arrived as `...\033[1;35m<0xEE>` on one line and
  `<0x82 0xA0> main...` on the next. This is why the first version hung:
  `BufRead::lines()` fails on invalid UTF-8, the reader thread quit, the
  stdout pipe filled, tmux blocked writing and never read its stdin again.
  Read `%output` as bytes (`read_until(b'\n')`) and feed a VT parser; never
  treat a line as a `String` first.
- **Typed input is echoed one key at a time** (`he`, `ll`, `o` above),
  because `send-keys` delivers key by key. Harmless for a terminal
  emulator, confusing if you expected lines.
- **Detach:** closing stdin ends the client cleanly (`%exit`, exit 0) *as
  long as its stdout is being drained*. Belt and braces: write
  `detach-client` first.
- Sessions keep running after the client goes: `has-session` succeeded
  right after the `%exit`.

Verdict: about 100 lines of Rust (`parse`, `unescape`, a reader thread)
gets a usable client. A full VT-state-aware consumer would sit behind it.
Control mode is the right transport for the session view; polling is
enough for the board.

## 4. App-owned PTYs with `portable-pty` (`pty_host`)

```
== spawn login shell in a pty with cwd + env
  shell=/bin/zsh pid=Some(3941)
== write probe command
  got END1: true
== resize to 50x200, then ask the shell what it sees
  got END2: true
== latency: write `echo pong<N>` and wait until the reply lands in the buffer
  write->echo round trip: n=50 min=22.46ms mean=24.96ms max=69.52ms
== latency without a shell: a pty running `cat`, write a line, wait for its echo
  write->cat echo round trip: n=50 min=5.25µs mean=25.12µs max=979.96µs
== exit detection
  before exit: try_wait = Ok(None)
  after `exit 7`: wait = exit_code 7 success=false
== raw pty output (escape sequences shown as-is)
  ...
  VAR=hello-from-pty
  CWD=/private/var/folders/yc/w_bz19lj32g4rv_3773ck1480000gn/T
  LOGIN=yes
  INTERACTIVE=yes
  ZSHRC_MARKER=0
  SIZE=50 200
  END1
  ...
== scrollback file: target/spike-out/pty-scrollback.log (30895 bytes)
```

Notes:

- `native_pty_system().openpty(size)`, `CommandBuilder` with `.cwd()` and
  `.env()`, `slave.spawn_command(cmd)`; drop the slave after spawning or
  EOF never arrives. `master.try_clone_reader()` / `take_writer()` give
  plain `Read`/`Write`, `master.resize()` sends SIGWINCH (the shell saw
  `50 200`), `child.try_wait()` / `wait()` give the exit code.
- The 25 ms "round trip" is the shell, not the transport: the prompt here
  is starship, which runs git status between commands, and zsh does not
  echo the next line until the prompt is drawn. The `cat` test shows the
  pty itself costs **~25 µs**, two orders of magnitude under a tmux client
  spawn and on par with control mode.
- Output is raw bytes with every escape sequence; the app would need a VT
  parser (`alacritty_terminal` / `vt100`) before it can show a "last line"
  caption, whereas `capture-pane -p` hands back rendered text.
- Reader thread with `Arc<Mutex<Vec<u8>>>`: in Rust the pty reader is
  moved into the thread and the buffer is shared through the `Arc`
  (reference count) and `Mutex` (one writer at a time). A hot spin on that
  mutex starved the reader thread in the first version; poll with a small
  sleep or use a channel.

**What happens to children when the host exits.** `pty_host` ends by
spawning `sleep 300` and `sh -c "trap '' HUP; exec sleep 300"` in two fresh
ptys and returning from `main` without waiting:

```
  plain child pid: Some(4682)  (expected to die with the host)
  HUP-ignoring child pid: Some(4683)  (expected to survive, orphaned, unreachable)
$ sleep 1; ps -o pid,ppid,stat,tty,command -p $(cat target/spike-out/orphan-pids)
  PID  PPID STAT TTY      COMMAND
(exit=1; no rows: both children are gone)
```

Both were gone within a second, in three separate runs. `portable-pty` has
no kill-on-drop (its only `Drop` writes newline+EOF into the master); the
child is made a session leader (`setsid`) with the pty as its controlling
terminal, and closing the master tears that terminal down. Even the child
that ignored SIGHUP did not survive, so on macOS this is not something a
process can opt out of from inside. Keeping a process alive would mean
double-forking it away from the pty entirely, at which point its output
has nowhere to go: there is no reattach without a daemon holding the
master, which is what tmux is.

Note the app-side flip side: a **crash** of Switchboard kills every
session under this model. Under tmux, a crash loses nothing.

## 5. Environment injection

Both hosts pass the variable and both run the user's login shell init:
`VAR=...`, `LOGIN=yes`, `INTERACTIVE=yes`, `ZSHRC_MARKER=0` appear in
sections 2 and 4 (`SAM_CLI_TELEMETRY=0` is exported by `~/.zshrc`, so it
can only be there if `.zshrc` ran). tmux runs `default-shell` (from
`$SHELL`) as a login shell by default; with `portable-pty` the spike
passed `-l` explicitly and the pty made it interactive.

tmux has three routes, and `tmux_env` shows which reach a pane:

```
== 1. the server inherits the environment of whoever starts it
$ tmux ... show-environment -g SWITCHBOARD_SERVER_BORN_WITH
  SWITCHBOARD_SERVER_BORN_WITH=yes
== 2. new-session -e sets a per-session variable (tmux >= 3.2)
$ tmux ... show-environment -t b SWITCHBOARD_TEST
  SWITCHBOARD_TEST=via-dash-e
== 3. set-environment on a session affects panes created afterwards, not existing ones
  a:0    VAR=                        (window made before set-environment)
  a:later VAR=via-set-environment    (window made after)
  b      VAR=via-dash-e
== 4. update-environment: copied from an attaching client (DISPLAY, SSH_AUTH_SOCK, ...)
```

Implications for the design's env profiles:

- Use `new-session -e` / `new-window -e` per session; that is the
  per-record env profile. tmux 3.2+ required (3.7c here).
- The server's global environment is frozen at whatever started it. If
  Switchboard starts the server, the server carries Switchboard's launch
  env (a GUI app's, which is not a login shell's). Since the login shell
  inside each pane runs `.zprofile`/`.zshrc` anyway, PATH and friends come
  out right; secrets from the app's env do not leak in unless passed with
  `-e`, which is the desired behavior.
- Changing a profile does not reach a running pane on either host. That
  is inherent to processes, not to tmux.

## 6. Scrollback to disk

**tmux:** `pipe-pane -o -t sess "cat >> file"` streams every byte the pane
produces to the file from that moment on, including escape sequences:

```
$ cat target/spike-out/tmux-scrollback.log
  echo VAR=$SWITCHBOARD_TEST; ...
  [1m[7m%[27m[1m[0m ... ]7;kitty-shell-cwd://... ]133;A ...
  [1;32m❯[0m [K[5 q[?2004heecho VAR=$SWITCHBOARD_TEST; ...
  VAR=hello-from-tmux
  CWD=/var/folders/yc/w_bz19lj32g4rv_3773ck1480000gn/T/
  ...
```

It survives app restarts along with the session (the pipe is the
server's), and `-o` makes the command a toggle so a second call turns it
off rather than doubling up. `capture-pane -p -S - -e` at reattach time
fills in anything before the pipe started, rendered.

**PTY:** the reader thread writes each chunk to the file before appending
it to the in-memory buffer (`pty_host`, "tee"); 30 895 bytes for the run
above, byte-identical to what the terminal would have seen. Stops when
the app stops, like everything else in that model.

Both files are raw VT streams, not text. A "readable when nothing is
running" view (design: scrollback on disk) needs either a stripping pass
or a replay through a VT parser. tmux's `capture-pane -p` (no `-e`) is the
cheap way to get plain text while the session lives; for the on-disk
file, strip on read.

## Comparison

| | tmux server | App-owned PTYs (`portable-pty`) |
|---|---|---|
| Survives app restart | **Yes**, processes, scrollback, exit status all intact (section 2) | No; both test children gone within 1 s of host exit (section 4) |
| Survives app crash | Yes, same | No |
| Survives reboot | No (neither does; that is what the workspace record is for) | No |
| Reattach | `has-session` / `attach` / `capture-pane`, any process, any time | None without a daemon holding the master |
| Liveness / exit code | `list-panes -F` with `pane_dead`, `pane_dead_status`, `pane_pid`, `pane_current_command`; `pane-died` hook available | `try_wait()` gives exit code; foreground command name needs `proc_pidinfo` per pid |
| Latency, command | 2.5 ms per spawned client; 0.1 to 0.5 ms over control mode | 25 µs pty round trip |
| Latency, output | Poll (`capture-pane`, 3 ms) or push (`%output`, immediate) | Push (blocking read), immediate |
| Rendered text | `capture-pane -p` gives plain text for captions and search | Raw VT bytes; needs a parser |
| Code size in spike | `lib.rs` 106 + `tmux_host` 139 + `tmux_reattach` 60 + `tmux_control` 233 lines | `pty_host` 215 lines, and a VT parser still to come |
| Dependencies | tmux >= 3.2 binary (not on macOS by default; brew or bundle with 4 dylibs), `std` only in Rust | `portable-pty` (pure Rust over libc, uses `unsafe` internally), no external binary |
| Env and login shell | `-e` per session; login shell by default; server env frozen at start | `.env()` per spawn; `-l` explicit |
| Scrollback to disk | `pipe-pane -o`, outlives the app | tee in the reader thread, dies with the app |
| Risk | Missing binary; version skew (`-e`, `pane_dead_status` need >= 3.2); control-mode byte handling; sharing a machine with the user's own tmux (mitigated by a private socket and `-f`) | Every crash or update kills all sessions, which contradicts priority 1; a VT parser is unavoidable before anything can be shown |

## Recommendation

**tmux is the process host.** It is the only option that keeps a running
agent alive across an app restart, crash, or update, which is the first
priority in the design, and it gives reattach, liveness, exit codes,
rendered text, and scrollback-to-disk for the price of one external
binary. Own-PTY hosting is faster on the wire but it makes every running
session as fragile as the GUI process, and the spike could not find a
detach path on macOS that leaves output reachable.

The shape that falls out of the numbers:

- **One private server** per user (`tmux -L switchboard -f
  <app-config>`), never the user's default socket. Set `remain-on-exit on`
  globally on it so exits are observable. Session names come from the
  workspace record's session id, so records and warm processes reconcile
  by name.
- **Board state by polling:** one `list-panes -a -F ...` every 500 ms to
  1 s (2.5 ms of CPU) covers every card. Add `window_activity` to the
  format for "last output at". Later, a `pane-died` hook can push instead.
- **Session view by control mode:** one long-lived `tmux -C attach` per
  open view, `%output` bytes fed to the VT parser the terminal spike
  (question 4) picks. Byte-oriented reader, `detach-client` on close.
- **Scrollback:** `pipe-pane -o` started at spawn and re-asserted at
  reattach (it is a toggle: check `#{pane_pipe}` first). Plain-text view
  via `capture-pane -p` while alive.
- **tmux missing:** detect at startup (`tmux -V`, parse >= 3.2). Milestone
  1 can require `brew install tmux` and say so; bundling (binary plus
  dylibs, re-signed) is a known, tested path if that ever matters.
- **Keep the PTY adapter** as a second implementation of the same port
  for the no-tmux case and for tests: the fake host in the design plays
  scripted output through the same trait.

### Proposed `ProcessHost` port

The trait is written for the tmux adapter but says nothing tmux-specific.
`SessionId` is the durable name stored in the workspace record. Everything
that can block goes through `std::io::Result`, and output arrives on a
channel so the app can drain it on its worker loop.

```rust
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::SystemTime;

/// Durable identity of a hosted session: the workspace record's session id.
/// The tmux adapter uses it as the session name; the PTY adapter as a map key.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SessionId(pub String);

/// Everything needed to start (or restart) a session. Comes straight from
/// the workspace record after env-profile resolution.
#[derive(Clone, Debug)]
pub struct SpawnSpec {
    pub id: SessionId,
    pub cwd: PathBuf,
    /// `None` runs the user's login shell; `Some` runs this argv inside it.
    pub command: Option<Vec<String>>,
    pub env: Vec<(String, String)>,
    pub size: TermSize,
    /// Where the raw output stream is appended (`pipe-pane` / tee).
    pub scrollback: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TermSize {
    pub cols: u16,
    pub rows: u16,
}

/// What the host knows about a session without reading its screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Liveness {
    /// Process is alive. `command` is the foreground process name
    /// (`pane_current_command`), the cheapest "what is it doing" signal.
    Running { pid: u32, command: String },
    /// Process ended; the pane is kept so this can be read (`remain-on-exit`).
    Exited { code: Option<i32> },
    /// No such session on the host: cold record, needs `spawn`.
    Missing,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionStatus {
    pub liveness: Liveness,
    /// Current directory of the foreground process, if known.
    pub cwd: Option<PathBuf>,
    /// When the session last produced output (`window_activity`).
    pub last_activity: Option<SystemTime>,
}

/// Pushed by a subscription. Bytes are the raw terminal stream; the
/// `Terminal` port turns them into cells.
#[derive(Clone, Debug)]
pub enum HostEvent {
    Output { id: SessionId, bytes: Vec<u8> },
    Exited { id: SessionId, code: Option<i32> },
    /// The session vanished (killed elsewhere, server died).
    Gone { id: SessionId },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal {
    Interrupt,
    Terminate,
    Kill,
    Hangup,
}

/// Hosts session processes. Implemented by `TmuxHost` (the choice),
/// `PtyHost` (fallback when tmux is absent), and `FakeHost` (tests).
///
/// `Send + Sync` because the app calls it from worker threads; `&self`
/// methods because the host owns no per-call state beyond the server.
pub trait ProcessHost: Send + Sync {
    /// Is the host usable at all (tmux present and new enough)? The app
    /// shows the reason when it is not.
    fn probe(&self) -> Result<HostInfo, String>;

    /// Sessions the host currently has, warm or dead-but-kept. Run at
    /// startup to reconcile against workspace records.
    fn list(&self) -> std::io::Result<Vec<SessionId>>;

    /// Start a session. Fails if one with this id already exists; call
    /// `status` first and reuse it when it is `Running`.
    fn spawn(&self, spec: &SpawnSpec) -> std::io::Result<()>;

    fn status(&self, id: &SessionId) -> std::io::Result<SessionStatus>;

    /// Raw bytes to the process's stdin (keystrokes, pasted text).
    fn write(&self, id: &SessionId, bytes: &[u8]) -> std::io::Result<()>;

    fn resize(&self, id: &SessionId, size: TermSize) -> std::io::Result<()>;

    /// The screen and history as rendered text, most recent `lines` lines
    /// (`None` for everything). What cards show as their caption and what
    /// "scrollback when nothing is running" falls back to.
    fn snapshot(&self, id: &SessionId, lines: Option<usize>) -> std::io::Result<String>;

    /// Live output. Ends when the session exits or `unsubscribe` is called.
    /// tmux: a control-mode client; PTY: the reader thread.
    fn subscribe(&self, id: &SessionId) -> std::io::Result<Receiver<HostEvent>>;
    fn unsubscribe(&self, id: &SessionId);

    fn signal(&self, id: &SessionId, signal: Signal) -> std::io::Result<()>;

    /// Remove the session and its process. The scrollback file stays.
    fn kill(&self, id: &SessionId) -> std::io::Result<()>;
}

#[derive(Clone, Debug)]
pub struct HostInfo {
    /// e.g. "tmux 3.7c" or "pty (no persistence)"
    pub description: String,
    /// Whether sessions outlive the app. Drives the UI's wording for
    /// "quit": with `false`, quitting means stopping every session.
    pub persistent: bool,
}
```

Mapping for `TmuxHost`, all exercised in this spike:

| Method | tmux command |
|---|---|
| `probe` | `tmux -V`, parse version |
| `list` | `list-sessions -F '#{session_name}'` |
| `spawn` | `new-session -d -s <id> -c <cwd> -e K=V... -x -y [cmd]`, then `pipe-pane -o -t <id> "cat >> <scrollback>"` |
| `status` | `list-panes -t <id> -F '#{pane_dead} #{pane_dead_status} #{pane_pid} #{pane_current_command} #{pane_current_path} #{window_activity}'` |
| `write` | `send-keys -t <id> -H <hex bytes>` (`-l` for literal text) |
| `resize` | `resize-window -t <id> -x -y` (needs `window-size manual` on a detached session) |
| `snapshot` | `capture-pane -p -t <id> -S -<lines>` (`-e` to keep colors) |
| `subscribe` | spawn `tmux -C attach -t <id>` on pipes, byte reader, `%output` -> `HostEvent::Output` |
| `signal` | `kill -<sig> <pane_pid>` from `status` |
| `kill` | `kill-session -t <id>` |

Open items for milestone 1, not blockers: bundling vs. requiring tmux;
whether one control client per view or one per server (`%output` carries
the pane id, so one is enough); and stripping the on-disk VT stream for
the read-only scrollback view.

## Layout

| Path | Purpose |
|---|---|
| `Cargo.toml` | Standalone crate, edition 2024, `unsafe` forbidden, pedantic clippy |
| `src/lib.rs` | `Tmux` command helper on a private socket, `bench`, `out_dir` |
| `examples/tmux_host.rs` | Q2 spawn/capture/liveness/exit/latency, Q5 env, Q6 `pipe-pane` |
| `examples/tmux_reattach.rs` | Q2 reattach from a new process, then `kill-server` |
| `examples/tmux_control.rs` | Q3 control mode parser over pipes |
| `examples/tmux_env.rs` | Q5 the three env routes into a tmux pane |
| `examples/pty_host.rs` | Q4 `portable-pty` spawn/resize/exit/orphans, Q5 env, Q6 tee |
| `target/spike-out/` | Scrollback files and orphan pids written by the runs |
