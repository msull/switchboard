# Spike 20: the automation shell

**Question.** Agents' tool calls and Dispatch's services should run in
a fixed, profile-free shell (`/bin/bash --noprofile --norc`) while the
owner's own sessions keep their login shell. Where does a pane's shell
come from today, and which knobs does tmux actually honour?

**Answer.**

- tmux's `default-shell` follows the `SHELL` of the client that started
  the server, so on the owner's machine it is `/bin/zsh`.
- A pane command given as **one word** runs as `default-shell -c
  "<word>"`, which is `zsh -c` and reads `~/.zshenv`. Switchboard used
  to join every argv into one word, so agents, the runner and services
  all went through the owner's zsh.
- A command given as **several words** is exec'd directly, no shell in
  between, through `source-file -` as on the command line.
- `new-session -e SHELL=…` is replaced with `default-shell` in the pane.
  `-e PATH=…` is replaced too, by the PATH of the client that sent the
  `new-session` (tmux does this for a client not attached to a session,
  which Switchboard's spawning client never is). Every other `-e` value
  gets through.
- So the agent's shell goes on its argv, `/usr/bin/env SHELL=/bin/bash
  CLAUDE_CODE_SHELL=/bin/bash claude …`, and a pane's PATH goes on the
  spawning client's environment.
- A missing program in the several-words form leaves a dead pane with
  status 1 (tmux's own failed exec), where `zsh -c` used to say 127.

Measured on tmux 3.7c (macOS) on a `switchboard-test-spike20` socket,
the probe itself run under `/bin/bash --noprofile --norc` with
`PATH=/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin` and the owner's
`SHELL=/bin/zsh`, with a config holding only `set -g remain-on-exit on`.

## Reproduce

```sh
S=switchboard-test-spike20
D=$PWD
printf 'set -g remain-on-exit on\n' > t.conf
run() { printf '%s\n' "$1" | tmux -L $S -f t.conf start-server \; source-file -; }
run "new-session -d -s one 'echo \"\$0 \$SHELL\" > $D/one.out; sleep 5'"
run "new-session -d -s many '/bin/sh' '-c' 'echo \"\$0 \$(ps -o comm= -p \$\$)\" > $D/many.out; sleep 5'"
run "new-session -d -s eshell -e 'SHELL=/bin/sb-marker' -e 'FOO=bar' -e 'PATH=/sb-e-path:/usr/bin:/bin' '/bin/sh' '-c' 'echo \"\$SHELL \$FOO \$PATH\" > $D/eshell.out; sleep 5'"
run "new-session -d -s envpre '/usr/bin/env' 'SHELL=/bin/sb-marker' '/bin/sh' '-c' 'echo \$SHELL > $D/envpre.out; sleep 5'"
run "new-session -d -s missing '/nonexistent/prog' 'arg'"
sleep 1
tmux -L $S show-options -gv default-shell
for f in one many eshell envpre; do echo "$f: $(cat $D/$f.out)"; done
tmux -L $S list-panes -t =missing -F '#{pane_dead} #{pane_dead_status}'
tmux -L $S kill-server
```

The scripts are single-quoted for tmux on purpose: inside double quotes
tmux expands `$VAR` from its own environment before the pane sees it,
which makes a probe report tmux's values instead of the pane's.

## Output

```
/bin/zsh                                         default-shell
one: zsh /bin/zsh                                one word: zsh -c
many: /bin/sh /bin/sh                            several words: exec'd, no zsh
eshell: /bin/zsh bar /usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin
                                                 -e SHELL and -e PATH replaced, FOO kept
envpre: /bin/sb-marker                           env prefix sets SHELL
1 1                                              missing program: dead, status 1
```

The `eshell` PATH is the probe's own PATH, the client's, not the
`/sb-e-path:…` passed with `-e`.

## Claude Code's shell (2.1.294, read from the binary, not run)

- The Bash tool's shell is `CLAUDE_CODE_SHELL` when it names bash or zsh
  and runs, else `$SHELL` when that is bash or zsh, else the first bash
  or zsh found in `/bin`, `/usr/bin`, `/usr/local/bin`,
  `/opt/homebrew/bin`.
- It builds a snapshot with `<shell> -c -l <script>`, which sources
  `~/.bashrc` (`~/.zshrc` for zsh) when it exists and ends with `export
  PATH=<the claude process's PATH>`. So `-l` still runs `/etc/profile`
  and `~/.profile` while the snapshot is built, but no profile's PATH
  reaches a command: the pane's PATH does.
- The snapshot, `~/.claude/shell-snapshots/snapshot-<bash|zsh>-*.sh`, is
  built at the first Bash exec, not at startup (the shell provider is
  made lazily by the exec path; a plugin refresh says "the next Bash
  exec rebuilds it"), and it is deleted when the session exits.

## What Switchboard does with this

- `spawn_script` passes a command of two or more elements as separate
  words; a one-element command keeps the `default-shell -c` form.
- The agent launcher puts `/usr/bin/env SHELL=/bin/bash
  CLAUDE_CODE_SHELL=/bin/bash` (Codex: `SHELL` only) ahead of the binary.
- `TmuxHost::spawn` sets the spawning client's PATH to the spec's own
  `PATH` if it has one, else `SWITCHBOARD_PATH_PREPEND` ahead of the
  augmented PATH, which now includes `~/.cargo/bin`.
- Dispatch's services run `/bin/bash --noprofile --norc -c 'exec "$@"'
  dispatch-service env … <argv>`.
