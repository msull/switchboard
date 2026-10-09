# Spike 20: what a `Stop` says is still pending

Question: when an agent ends its turn with background work running or a
wakeup scheduled, does the `Stop` hook payload say so, in a shape the
helper can carry to `events.log` without copying owner text?

## What was checked (2026-10-08, Claude Code 2.1.295)

An interactive haiku session with `settings-spike.json`, one event per
line in the order they arrived. Spike 03 had recorded
`background_tasks` and `session_crons` as `[]` on every `Stop`
(2.1.263); this run fills them.

**1. A background `Agent` running `sleep 90`, then the turn ends.**

| Hook | `background_tasks` | `session_crons` |
|---|---|---|
| `Stop` (turn that launched it) | `[{id, type:"subagent", status:"running", description, agent_type:"general-purpose"}]` | `[]` |
| `SubagentStop`, then `UserPromptSubmit` (`<task-notification>…`), `Stop` | `[{id, type:"shell", status:"running", description, command:"sleep 90"}]` | `[]` |
| `UserPromptSubmit` (`<agent-message from="…">`), `Stop` | `[]` | `[]` |

The subagent put its own `sleep 90` in the background and handed back
early, so the second `Stop` listed the subagent's shell. A finished
task is not listed on the next `Stop`: every list is a fresh snapshot.
`SubagentStop` carries the same two keys.

**2. `ScheduleWakeup` for 60 s, then the turn ends.** The runtime rounded
it up to the next whole minute plus slack (it fired at 22:24:00 for a
request at 22:22:03):

```json
"background_tasks":[],
"session_crons":[{"id":"dd077d3e","schedule":"24 22 * * *","recurring":false,"prompt":"say woke"}]
```

A one-shot `schedule` is five cron fields with minute and hour fixed and
day, month and weekday `*`, in local time. `prompt` is owner text and is
never copied.

**3. The fired wakeup.** It runs `UserPromptSubmit` with the cron's
prompt as given (`say woke`), no tag, then `Stop` with both lists
empty. The subagent hand-back in case 1 starts `<agent-message from=…>`.

## Result

- The helper keeps, per task, only `type`, `status` and `agent_type`,
  and per cron only `schedule` and `recurring`; it never copies
  `description`, `command` or `prompt`.
- A one-shot fire time is the first local minute at or after the
  `Stop` (less a minute) that matches the schedule's fixed fields.
- Neither the fired wakeup's prompt nor `<agent-message` begins with a
  tag `is_injected` recognises, so both read as the owner typing and
  clear an open ask. That is a separate bug, not fixed with this spike.

## The live capture

Spends a few haiku-class turns. From this directory, with `$S` a private
scratch directory:

```sh
bin/run.sh "$S"        # interactive claude on switchboard-test-spike20, cwd under ~/code_repos; prints it as $W
t() { tmux -L switchboard-test-spike20 "$@"; }
send() { t send-keys -t s -l "$1"; sleep 1; t send-keys -t s Enter; }
send 'Use the Agent tool with run_in_background true and subagent_type general-purpose, with the prompt: run the Bash command sleep 90 and then reply ok. Then end your turn immediately without waiting.'   # wait 2 min
send 'Use the ScheduleWakeup tool to wake yourself in 60 seconds with the prompt: say woke. Then end your turn immediately.'   # wait 3 min
cut -f3 "$S/hooks.log" | jq -c '{e: .hook_event_name, p: ((.prompt // "") | .[0:40]), bt: .background_tasks, sc: .session_crons}'
t kill-server; rm -rf "$W"
```
