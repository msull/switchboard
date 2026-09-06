# Spike 03: how Switchboard knows a session's state

Question 3 of Spike 0 in `docs/design.md`: how is *waiting on you* /
*working* / *idle at a prompt* / *exited* known **without reading the
screen, and cheaply**?

Tested 2026-09-05 on macOS 15 (Darwin 24.6), Claude Code 2.1.263, Ghostty
(`/Applications/Ghostty.app`), model `haiku` for every run to keep spend tiny.
tmux 3.7c (Homebrew, installed after the first pass) was tested on a
private server. Everything below is backed by output in this directory.

Files:

| file | what |
|---|---|
| `.claude/settings.json` | project-local hook config registering the logger on 26 events (only read when `claude` starts with this dir as cwd) |
| `settings-abs.json` | same config with absolute paths, for `claude --settings` when running from another cwd |
| `bin/log-hook.sh` | the hook: appends the stdin JSON as one line to `hooks.log` with the event name and a timestamp |
| `bin/drive-interactive.py` | drives a real interactive `claude` in a pty: permission prompt, AskUserQuestion, 60 s idle, `/exit`; timestamps every OSC/BEL it emits |
| `bin/permission-delay.py` | leaves a permission prompt unanswered and times the `Notification` |
| `bin/switchboard-hook` | prototype of the command Switchboard would register (socket first, spool file fallback) |
| `bin/tmux-spike.sh`, `bin/tmux-spike2.sh` | tmux tests on `-L switchboard-spike-state -f /dev/null`; outputs `tmux-spike.out`, `tmux-spike2.out`, `tmux-alerts.log`, `tmux-*.raw` |
| `run1.hooks.log`, `run2.hooks.log`, `run3c.hooks.log` | logs from the `claude -p` runs |
| `interactive.hooks.log`, `interactive.out`, `interactive.raw` | log, driver timeline, raw pty bytes of the interactive run |
| `permission-delay.hooks.log`, `permission-delay.out` | the unanswered-permission run |

Nothing outside this directory was modified. `~/.claude/settings.json` has
no hooks and no permission rules (checked with `jq`), so every event below
came from the spike's own config.

---

## 1. Claude Code hook events

Sources: `claude --help` (only mentions hooks in passing: `--bare` skips
them, `--include-hook-events` streams them in `stream-json`), the docs at
<https://code.claude.com/docs/en/hooks> (the old `docs.claude.com/...` URL
301s there), and `strings` on the 2.1.263 binary, which contains these event
names:

```
PreToolUse PostToolUse PostToolUseFailure PermissionRequest PermissionDenied
UserPromptSubmit Stop StopFailure SubagentStart SubagentStop
SessionStart SessionEnd Setup Notification Elicitation ElicitationResult
PreCompact PostCompact TeammateIdle TaskCompleted CwdChanged FileChanged
ConfigChange InstructionsLoaded WorktreeCreate WorktreeRemove
```

plus (docs only, newer) `PostToolBatch`, `TaskCreated`, `DirectoryAdded`,
`MessageDisplay`, `PreModelSwitch`, `PostModelSwitch`, `UserPromptExpansion`.

`Notification` matcher values in the binary: `permission_prompt`,
`idle_prompt`, `auth_success`, `elicitation_dialog` (docs add
`agent_needs_input`, `agent_completed`, `elicitation_*`, `quota_*`).
`SessionEnd` reasons: `clear`, `logout`, `prompt_input_exit`, `other`
(plus `resume`). `SessionStart` sources: `startup`, `resume`, `clear`,
`compact`, `fork`.

Every payload carries `session_id`, `transcript_path`, `cwd`,
`hook_event_name`; most also `prompt_id`, `permission_mode`, and (new,
undocumented) `scratchpad_dir`. Hooks run as `/bin/sh -c <command>` with
the JSON on stdin, `CLAUDE_PROJECT_DIR` set, default timeout 600 s (short
budget on `SessionEnd`), exit 0 = fine, exit 2 = block where blocking is
possible. A `"async": true` field exists for command hooks. A failing hook
does **not** stop the agent: run A in the notes below had every hook
pointing at a missing file; the agent still answered and printed one
stderr line `SessionEnd hook [...] failed: ... No such file or directory`.

### Which event fires for which situation (measured)

| situation | events, in order | notes |
|---|---|---|
| (d) starts working on a prompt | `UserPromptSubmit` (payload has `prompt`) | fires before the model call. Then `PreToolUse`/`PostToolUse` per tool, each with `tool_name`, `tool_use_id`. |
| (a) needs permission | `PreToolUse` then **`PermissionRequest`** immediately (same second), then **`Notification`/`permission_prompt`** about **6 s later** if still unanswered | `PermissionRequest` carries `tool_name`, `tool_input`, `permission_suggestions`. The Notification carries `message: "Claude needs your permission"` and is what also drives the OSC 777 desktop notification. |
| (b) asks the user a question | `PreToolUse` `AskUserQuestion` then **`PermissionRequest` with `tool_name: "AskUserQuestion"`** and the questions in `tool_input` | A question is surfaced as a permission request on the `AskUserQuestion` tool, so one handler covers (a) and (b). `PostToolUse` `AskUserQuestion` fires once answered (`toolUseResult.answers`). |
| (c) finishes its turn, idle | **`Stop`** (`stop_hook_active:false`, `last_assistant_message` = final text) then, after **60 s** of no input, **`Notification`/`idle_prompt`** (`message: "Claude is waiting for your input"`) | `Stop` is the turn boundary; `idle_prompt` is a later reminder. |
| (e) exits | **`SessionEnd`** with `reason` (`prompt_input_exit` for `/exit`; `other` for `-p` runs ending) | fired for `-p` too. |
| session begins | `SessionStart` with `source: startup` (or `resume`) | first event; gives `session_id` and `transcript_path` up front. |

Also seen: `InstructionsLoaded` (CLAUDE.md), and a spurious `SubagentStop`
with `agent_type: ""` after the AskUserQuestion turn.

Sample payloads (from `interactive.hooks.log`, trimmed):

```json
{"hook_event_name":"PermissionRequest","session_id":"18fc9609-…","cwd":"/Users/sully/code_repos",
 "transcript_path":"/Users/sully/.claude/projects/-Users-sully-code-repos/18fc9609-….jsonl",
 "prompt_id":"dda4a806-…","permission_mode":"default","tool_name":"Bash",
 "tool_input":{"command":"touch …/spike-perm-test.txt","description":"…"},
 "permission_suggestions":[{"type":"addDirectories",…},{"type":"setMode","mode":"acceptEdits",…}]}

{"hook_event_name":"Notification","notification_type":"permission_prompt","message":"Claude needs your permission", …}
{"hook_event_name":"Notification","notification_type":"idle_prompt","message":"Claude is waiting for your input", …}
{"hook_event_name":"Stop","stop_hook_active":false,"last_assistant_message":"Done. Created the file …","background_tasks":[],"session_crons":[], …}
{"hook_event_name":"SessionEnd","reason":"prompt_input_exit", …}
```

## 2. Proof runs

Hook: `bin/log-hook.sh <Event>` appends `{ts, registered, ...payload}` to
`hooks.log`. Registered for all 26 events with no matcher, `timeout: 5`.

### `claude -p "say pong" --max-turns 1 --model haiku` from this directory

3.0 s wall, output `pong`, `run1.hooks.log`:

```
SessionStart(source=startup) -> InstructionsLoaded -> UserPromptSubmit(prompt="say pong")
-> Stop(last_assistant_message="pong") -> SessionEnd(reason=other)
```

A second `-p` run that used Bash (`run2.hooks.log`) added
`PreToolUse(Bash) -> PostToolUse(Bash)` between prompt and Stop. In `-p`
mode `echo hi` ran with **no** `PermissionRequest`; non-interactive mode
cannot prompt, and this version evidently allowed that command rather than
deny it. Not relevant to the board (Switchboard's agents are interactive)
but worth knowing.

Two gotchas found on the way:

- **Trust.** The switchboard folder has `hasTrustDialogAccepted: false` in
  `~/.claude.json`. `-p` runs executed the project-local hooks anyway, but
  interactive `claude` showed the trust dialog and loaded no project hooks
  until it is accepted. Accepting writes to `~/.claude.json`, which this
  spike avoids, so the interactive runs were started from the already
  trusted `/Users/sully/code_repos` with `--settings settings-abs.json`.
  `--settings` hooks load fine (and their command runs relative to the
  session cwd, hence absolute paths).
- **Nested sessions.** Running `claude` from inside a Claude Code session
  inherits `CLAUDECODE`, `CLAUDE_CODE_CHILD_SESSION`, etc. and the child
  says "Transcript saving is off — inherited CLAUDE_CODE_CHILD_SESSION
  marker". The driver scrubs `CLAUDE*` from the child env. Switchboard will
  never be inside a Claude session, but its test harness might be.

### Interactive run (`bin/drive-interactive.py`, `interactive.out`)

Prompt 1 forces a permission prompt (`touch` outside cwd), answered after
~2 s; prompt 2 forces `AskUserQuestion`, answered; then 90 s idle; then
`/exit`. Events with wall-clock offsets from `hooks.log` timestamps:

```
+0   SessionStart startup
+5   UserPromptSubmit
+8   PreToolUse Bash
+8   PermissionRequest Bash            <- waiting on you (approval)
+11  PostToolUse Bash                  <- approved, working again
+13  Stop                              <- idle at prompt
+14  UserPromptSubmit
+17  PreToolUse AskUserQuestion
+17  PermissionRequest AskUserQuestion <- waiting on you (question)
+21  PostToolUse AskUserQuestion
+24  Stop
+27  SubagentStop (agent_type "")
+84  Notification idle_prompt          <- 60 s after Stop
+85  SessionEnd prompt_input_exit      <- exited
```

`bin/permission-delay.py` (`permission-delay.out`) left the permission
prompt unanswered: `PermissionRequest` at +9.0 s, OSC 777 desktop notify at
+14.2 s, `Notification/permission_prompt` hook at +15.2 s. So the
Notification lags the actual prompt by ~6 s; `PermissionRequest` is the
instant signal.

### Mapping events to card states

| card state | enter on | leave on |
|---|---|---|
| **working** | `UserPromptSubmit`; also `PostToolUse` after a permission (approval given) | `Stop`, `PermissionRequest`, `StopFailure`, `SessionEnd` |
| **waiting on you** | `PermissionRequest` (any tool, including `AskUserQuestion`); `Notification/permission_prompt`, `Notification/elicitation_dialog`, `Elicitation` as confirmations | `PostToolUse`/`PostToolUseFailure` for that `tool_use_id`, `PermissionDenied`, `UserPromptSubmit`, `Stop` |
| **idle at a prompt** | `Stop` (with `last_assistant_message` as the caption); `Notification/idle_prompt` re-confirms; `SessionStart` before the first prompt | `UserPromptSubmit` |
| **exited** | `SessionEnd` (reason), or the process is gone (see fallbacks) | next `SessionStart` with the same session id (`--resume`) |
| **not running** | record exists, no live process | `SessionStart` |

`StopFailure` (rate limit, auth, server error) should render as *waiting on
you* too: the turn ended without the agent finishing. Compaction events and
`SubagentStart/Stop` are noise for the board and should not be registered.

## 3. Transport: how the hook reaches the app

Measured on this machine (`time`, cold): a Unix-socket connect with nobody
listening fails in **7 ms** (`nc -U` exit 1); `curl` to a closed localhost
port fails in **9 ms** (exit 7); appending to a file is ~0.1 ms per line.
With a listener, `nc -U` delivers in 14 ms. The `switchboard-hook`
prototype (jq + date + nc) costs ~20 ms warm, ~300 ms on first cold run.

| option | app not running | correlation | cost per event | verdict |
|---|---|---|---|---|
| (a) append to a file the app watches | free, nothing can fail; events queue up and are replayed on start (good: the app learns a session exited while it was down) | `session_id` + `cwd` in each line | ~0 | needed as the fallback regardless |
| (b) `switchboard-hook` CLI -> Unix socket | must detect "no listener" fast and not fail the agent; 7 ms, exit 0 | same, plus the app can trust the sender is local | one process spawn (the shell already spawns one for the hook) | best live path: push, no polling, no partial-line parsing |
| (c) HTTP to localhost | same as (b) with a port to pick and `curl` in the loop; anything on the machine can post to it | same | slightly more (TCP + HTTP parse) | no advantage over (b) on one machine |

One important observation for (b) and (c): Claude Code runs hooks
**synchronously** and waits for exit. The hook must therefore be fire and
forget: connect, write one line, exit. Never wait for a reply. The
prototype `bin/switchboard-hook` does socket first, then spool file, always
exits 0, and was tested both ways (output in the session log; app received
the line over the socket in 19 ms; without a listener the line landed in
the spool file). It also drops all the big fields (`tool_input`,
`tool_result`) so a line stays small.

Correlation: every payload has `session_id`, `cwd`, and `transcript_path`.
Switchboard knows the `cwd` it launched the session in and, after the first
`SessionStart` arriving from that `cwd` while a launch is pending, records
the `session_id` as the resume handle (this is also the answer to Spike 0
question 1's "discover the id after the fact"). From then on `session_id`
alone is the key. `$PPID` inside the hook is the `sh` that Claude Code
spawned, not the `claude` pid, so pid-based correlation is not worth it.

## 4. Fallbacks without hooks

### What Claude Code itself emits to the terminal (`interactive.raw`)

Even with no hooks, the TUI writes standard escape sequences an emulator or
tmux can see. Timeline from `interactive.out` (offsets from pty start):

```
0.7s  OSC 0 title "✳ Claude Code"     BEL   OSC 9;4;0 (progress off)   -> idle
5.9s  OSC 0 title "◐ Claude Code"     BEL   OSC 9;4;3 (progress on)    -> working
6.7s  OSC 0 title "◐ Create spike-perm-test.txt"  (title becomes the ai-title of the session)
8.4s  OSC 0 title "✳ Create spike-perm-test.txt"  OSC 9;4;0             -> stopped (permission prompt up)
…
14.2s OSC 777;notify;Claude Code;Claude needs your permission             -> waiting on you (only if unattended ~6 s)
84.2s OSC 777;notify;Claude Code;Claude is waiting for your input         -> idle 60 s
85.5s OSC 0 title ""                                                       -> exit
```

Rules that fall out of this, all cheap, all in the output stream:

- **Title glyph**: `◐`/`◑`/`◒`/`◓` spinner = working; `✳` = not working
  (idle *or* waiting); empty = exited. tmux exposes this as `#{pane_title}`
  with no parsing of screen content at all.
- **OSC 9;4;3 / 9;4;0** (ConEmu progress) toggles with working/stopped and
  is emitted independently of the title.
- **OSC 777 notify** text distinguishes "needs your permission" from
  "waiting for your input". It is only sent after the prompt has sat there
  (~6 s for permission, 60 s for idle), so it is a confirmation, not the
  first signal.
- **BEL**: Claude Code sends a BEL with every title update (35 BELs in a
  90 s session), so the bell by itself means "something changed", not
  "needs you". Useful for other agents that only ring on completion, and
  for `monitor-bell`; useless as a "waiting" detector for Claude Code.

### OSC 133 (shell integration) for plain shells

Ghostty's shell integration script sourced into a bare `bash -i` in a pty
emitted, for `echo hi; false` then `exit`:

```
ESC]133;A;aid=86428 BEL   prompt start
ESC]133;B BEL             prompt end / command start
ESC]133;C; BEL            command output starts   -> "working"
ESC]133;D;1;aid=86428 BEL command done, exit 1    -> "idle at a prompt", last exit code
ESC]133;A;aid=86428 BEL   next prompt
```

plus `OSC 7` with the cwd. So a shell session's state is exact: between
`C` and `D` = working, after `D`/`A` = idle with the exit status of the
last command available for the caption. Ghostty injects this automatically
(`shell-integration = detect`; scripts in
`/Applications/Ghostty.app/Contents/Resources/ghostty/shell-integration/`);
tmux 3.4+ passes 133 through and can be asked about `#{pane_...}` only via
the title, so if Switchboard owns the PTY it parses 133 itself; if tmux is
the host, a `pipe-pane` or control-mode `%output` stream gives the bytes.

### Idle-output timer

Last resort for anything else (an agent that emits neither hooks nor
OSC): no output for N seconds after a burst = "stopped, probably idle or
waiting"; the two cannot be told apart. Keep N around 5 s and label the
state "quiet" rather than claim "waiting on you".

### tmux signals (measured, tmux 3.7c, private server `-L switchboard-spike-state -f /dev/null`)

Script: `bin/tmux-spike.sh` and `bin/tmux-spike2.sh`, output in
`tmux-spike.out` / `tmux-spike2.out`. No keystrokes were sent to any
window; every pane was started with its command line, and Claude Code got
its prompt as a positional argument. The server was killed at the end.

**Exit status.** With `remain-on-exit on`, a window running `sleep 1; exit 3`
reads `dead=1 status=3` from `#{pane_dead} #{pane_dead_status}` 1.8 s in,
and the `pane-died` hook fired (`pane-died @1 %1` in `tmux-alerts.log`).
`#{pane_current_command}` keeps showing the last foreground command
(`sleep`) after death, and for Claude Code it shows **`2.1.263`** (the
versioned binary name behind the `claude` symlink), so match on
`#{pane_pid}` + `ps`, not on the command name.

**Alert flags and hooks.** With `monitor-bell on`, `monitor-activity on`,
`monitor-silence 3`, a non-current window that printed `\a` then `more`
then went quiet showed `bell=1 act=1 sil=1` (`#{window_bell_flag}`,
`#{window_activity_flag}`, `#{window_silence_flag}`), and the global hooks
`alert-bell`, `alert-activity`, `alert-silence` each ran a `run-shell`
that logged `#{hook_window}` (`alert-bell @2`, `alert-silence @2` 4 s
later). Flags stick until the window is viewed, so they are edge signals;
a `run-shell 'switchboard-hook tmux-bell ...'` in those hooks is the push
path. Note the flags only set for windows that are not current in an
attached client, which is fine for a headless host server.

**OSC passthrough.** A pane that printed OSC 0, 777, 133, 7 and a
`\ePtmux;` wrapped 777:

| sequence | tmux state | `capture-pane -e` (grid) | `pipe-pane` raw |
|---|---|---|---|
| OSC 0 / 2 title | **`#{pane_title}` = `TITLE-FROM-OSC0`** | no | yes |
| OSC 777 notify | dropped | no | yes |
| OSC 133 A/B/C/D | dropped (not stored) | no (only SGR survives) | yes, intact |
| OSC 7 cwd | dropped (tmux uses `pane_current_path` from the process) | no | yes |
| `\ePtmux;...` wrapped | forwarded to the outer terminal (`allow-passthrough on`) | no | yes |

So tmux **state** exposes exactly one of these: the title. The rest are
only available from the raw byte stream (`pipe-pane -O 'cat >> file'`, or
`%output` lines in control mode, which showed `%output %1 bye\015\012\007`,
the BEL included as `\007`).

**Ghostty shell integration inside a pane.** `bash --norc -i` with
`ghostty.bash` sourced (commands on stdin) emitted, through tmux, the full
`OSC 7 … 133;A … 133;B … OSC 2 <cwd> … 133;C … 133;D;1 … 133;A` sequence
in `pipe-pane` output, and because the integration also sets the title
(OSC 2) to the running command, **`#{pane_title}` read `sleep 20` while the
command ran and the cwd (`~/code_repos/...`) at the prompt**. For shells
under tmux that is an idle/working signal from tmux state alone, with the
exit code available from `133;D;<n>` in the pipe.

**Claude Code inside a pane** (`claude --model haiku --settings … '<prompt
that needs permission>'`, hooks via `--settings`; 24 s of 1 s samples, then
a second run with `default-terminal xterm-ghostty`):

```
[ 1s] cmd=2.1.263 dead=0 bell=0 act=1 sil=0  title=✳ Claude Code
[ 2s] cmd=2.1.263 dead=0 bell=0 act=1 sil=0  title=✳ spike-perm-test.txt
[ 6s] ...                    act=1 sil=1  title=✳ spike-perm-test.txt   (silence 3 s tripped)
hooks: SessionStart, UserPromptSubmit, PreToolUse Bash, PermissionRequest Bash, Notification permission_prompt
pipe-pane raw: OSC 0 "✳ Claude Code", OSC 0 "✳ spike-perm-test.txt",
               OSC 777 "Claude needs your permission"; 3 BEL; no OSC 9;4
kill-window -> SessionEnd reason=other
```

Findings:

- Hooks work unchanged inside tmux, and `kill-window` produces a clean
  `SessionEnd`.
- With the prompt passed as argv, Claude Code emitted **no spinner title
  and no OSC 9;4 progress** under either `TERM=tmux-256color` or
  `TERM=xterm-ghostty` (both runs), unlike the pty run where a typed prompt
  produced `◐/◑` titles and `9;4;3`. Whether a typed prompt inside tmux
  animates the title was not tested (no keystrokes allowed). Treat the
  spinner as a bonus, not a dependency.
- **Hook-free "waiting on you" from tmux state alone: no.** What tmux
  knows is `title=✳ …` (not working) plus `silence=1` after 3 s, which is
  the same for *idle at a prompt* and *waiting for approval*. The only
  hook-free discriminator is the OSC 777 text `Claude needs your
  permission`, which tmux discards from state but hands over verbatim via
  `pipe-pane` (or `%output` in control mode). `capture-pane` does show the
  literal `Do you want to proceed?` line, but that is screen reading, which
  this spike excludes.
- Exit: `pane_dead`/`pane_dead_status` or the `pane-died` hook. A
  `%pane-died` control-mode notification did not appear within the 4 s
  the client was held open, so poll `pane_dead` or use the hook.

## 5. Transcript watching (hook-free option for Claude Code)

Transcripts land in
`~/.claude/projects/<cwd with / and _ replaced by ->/<session_id>.jsonl`,
one JSON object per line. The `-p` run's transcript (11 lines) and the
interactive one (42 lines) show the same shape. Ignoring bookkeeping types
(`queue-operation`, `attachment`, `atis-latch`, `file-history-snapshot`,
`mode`, `permission-mode`, `ai-title`, `last-prompt`, `cost-state`), the
conversation lines are:

```
user      (message.content = string)                  prompt submitted   -> working
assistant content [thinking] / [text], stop_reason end_turn             -> idle (turn over)
assistant content [tool_use], stop_reason tool_use    tool requested     -> working, OR waiting if
user      content [tool_result] (+ toolUseResult)     tool finished         no tool_result follows
system    subtype stop_hook_summary / turn_duration   turn ended         -> idle (reliable marker)
```

Inference rule on the tail: last conversation line is `assistant` with
`stop_reason: "end_turn"` (or a `system turn_duration`) = idle at prompt;
last is `user` string = working; last is `assistant tool_use` with no
matching `tool_result` yet = executing a tool **or** sitting at a
permission/question prompt. That last ambiguity is the problem: while the
permission prompt was up (3 s in the run, could be hours), the transcript
did not change at all; the `tool_use` line is written before the prompt and
the `tool_result` line only after approval. The transcript therefore
detects idle and working well but cannot tell *waiting on you* from a
long-running tool without a timer, and `AskUserQuestion` is only
recognisable by the tool name in the `tool_use` block. Exit is not
recorded at all (no line on `/exit`).

Cost: the file is appended, so a `kqueue`/FSEvents watch plus reading the
last few KB is cheap. Session discovery is a bonus: a new `<uuid>.jsonl`
appearing in the project's directory is the session id for the resume
handle, even with no hooks.

## Signal per card state, per session kind

| card state | Claude Code agent | other agent (no hooks) | service / command | shell |
|---|---|---|---|---|
| working | `UserPromptSubmit`, `PostToolUse` after approval; fallback: title spinner `◐` / OSC 9;4;3 when typed interactively (not seen with an argv prompt under tmux), transcript tail `user`/`tool_use` | output flowing (`window_activity_flag`); OSC 133 `C` if under a shell wrapper; title/progress if it emits them | process alive; for commands OSC 133 `C` seen, or tmux `pane_title` = command name via Ghostty integration | OSC 133 `C`; under tmux, `pane_title` = running command |
| waiting on you | `PermissionRequest` (incl. `AskUserQuestion`), confirmed by `Notification/permission_prompt` +6 s; fallback: OSC 777 "needs your permission" in the raw stream (`pipe-pane`/`%output`); **not derivable from tmux state** (`✳` title + silence is the same as idle) | BEL after a burst + silence (`window_bell_flag` + `window_silence_flag`, heuristic), OSC 777 if it sends one | n/a (a service that stops is *exited*) | n/a |
| idle at a prompt | `Stop` (caption = `last_assistant_message`), `Notification/idle_prompt`; fallback: transcript `end_turn`/`turn_duration`, OSC 777 "waiting for your input"; tmux: `pane_title` starts with `✳` + `window_silence_flag` (= idle *or* waiting) | silence > N s with no bell (`window_silence_flag`); OSC 133 `A` if a shell prompt is showing | n/a | OSC 133 `A`/`D` (exit code in `D;<n>`); under tmux, `pane_title` = cwd |
| exited | `SessionEnd` (reason; `other` on `kill-window`); fallback: child process gone, `pane_dead=1` / `pane-died` hook, title cleared | process gone / `pane_dead_status` | process gone, exit code from waitpid or `pane_dead_status` (`remain-on-exit on`) | shell process gone / `pane_dead` |
| not running | no live process for the record | same | same | same |

## Recommendation

**Primary mechanism for Claude Code: hooks, pushed to a Unix socket, spooled
to a file when the app is down.**

1. Switchboard ships one small binary, `switchboard-hook` (a Rust bin in
   this workspace; the shell prototype is `bin/switchboard-hook`). It reads
   stdin, keeps `session_id`, `cwd`, `transcript_path`, `hook_event_name`,
   `notification_type`, `tool_name`, `tool_use_id`, `reason`, `source`,
   `permission_mode`, the first 200 chars of `last_assistant_message`,
   adds a timestamp, writes one line to
   `~/Library/Application Support/switchboard/hook.sock` with a 100 ms
   connect timeout, and on any failure appends the line to
   `hook-spool.jsonl` beside it. It always exits 0 and never reads a reply.
   No jq, no nc, no shell: one exec, sub-millisecond.
2. Registered events, all without matchers (`"async": true` is not needed
   because the command returns immediately):
   `SessionStart`, `UserPromptSubmit`, `PermissionRequest`, `PostToolUse`,
   `PostToolUseFailure`, `PermissionDenied`, `Stop`, `StopFailure`,
   `Notification`, `SessionEnd`. Not `PreToolUse`/`PostToolUse` per tool
   beyond what is needed to clear a pending permission (keep `PostToolUse`,
   drop `PreToolUse`), not compaction, not subagents.
3. Where the config lives: Switchboard writes the hooks into the
   **project-local** `<root>/.claude/settings.local.json` of each project it
   manages (merging, never overwriting other keys), so it does not touch
   the user's global settings and the hooks only exist where Switchboard
   launches agents. `settings.local.json` is gitignored by Claude Code's
   convention. Alternative if a project should stay untouched: launch with
   `claude --settings <switchboard-managed file>`; this works (shown above)
   and needs no file in the repo, at the cost of an extra flag in the
   resume command. The snippet Switchboard generates:

   ```json
   {
     "hooks": {
       "SessionStart":      [{"hooks":[{"type":"command","command":"switchboard-hook SessionStart","timeout":5}]}],
       "UserPromptSubmit":  [{"hooks":[{"type":"command","command":"switchboard-hook UserPromptSubmit","timeout":5}]}],
       "PermissionRequest": [{"hooks":[{"type":"command","command":"switchboard-hook PermissionRequest","timeout":5}]}],
       "PostToolUse":       [{"hooks":[{"type":"command","command":"switchboard-hook PostToolUse","timeout":5}]}],
       "PostToolUseFailure":[{"hooks":[{"type":"command","command":"switchboard-hook PostToolUseFailure","timeout":5}]}],
       "PermissionDenied":  [{"hooks":[{"type":"command","command":"switchboard-hook PermissionDenied","timeout":5}]}],
       "Stop":              [{"hooks":[{"type":"command","command":"switchboard-hook Stop","timeout":5}]}],
       "StopFailure":       [{"hooks":[{"type":"command","command":"switchboard-hook StopFailure","timeout":5}]}],
       "Notification":      [{"hooks":[{"type":"command","command":"switchboard-hook Notification","timeout":5}]}],
       "SessionEnd":        [{"hooks":[{"type":"command","command":"switchboard-hook SessionEnd","timeout":5}]}]
     }
   }
   ```

   (`switchboard-hook` must be on `PATH` or written as an absolute path
   into the app bundle; absolute is safer. Note that a `PermissionRequest`
   hook that prints nothing leaves the decision to the user, which is what
   we want.)
4. In the app: a `HookListener` adapter owns the socket (a `ProcessHost`
   sibling in `ports`), turns each line into a `SessionEvent {session_id,
   cwd, kind, at}` and feeds `core`, whose state machine is the table in
   section 2. On start the app drains `hook-spool.jsonl` first (so a
   session that ended while the app was closed is shown as *exited*), then
   deletes it. Correlation: pending launches are keyed by `cwd`; the first
   `SessionStart` from that cwd binds `session_id` to the record and stores
   it as the resume handle; every later event is keyed by `session_id`.
5. Degradation, in order, all implemented as the same `SessionEvent`
   stream so `core` does not care where a signal came from:
   - **Hooks absent** (agent launched outside Switchboard, or a user who
     removed them): watch the transcript `<project dir>/<id>.jsonl` with
     FSEvents; tail rule from section 5 gives *working*/*idle* exactly and
     *waiting* as "tool_use with no result for > 10 s".
   - **Terminal stream available** (Switchboard owns the PTY, or tmux
     `pipe-pane`/control-mode `%output`): parse OSC 0 title glyph, OSC 9;4,
     OSC 777 text, and OSC 133 for shells. tmux's own state only carries
     the title, `pane_dead(_status)`, and the bell/activity/silence flags;
     that is enough for shells and services but not to tell an agent's
     *waiting* from *idle*, so with tmux as host always attach a
     `pipe-pane` per agent pane. This alone gives every state for Claude Code except
     the exact question text, and gives exact *idle/working/exit code* for
     shells and commands.
   - **Nothing but bytes**: BEL after output then silence > 5 s = "may
     need you" (soft highlight), silence alone = "quiet", process gone =
     *exited* with `waitpid` status.
   - Process liveness (`kill -0` / tmux `pane_dead`) always overrides:
     no process means *exited* or *not running* whatever the last event
     said.

Cost summary: hooks add one short process per event (about 10 events in a
typical turn), the socket write is microseconds, and the app does no
polling for Claude Code sessions at all. The transcript and PTY fallbacks
are event-driven too (FSEvents, pty reads the app already does). Only the
idle-output timer is a timer, and it only runs for sessions that emit
nothing else.
