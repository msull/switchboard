# Spike 17: telling the owner's prompt from an injected one

Question: when Claude Code starts a turn on its own (a background task's
`<task-notification>`, a harness `<system-reminder>`), does the
`UserPromptSubmit` hook payload say so, so that a session's ask
(`switchboard-ask`) clears only on a prompt the owner typed?

## What was checked (2026-10-07, Claude Code 2.1.293)

The live capture at the end of this file, plus two read-only sources
that agree with it.

**The live capture.** An interactive haiku session with
`settings-spike.json`, one event per line in the order they arrived
(prompt tags are the first characters of `prompt`):

| Case | Hooks that ran | `UserPromptSubmit` prompt starts |
|---|---|---|
| launch | `SessionStart` (`source: startup`) | — |
| 1. typed `say pong` | `UserPromptSubmit`, `Stop` | `say pong` |
| 2. `run_in_background` `sleep 15`, then stop | `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `Stop`, `SubagentStop`; 15 s later `UserPromptSubmit`, `Stop` | `<task-notification>\n<task-id>…` |
| 3. read a file, `sleep 20`, change the file mid-sleep | `UserPromptSubmit`, `PreToolUse`, `PostToolUse` ×2, `Stop` | the typed prompt only |
| 4. typed `/clear` | `SessionEnd`, `SessionStart` (`source: clear`) | — (no `UserPromptSubmit`, no `Stop`) |

Every `UserPromptSubmit`, typed or not, had the same keys:

```
cwd hook_event_name permission_mode prompt prompt_id scratchpad_dir session_id transcript_path
```

So a background task's return does run `UserPromptSubmit`, after the
turn's `Stop`, with `<task-notification>` as the very first characters
and no field marking the origin. A file changed mid-turn starts no
prompt (the reminder rides on a tool result), and `FileChanged` did
not fire for it. `/clear` runs `SessionStart` with no `Stop` before
the owner's next prompt.

**Logged payloads.** Every `UserPromptSubmit` in spike 03's logs
(`spikes/03-session-state/*.log`, Claude Code 2.1.263, typed prompts
only) has these top-level keys and no others:

```
cwd hook_event_name permission_mode prompt prompt_id session_id transcript_path
(+ scratchpad_dir on interactive runs)
```

**The 2.1.293 binary.** `strings` on
`~/.local/share/claude/versions/2.1.293` shows the payload built in both
places that run the hook:

```js
{...xd(session, ..., mode), hook_event_name:"UserPromptSubmit", prompt:e, ...!1, session_title:cm(id)}
```

`...!1` is a spread the build compiled out, so no origin field reaches
the hook. The caller passes a prompt source (`typed`, `queued`,
`suggestion_accepted`, `system`, `sdk`) and a wake-up source, but
neither is in the payload. Task notifications carry
`origin: {kind: "task-notification", ...}` inside the transcript, not
in the hook input, and their text starts with the `<task-notification>`
tag (the binary's `xu="task-notification"` builds `<${xu}>`).

## Result

- No top-level string field marks the origin, and there is no nested
  `origin` in the hook input, so the helper uses the prompt's prefix:
  after `trim_start`, `<task-notification>` or `<system-reminder>` means
  injected. Only the verdict is written to `events.log`.
- A `SessionStart` does not open a turn: after `/clear` the owner's
  next prompt arrives with no `Stop` between, so the core must not read
  the `Working` that `SessionStart` set as a turn in progress.
- Unmeasured: whether an Esc interrupt fires any hook, and whether a
  `<system-reminder>` ever starts a turn of its own (none did here). The
  core does not depend on them: a prompt that arrives while a turn is
  open never clears an ask (the turn guard), and `docs/design.md`
  "Asking the owner" lists what that leaves open.

## The live capture

Spends a haiku-class turn or three. From this directory, with `$S` a
private scratch directory:

```sh
bin/run.sh "$S"        # interactive claude on switchboard-test-spike17, cwd under ~/code_repos; prints it as $W
t() { tmux -L switchboard-test-spike17 "$@"; }
send() { t send-keys -t s "$1"; sleep 1; t send-keys -t s Enter; }   # Enter apart, or it lands in the text
send 'say pong'                                                       # case 1
send 'Use the Bash tool with run_in_background true to run: sleep 15; echo done. Then end your turn immediately without waiting.'   # case 2; wait 30 s
echo 'first version' > "$W/notes.txt"
send 'Read notes.txt with the Read tool, then run sleep 20 in the foreground with Bash, then say the word finished.'
sleep 10; echo 'second version' > "$W/notes.txt"                      # case 3, mid-sleep; wait 30 s
send '/clear'                                                         # case 4
cut -f3 "$S/hooks.log" | jq -c '{event: .hook_event_name, source, keys: keys, tag: ((.prompt // "") | .[0:40])}'
t kill-server; rm -rf "$W"
```

`settings-spike.json` registers `bin/log-hook.sh` for every event the
app uses plus `PreToolUse`, `SubagentStop` and `FileChanged`; it copies
each payload whole into `$S/hooks.log`, which stays in the scratch
directory. Record here only the first tag of each prompt and the keys.
