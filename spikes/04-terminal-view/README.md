# Spike 04: how is a terminal shown for a session?

Question 4 of "Spike 0" in `docs/design.md`: render the terminal inside the
egui app, or hand off to an external terminal (Ghostty, Cmux) with a
jump-to button? Answered by building and running, 2026-09-05, on macOS
(Darwin 24.6, Apple Silicon), Rust 1.98.1, egui/eframe 0.36.1, Ghostty
1.2.3, cmux 0.61.0.

**Short answer.** Both work. `egui_term` (git master, vendored here) compiles
against egui 0.36 without a single source change and renders `top`, `vim`
and Claude Code's TUI correctly at 0.8 to 2.7 ms per frame with 0% idle CPU.
Ghostty can be handed a command, a directory and a window title from the
command line and that window can be raised by title later. Cmux has the
richest control surface (workspaces, status, Claude hooks) but its socket
refuses processes not started inside cmux unless the user changes a setting.
Recommendation at the end: embed for the board and for commands/services,
hand off to Ghostty for long agent sessions, treat Cmux as an optional
integration.

## Incident during this spike

An early attempt to test keyboard input used `osascript` / System Events to
type into the spike window. The spike had already exited (an empty
`SPIKE_CMD` ran `zsh -lc ""`), so the keystrokes went to whichever window was
frontmost: the user's own Claude Code session in Ghostty. It received
`echo first-command`, an Enter, arrow keys, `sleep 30`, a Ctrl-C (which
interrupted a running tool there) and `echo pasted-from-clipboard`.

Every result from synthetic OS-level keystrokes is therefore discarded and
the helper was deleted. Keyboard behaviour below was verified instead with
`egui_kittest`, which delivers events to the widget in-process and cannot
reach any other window (`tests/keys.rs`). Window focus experiments only
activate applications or raise windows; nothing types into anything.

## Layout

| Path | What |
|---|---|
| `Cargo.toml` | Standalone crate (empty `[workspace]` so it never joins the repo root), edition 2024 |
| `src/lib.rs` | `SpikeApp`: eframe app with one `egui_term` terminal, a status bar (fps, frame ms), `screen_text()` for tests |
| `src/main.rs` | Window; `SPIKE_CMD="top"` runs a command instead of an interactive shell |
| `tests/keys.rs` | Headless keyboard test (text, Enter, arrows, Ctrl-C, paste) and an `#[ignore]` test that runs `claude` |
| `vendor/egui_term/` | `Harzu/egui_term` at `31bbc7ab` (2026-07-30), only `Cargo.toml` rewritten (egui 0.35 to 0.36) |
| `shots/` | Screenshots, cropped to the window |

```sh
cargo run                                  # interactive $SHELL in an egui window
SPIKE_CMD="vim src/lib.rs" cargo run       # a TUI instead of a prompt
cargo test                                 # headless keyboard test, ~0.5 s
cargo test -- --ignored claude_tui --nocapture   # runs claude with a one-word prompt (costs tokens)
```

## 1. Crate survey

Fetched from the crates.io API and GitHub on 2026-09-05.

| Crate | Latest release | Released | License | egui | State |
|---|---|---|---|---|---|
| `egui_term` (also published as `egui-term`) | 0.1.0 | 2025-04-24 | MIT | release: old; **git master: egui 0.35, alacritty_terminal 0.26** | Maintained: last push 2026-08-31, 54 commits, 72 stars, dependabot bumps egui within days of release. Features: pty rendering, multiple instances, keyboard, custom bindings, resize, scrolling, focus, selection, fonts/themes, hyperlinks. **Compiles unchanged against egui 0.36.** |
| `egui_terminal` | 0.1.0 | 2023-07-29 | GPL-3.0-or-later | ancient | Dead; no repository field. License also rules it out. |
| `alacritty_terminal` | 0.26.0 | 2026-04-06 | Apache-2.0 | n/a | The Alacritty core: `Term` grid, `vte` parser, pty spawning (`tty::new`), event loop. Actively released. What `egui_term` builds on. |
| `vte` | 0.15.0 | 2025-02-02 | Apache-2.0 OR MIT | n/a | Escape-sequence parser only; you would still write the grid. Pulled in by `alacritty_terminal`. |
| `termwiz` | 0.23.3 | 2025-03-20 | MIT | n/a | WezTerm's terminal toolkit; has a `Surface`/`Terminal` model and an escape parser but no egui widget. Heavier, more Windows-oriented. |
| `portable-pty` | 0.9.0 | 2025-02-11 | MIT | n/a | Cross-platform pty spawn only (WezTerm). Not needed: `alacritty_terminal::tty` already spawns the pty. |

`ratatui-ghostty` 0.2.0 and `gpui-libghostty` 0.2.1 (2026-09-02) also exist;
see section 5.

Best candidate: `egui_term` from git. The crates.io 0.1.0 release is not
usable (old egui), and the crate uses `egui = { workspace = true }` pinned to
0.35 on master, which would drag a second egui into the build. Vendoring
the 2 000 lines and rewriting its `Cargo.toml` to `egui = "0.36"` was the
whole port; `cargo build` succeeded first time with no warnings.

## 2. What was built

`src/lib.rs` (about 110 lines): `TerminalBackend::new(id, ctx, sender,
BackendSettings { shell, args, working_directory })` spawns `$SHELL -l` in a
pty; `TerminalView::new(ui, &mut backend).set_focus(true).set_size(..)` draws
it. The backend thread calls `ctx.request_repaint()` when the pty produces
output, so the app repaints only when something changed. A top panel shows
frames in the last second and the previous frame's wall time.

Things learned about `egui_term` that matter for real use:

- **Keyboard input is dropped unless the mouse pointer is over the widget**
  (`view.rs` `process_input`: `if !layout.has_focus() || !layout.contains_pointer() { return }`).
  With the pointer elsewhere the terminal looks focused but ignores typing.
  A one-line patch for Switchboard; noted, not applied here.
- It never calls `alacritty_terminal::tty::setup_env`, so `TERM` and
  `COLORTERM` are inherited from the parent process. Launched from Ghostty
  the shell saw `TERM=xterm-ghostty`; the spike's launcher sets
  `TERM=xterm-256color`. Switchboard should set both explicitly.
- Scrollback size, selection and hyperlinks come from `alacritty_terminal`
  and work; Cmd-C/Cmd-V are bound on macOS.
- A `PtyEvent::Exit` arrives on the channel when the child exits.

## 3. Measurements

Debug build with `opt-level = 2` for dependencies (the repo's dev profile).
Window 1000x640 logical, 125 columns x 38 rows, retina. CPU from
`top -l 10 -s 1 -pid` (per-second samples of the spike process), frame time
from the app's own status bar.

| Test | Renders correctly? | Frame time | Repaints/s | CPU (spike process) | RSS |
|---|---|---|---|---|---|
| `ls --color -la ~` | Yes: 16 colours, bold, block cursor (`shots/ls-color-d1.png`) | n/a | 6 | 0 % after output | 74 MB |
| `top -o cpu` (1 s refresh) | Yes: full-screen redraw, reverse-video header (`shots/top-d1.png`) | 2.7 ms | 56 while `top` streams its redraw, 3 between | `top` itself reported the spike at 14 % during redraws | 280 MB |
| `vim` with `syntax on`, `set number` | Yes: alternate screen, syntax colours, status line (`shots/vim-d1.png`) | 0.8 ms | 6-7 | 0.0 % idle in vim | 77 MB |
| Window resized 1000x640 to 700x420 while in vim | Yes: grid shrank, vim reflowed with line numbers intact (`shots/vim-after-resize-d1.png`) | | | | |
| `cat` of a 25 MB file (300 000 numbered lines; through an awk quoting slip the file was one 25 MB line, wrapped, which is the harder case) | Yes; `cat` wall time **1.47 s** (~17 MB/s into the grid) (`shots/cat-scrolling-d1.png`) | | 17 during the burst | **95 % for one 1-s sample**, then 0.0 % | 335 MB during, 141 MB after |
| Idle at prompt, nothing happening | | | 0-1 | **0.0 %** over 8 samples | 74 MB |
| `claude` (v2.1.263) interactive, prompt "Reply with exactly the single word pong" | Yes: logo, prompt box, status line, `⏺ pong` reply, redraw after the reply (`tests/keys.rs::claude_tui` prints the grid) | | | | |

Keyboard, from `cargo test` (`tests/keys.rs`, in-process events, real
`zsh -f` in the pty):

- Typed text plus Enter: echoed back in **4.2 to 5.2 ms** end to end
  (event queued, written to pty, shell echo read, grid updated, next frame).
- Arrow keys: `eco AB` + 4 x Left + `h` edited to `echo AB` (zle received
  `ESC[D`).
- Ctrl-C: `sleep 30 && echo NOT-PRINTED` interrupted; `^C` shown, prompt
  back, nothing printed.
- Paste (`egui::Event::Paste`): written to the pty verbatim.

The whole test takes 0.44 s including shell startup.

Honest caveats: a 125x38 grid at 56 repaints/s is a debug build on an M-series
laptop; a 200x60 grid would cost more per frame because `egui_term` lays out
every cell as a galley each frame. `top` reporting 14 % during redraws means
one always-busy TUI would cost a noticeable but not alarming amount of CPU
inside Switchboard. The 25 MB `cat` shows the grid keeps up with bulk output
(Alacritty's parser is fast); the 300 MB peak is scrollback growth.

## 4. Hand-off to an external terminal

### Ghostty 1.2.3 (installed at `/Applications/Ghostty.app`)

`ghostty --help` is explicit: "On macOS, launching the terminal emulator from
the CLI is not supported and only actions are supported. Use `open -na
Ghostty.app` instead." `ghostty +new-window --help`: "Only supported on GTK."
So on macOS everything goes through `open`.

What worked (exact commands):

```sh
# New Ghostty instance, one window, given directory, title, and command.
open -na Ghostty --args \
  --window-save-state=never \
  --quit-after-last-window-closed=true \
  --title=switchboard-agent-a \
  --working-directory=/Users/sully/code_repos/personal/switchboard \
  -e sh -c 'echo "cwd=$(pwd)"; exec claude --resume <id>'
```

Verified: a window titled `switchboard-agent-a` opened, printed
`cwd=/Users/sully/code_repos/personal/switchboard`, and ran the command
(`shots/ghostty-handoff-d1.png`). Every `--key=value` config option is
accepted as an argument; `-e` must come last.

Caveats found by running it:

- `-n` starts a **second Ghostty process**. Without `--window-save-state=never`
  the new process restored the previous session's windows too, and `--title`
  renamed all of them, since `title` is an instance-wide config, not a
  per-window one. With the two flags above it opens exactly one window.
- `open -a Ghostty --args ...` (no `-n`) with Ghostty already running
  **ignores the arguments**: it just activates the existing app. Tested;
  no window appeared. So each hand-off is its own process, which is fine
  (the process count is the window count) but means Ghostty's tabs cannot
  be used from outside.
- `--quit-after-last-window-closed=true` makes that instance exit when its
  window closes, so instances do not accumulate.

Focusing an existing window later, by title, with System Events (this is
window activation only, no input):

```applescript
tell application "System Events"
  repeat with p in (every process whose name is "Ghostty")
    repeat with w in windows of p
      if title of w is "switchboard-agent-a" then
        set frontmost of p to true
        perform action "AXRaise" of w
        return "raised in pid " & (unix id of p)
      end if
    end repeat
  end repeat
  return "not found"
end tell
```

Verified: with Finder frontmost, this raised the Ghostty window and
`first process whose frontmost is true` then reported that pid and title.
Requires the Accessibility permission for the caller (already granted to the
terminal here). Since `--title` is forced for the whole instance, the title is
a reliable key as long as Switchboard picks unique ones. The instance pid
returned by `open`/`pgrep` is an alternative key.

Ghostty 1.2.3 has no AppleScript dictionary (`sdef` fails, `tell application
"Ghostty" to get windows` errors with -1728). **Ghostty 1.3 adds one**
(PR #11208, merged 2026-03-07, documented at ghostty.org/docs/features/applescript
as a preview feature): `new surface configuration` with `initial working
directory`, `new window with configuration cfg`, `new tab in win`, `terminals`,
`every terminal whose working directory contains ...`, `focus t`, `input text
... to t`, `send key "enter" to t`. That gives tabs in one instance, focus by
object, and an id per terminal. Not testable on this machine (1.2.3), so the
recommendation below does not depend on it; it is the upgrade path.

### Cmux 0.61.0 (installed, `/opt/homebrew/bin/cmux` -> app bundle CLI)

Built on libghostty (GPL-3.0-or-later). The CLI talks to `/tmp/cmux.sock`
and has, from `cmux --help`: `new-workspace [--command <text>]`,
`select-workspace`, `rename-workspace`, `focus-window`, `list-workspaces`,
`current-workspace`, `read-screen`, `notify`, `set-status <key> <value>
--icon --color`, `set-progress`, `log`, `claude-hook <session-start|stop|notification>`,
plus a tmux-compatibility set (`capture-pane`, `respawn-pane`, `wait-for`,
`pipe-pane`) and a browser automation set. There is no cwd flag; the
recipe would be `--command 'cd <dir> && exec claude --resume <id>'`.
`Resources/bin/claude` is a wrapper that injects `--session-id` and hook
settings so Claude Code's hooks report status back into cmux; `cmux
restore-session` and `surface resume` exist (the README describes agent
resume integrations), so Cmux 0.61 has grown some of the persistence the
design doc says it lacked. Worth re-reading that premise.

What happened when tested: with the app running and the socket present,
every CLI command died with SIGPIPE, and a raw connection answered:

```
ERROR: Access denied — only processes started inside cmux can connect
```

The app's strings show the modes: default ancestry check (only descendants
of cmux), a password mode ("Require socket authentication with a password
stored in your keychain", `CMUX_SOCKET_PASSWORD`), and an open mode that
"disables ancestry and password checks and opens the socket to all local
users. Only enable when you understand the risk." Changing the user's cmux
settings is outside this spike, so **workspace creation and focus from
Switchboard are untested**; they are plausible once the user enables
password mode. No URL scheme is registered (`Info.plist` has no
`CFBundleURLTypes`) and there is no AppleScript dictionary, but System
Events can raise its window like any app. `cmux --help` run as the app binary
by mistake launches the GUI; the CLI is the one under `Resources/bin`.

## 5. libghostty from Rust

From docs only. ghostty.org/docs/about: libghostty is "a cross-platform,
C-ABI compatible library" that powers both GUI apps, but "as of the initial
public release, libghostty is not yet a stable API and has not been released
as a standalone, stable library"; the stated goal is to stabilise and release
it later. The C header is `include/ghostty.h` (38 KB) in the repo; the macOS
app consumes it as `GhosttyKit.xcframework` built by `zig build`.

Usable from Rust today, with effort: `gpui-libghostty` 0.2.1 (2026-09-02,
alpha) does exactly this for GPUI. Its README shows the cost: Ghostty pinned
to one commit and vendored, **Zig 0.16 and Xcode command-line tools required
at build time**, first build compiles Ghostty, a shared native cache to make
later builds bearable, and the terminal is a native Metal layer hosted inside
the GPUI window with events forwarded to it. For egui/eframe there is no
equivalent; it would mean writing that host layer (an `NSView` under the
winit window, keyboard/mouse forwarding, resize sync) with `objc2`, which the
repo's `unsafe_code = "forbid"` rule pushes into a wrapper crate. `ratatui-ghostty`
uses `ghostty-vt` (the VT core only, no renderer) and is not an egui path.
Verdict: not for milestone 1; reconsider when libghostty ships a versioned
release with a documented C API.

## Comparison

| | Embedded `egui_term` | Hand-off to Ghostty | Hand-off to Cmux |
|---|---|---|---|
| TUI fidelity (`vim`, `top`, Claude Code) | Correct in every test; colours, alt screen, resize, cursor. Font rendering is egui's (no ligatures, plain block cursor, no scrollbar). | Native, best in class; the user's own config, fonts, themes, scrollback search. | Same renderer as Ghostty. |
| Input to echo | 4-5 ms measured. Keyboard only when pointer over the widget until patched. | Native. | Native. |
| Cost when busy | 2.7 ms/frame at 125x38, ~14 % CPU while `top` redraws; 0 % idle. Grows with grid size and number of visible terminals. | None inside Switchboard. | None inside Switchboard. |
| Effort | Done: vendored crate + ~110 lines. Remaining: focus patch, `TERM`, theme, font size, scrollbar, a `Terminal` adapter. | ~20 lines: one `open -na` command and one System Events raise. Accessibility permission for raise. | Untested: user must enable socket password mode; then `new-workspace --command`, `select-workspace`, `set-status`, `claude-hook` are all there. |
| Persistence interplay | Switchboard owns the pty, so the process dies with the app unless tmux (spike 02) is the host; the terminal then attaches to tmux. Scrollback is in hand. | Process lives in Ghostty, survives Switchboard restarts, dies with Ghostty or reboot. Switchboard cannot read its screen or scrollback; state comes only from hooks/transcripts (spike 03). Resume is just relaunching the command. | Cmux also keeps a session model and Claude hooks; two apps would then each believe they own the workspace record. |
| Fit with the card board | Best: a card can open in place, split view, "last two lines" caption from the live grid, waiting-state and terminal in one window. | The board is a launcher; a card is a button that raises a window elsewhere. Captions and last output need another source. | Same as Ghostty, plus `read-screen` for captions if the socket is open. |
| License | MIT + Apache-2.0 | n/a (separate app) | GPL-3.0 app; CLI use is fine. |

## Recommendation

**Agent sessions: hand off to Ghostty for the terminal, and keep the
embedded widget as the in-app view.** Concretely, in milestone 1 the
"return to it" action on an agent card runs:

```sh
open -na Ghostty --args --window-save-state=never \
  --quit-after-last-window-closed=true \
  --title="switchboard: <project>/<session name>" \
  --working-directory=<cwd> -e <shell> -lc '<resume command>'
```

and the card's "jump to" raises that window by title with the System Events
script above (record the instance pid too). This matches the design's
priority 4 (hand off when the native tool is better) and needs no terminal
rendering to ship persistence. When Ghostty 1.3 is installed, switch to its
AppleScript (`new window with configuration`, `focus`), which gives tabs and
proper ids. Do not build on Cmux's socket for the product; offer it as an
optional integration for users who enable its password mode, mainly for
`set-status` and `claude-hook`.

**Commands, services and shells: embed `egui_term`.** These are the things
the board wants to show inline: a service's log tail, a build's last lines,
a quick shell in a directory. The spike shows the widget handles them at
0 % idle and a few ms per frame, and the `Terminal` port can wrap it in one
adapter. Pin it as a vendored copy of `Harzu/egui_term` at `31bbc7ab` (or a
git dependency once master bumps to egui 0.36) with `alacritty_terminal =
"0.26"`; apply the focus patch and call `tty::setup_env`-equivalent env
setting. If spike 02 picks tmux as the process host, the embedded terminal
attaches with `tmux attach -t <session>` and the same widget shows agents in
the session view too, so the user gets both: embedded for a glance, Ghostty
for a long sit-down with an agent.

Crate versions: `eframe`/`egui` 0.36.1, `egui_term` git `31bbc7ab`
(vendored), `alacritty_terminal` 0.26.0, `egui_kittest` 0.36.1 for tests.
