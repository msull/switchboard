# Spike 1: Can Claude Code sessions be captured and resumed reliably?

**Answer: yes.** Claude Code (2.1.263) lets the caller *assign* the session id
at launch (`--session-id <uuid>`), writes one transcript file per session at a
predictable path, and `--resume <uuid>` works from any terminal, any cwd, after
the original process is gone. The one real risk is retention: transcripts are
deleted after 30 days by default (`cleanupPeriodDays`), which is confirmed on
this machine.

Everything below was run on 2026-09-05/06 in this directory. Test sessions
were run with `--model haiku --max-turns 1`; total spend was about $0.04.

- Claude Code version: `2.1.263`
- Codex version: `codex-cli 0.152.1`
- Helper script: [`watch-new-session.sh`](watch-new-session.sh)
- Raw outputs kept as evidence: `help.txt`, `run*.json`, `resume-*.json`

## 1. Session identity: can an id be assigned at launch?

Yes. Relevant flags from `claude --help` (full text in `help.txt`):

```
  --session-id <uuid>                   Use a specific session ID for the
                                        conversation (must be a valid UUID)
  -r, --resume [value]                  Resume a conversation by session ID, or
                                        open interactive picker with optional
                                        search term
  -c, --continue                        Continue the most recent conversation in
                                        the current directory
  --fork-session                        When resuming, create a new session ID
                                        instead of reusing the original (use
                                        with --resume or --continue)
  -n, --name <name>                     Set a display name for this session
                                        (shown in the prompt box, /resume
                                        picker, and terminal title)
  --no-session-persistence              Disable session persistence - sessions
                                        will not be saved to disk and cannot be
                                        resumed (only works with --print)
  --bg, --background                    Start the session in the background and
                                        return immediately. Prints the id ...
```

Note that `--session-id`, `--resume`, `--name` and `--fork-session` carry no
"(only works with --print)" qualifier; `--no-session-persistence` does.

Proof. Generate a UUID, launch with it, check the reported id:

```
$ ID=$(uuidgen | tr A-Z a-z); echo $ID
b3b9fe1f-3ace-4d7e-ad99-1649d638387e
$ cd work-a
$ claude -p --model haiku --max-turns 1 --session-id "$ID" --name "spike-a" \
    --output-format json "reply with the word pong"
# result record, abridged:
{'result': 'pong', 'is_error': False,
 'session_id': 'b3b9fe1f-3ace-4d7e-ad99-1649d638387e', 'total_cost_usd': 0.0025089}
```

Resume it with a second process and ask what it said before:

```
$ claude -p --model haiku --max-turns 1 --resume "$ID" --output-format json \
    "What single word did you reply with in your previous message? Answer with just that word."
{'result': 'pong', 'is_error': False,
 'session_id': 'b3b9fe1f-3ace-4d7e-ad99-1649d638387e', 'total_cost_usd': 0.0027109}
```

The resumed process kept the same id and remembered the conversation.

Two gotchas found along the way:

- A malformed id is rejected up front:
  `claude -p --session-id nope "hi"` ->
  `Error: Invalid session ID. Must be a valid UUID.`
- `--bare` broke authentication in this environment
  (`"result":"Not logged in · Please run /login"`, `apiKeySource: none`),
  while the same command without `--bare` worked. Do not use `--bare` for
  Switchboard launches.

## 2. Discovery: where sessions live and whether an id can be found after the fact

**Location.** `~/.claude/projects/<slug>/<session-id>.jsonl`, where `<slug>` is
the absolute cwd with every `/`, `_` and `.` replaced by `-`:

```
$ ls ~/.claude/projects | head
-Users-sully
-Users-sully-code_repos-oleev-oleev-app          # older slug form kept '_'
-Users-sully-code-repos-delta
-Users-sully-code-repos-personal-promptbox
...
$ ls -la ~/.claude/projects/-Users-sully-code-repos-personal-switchboard-spikes-01-claude-resume-work-a/
-rw-------  17332 4f200626-7fd2-4f1c-b6b8-a129e11a3244.jsonl
-rw-------  17421 54eab909-8116-4ebc-9d84-4aaecf07e8ce.jsonl
-rw-------  27638 b3b9fe1f-3ace-4d7e-ad99-1649d638387e.jsonl
-rw-------   2247 e57dca0c-edbf-4265-9dd1-27e044f0c828.jsonl
```

(One existing directory on this machine keeps a literal `_`, so the slug rule
has changed across versions; current 2.1.x replaces `_` too. Do not rely on
computing the slug for old sessions; store the id and let `--resume` find it.)

A session may also own a sibling directory `<session-id>/` (seen:
`<id>/subagents/agent-*.jsonl`) for subagent transcripts.

**Discovery by watching the directory works.** `watch-new-session.sh <cwd>`
polls the slug directory every 100 ms and prints the first new `*.jsonl`
basename. Run alongside an unnamed launch:

```
$ ./watch-new-session.sh "$PWD/work-a" 60 > watched.id &
$ (cd work-a && claude -p --model haiku --max-turns 1 --output-format json "reply with the word pong" > ../run3.json)
$ cat watched.id
23d0848d-f5f6-4cd9-ba26-4abc19cd76e7
$ grep -o '"session_id":"[^"]*"' run3.json | sort -u
"session_id":"23d0848d-f5f6-4cd9-ba26-4abc19cd76e7"
```

The watcher's id matches the id Claude reported. The file appears at process
start (before the first API response). Caveat: two sessions started in the
same cwd within the same 100 ms window would be ambiguous, which is why the
recommendation below assigns the id instead of discovering it.

**A better discovery source: the live-session registry.** Interactive
sessions register themselves in `~/.claude/sessions/<pid>.json`:

```
$ cat ~/.claude/sessions/76838.json
{
    "pid": 76838,
    "sessionId": "8c7ba60e-047d-4563-b6af-5f7f4bd030b0",
    "cwd": "/Users/sully/code_repos/personal/promptbox",
    "startedAt": 1788672180062,
    "procStart": "Sun Sep  6 05:22:57 2026",
    "version": "2.1.263",
    "kind": "interactive",
    "entrypoint": "cli",
    "messagingSocketPath": "/tmp/cc-socks/76838.sock",
    "name": "promptbox-d7",
    "nameSource": "derived",
    "status": "busy",
    "statusUpdatedAt": 1788674187593
}
```

Switchboard knows the pid it spawned, so `~/.claude/sessions/<pid>.json` gives
an unambiguous pid -> sessionId mapping, plus `status: idle|busy` (directly
relevant to spike question 3, "waiting on you"). Only interactive sessions
appeared here; the `-p` test runs did not register.

**Transcript metadata usable for a card caption.** Each JSONL line is one
record. Fields observed (parsed from `b3b9fe1f...jsonl`):

| record type      | fields                                                                |
|------------------|-----------------------------------------------------------------------|
| `custom-title`   | `customTitle` (from `--name` or `/rename`), `sessionId`               |
| `ai-title`       | `aiTitle` (auto-generated summary, e.g. "Reply with the word pong"; interactive example: "Doc icon recording status indicator") |
| `user`           | `cwd`, `gitBranch`, `version`, `timestamp` (ISO 8601), `entrypoint` (`cli` or `sdk-cli`), `permissionMode`, `message.content` (first user message text) |
| `assistant`      | same envelope plus `message.model` (`claude-haiku-4-5-20251001`), `requestId` |
| `last-prompt`    | `lastPrompt`, `leafUuid`                                              |
| `mode`, `permission-mode`, `file-history-snapshot`, `queue-operation`, `attachment`, `atis-latch` | bookkeeping |

Note that `cwd` is per-record: after resuming from `work-b`, later records
carry `cwd: .../work-b` while the file stays under the `work-a` slug.

## 3. Resume from elsewhere

All of these were separate processes started after the original exited.

**Same cwd:** works (section 1).

**Different cwd (`work-b`), by UUID:** works. The transcript is found
globally, not by cwd; no `work-b` slug directory was created.

```
$ cd work-b
$ claude -p --model haiku --max-turns 1 --resume "$ID" --output-format json "What single word ..."
init cwd: /Users/sully/code_repos/personal/switchboard/spikes/01-claude-resume/work-b
{'result': 'pong', 'is_error': False, 'session_id': 'b3b9fe1f-...', 'total_cost_usd': 0.0195777}
$ ls ~/.claude/projects/ | grep work-b
(nothing)
```

Cost was ~7x the same-cwd resume (0.0196 vs 0.0027), presumably a prompt-cache
miss from the changed cwd/context. Resume in the recorded cwd.

**Different cwd, by title (`--name`):** does not work; title lookup is scoped
to the current directory's sessions:

```
$ cd work-b && claude -p --resume "spike-a" "..."
Error: --resume requires a valid session ID or session title when used with --print.
Usage: claude -p --resume <session-id|title>. Provided value "spike-a" is not a UUID
and does not match any session title.
```

**Same cwd, by title:** works.

```
$ cd work-a && claude -p --model haiku --max-turns 1 --resume "spike-a" --output-format json "..."
{'result': 'pong', 'is_error': False, 'session_id': 'b3b9fe1f-3ace-4d7e-ad99-1649d638387e', 'total_cost_usd': 0.003412}
```

**Unknown but well-formed id:** clean failure, exit 1.

```
$ claude -p --resume "00000000-0000-4000-8000-000000000000" "hi"; echo exit=$?
No conversation found with session ID: 00000000-0000-4000-8000-000000000000
exit=1
```

**Not a UUID and not a title:** error message as above, but exit code 0.
Check stdout/stderr text, not only the exit code.

**Permissions and other state:** resume did not require anything beyond the
transcript file. `permissionMode` is recorded per record and the `mode` /
`permission-mode` records are replayed; MCP servers, hooks and settings are
loaded fresh from the cwd at resume time (the `system/init` record lists what
was loaded).

## 4. Interactive sessions

Same mechanism. Evidence from the on-disk files rather than driving the TUI:

- Interactive transcripts have exactly the same layout and fields; the only
  difference is `entrypoint`:
  ```
  $ grep -h -o '"entrypoint":"[^"]*"' ~/.claude/projects/*/*.jsonl | sort | uniq -c
  57045 "entrypoint":"cli"
     63 "entrypoint":"sdk-cli"
  ```
- The current interactive session on this machine is
  `~/.claude/projects/-Users-sully-code-repos-personal-promptbox/8c7ba60e-....jsonl`
  and is registered in `~/.claude/sessions/76838.json` with `kind: interactive`.
- `--session-id`, `--resume`, `--name` and `--fork-session` are not marked
  print-only in `--help`; `--no-session-persistence` is, which shows the
  authors mark print-only flags explicitly.
- `--bg` starts a session detached and prints its id; `claude attach <id>`
  reopens it. That is an alternative process host, but it is Claude-specific
  and does not survive reboot, so it is not the persistence mechanism.

## 5. Other agents

```
$ for a in codex gemini aider cursor-agent opencode; do printf '%-13s ' $a; which $a || echo "not found"; done
codex         /opt/homebrew/bin/codex
gemini        not found
aider         not found
cursor-agent  not found
opencode      not found
```

Codex (`codex-cli 0.152.1`) has an equivalent model:

```
$ codex resume --help
Usage: codex resume [OPTIONS] [SESSION_ID] [PROMPT]
  [SESSION_ID]  Session id (UUID) or session name. UUIDs take precedence if it parses.
      --last    Continue the most recent session without showing the picker
      --all     Show all sessions (disables cwd filtering and shows CWD column)
$ codex --help | grep -E 'resume|fork|archive'
  resume    Resume a previous interactive session ...
  fork      Fork a previous interactive session ...
  archive   Archive a saved session by id or session name
```

Codex stores sessions at
`~/.codex/sessions/YYYY/MM/DD/rollout-<timestamp>-<uuid>.jsonl`; the first
line is a `session_meta` record with `session_id`, `cwd`, `cli_version`,
`originator`. No flag to assign the id at launch was found in
`codex --help` / `codex exec --help` (only `--ephemeral` to disable saving), so
for Codex the id must be discovered from the newest rollout file after
launch, then resumed with `codex resume <uuid>`. Codex's resume picker filters
by cwd by default (`--all` disables), but resume by UUID is direct.

## 6. Retention

**Sessions are pruned after 30 days by default. A month-old resume will not
work unless `cleanupPeriodDays` is raised.**

Evidence, from the CLI bundle's settings schema:

```
$ grep -a -o 'cleanupPeriodDays:T()[^"]*"[^"]*"' "$CLAUDE_CODE_EXECPATH"
cleanupPeriodDays:T().int().positive().optional().describe("Number of days to
retain chat transcripts before automatic cleanup (default: 30). Minimum 1. Use
a large value for long retention; use --no-session-persistence to disable
transcript writes entirely."
```

And on this machine, which has used Claude Code since 2025-09 (per
`~/.claude/history.jsonl`) with no `cleanupPeriodDays` in `~/.claude/settings.json`:

```
$ head -1 ~/.claude/history.jsonl | ... -> 2025-09-29
$ find ~/.claude/projects -maxdepth 2 -name '*.jsonl' | xargs stat -f '%Sm %N' -t %Y-%m-%dT%H:%M | sort | head -1
2026-08-07T14:41 .../-Users-sully-code-repos-delta/0c3fccb9-....jsonl
$ date -u
Sun Sep  6 06:01:08 UTC 2026
$ find ~/.claude/projects -maxdepth 2 -name '*.jsonl' | wc -l ; du -sh ~/.claude/projects
77
301M
```

Of 77 transcripts, the oldest is exactly 30 days old despite a year of use:
the default pruning is active. The bundle also contains messages such as
"Skipping cleanup: a settings file could not ..." and "cleanup is paused until
the settings errors above are fixed", so cleanup runs at startup and is
skipped when settings are unreadable.

Mitigation options (Switchboard should pick one; the last is safest):

1. Ask the user to set `"cleanupPeriodDays": 3650` in `~/.claude/settings.json`
   (Switchboard could detect its absence and show a warning card).
2. Pass it per launch: `claude --settings '{"cleanupPeriodDays":3650}' ...`
   (`--settings <file-or-json>` exists; it would keep the *pruner* from
   deleting during launches Switchboard makes, but a plain `claude` run by the
   user would still prune. Not sufficient alone.)
3. Copy the transcript (`~/.claude/projects/<slug>/<id>.jsonl` plus the `<id>/`
   directory if present) into the project's `.switchboard/` at session end and
   restore it before resuming. Resume reads the file at the slug path, so a
   restored copy resumes normally. Untested here.

## Recommendation

Assign the id; do not discover it.

**Launch** (in the session's recorded cwd, with the user-chosen session name):

```
ID=$(uuidgen | tr A-Z a-z)          # store in the workspace record before spawning
cd "$CWD" && claude --session-id "$ID" --name "$NAME"
```

Store `{agent: "claude", resume: ID, cwd: CWD, name: NAME, pid}` in the
workspace record before the process starts, so a crash mid-launch loses
nothing. As a cross-check after spawn, read `~/.claude/sessions/<pid>.json`
and confirm `sessionId == ID`; it also yields `status` for free.

**Resume** (same command shape, any time later, from any terminal):

```
cd "$CWD" && claude --resume "$ID" --name "$NAME"
```

Use the recorded cwd (resume works elsewhere but is ~7x more expensive and
title lookup is cwd-scoped). Never pass `--bare`. Treat "No conversation
found with session ID" on stdout/stderr as "transcript gone": fall back to a
fresh `claude --session-id "$NEW_ID" --name "$NAME"` seeded with the card's
notes, as the design doc already prescribes.

For "branch this session" use `claude --resume "$ID" --fork-session
--session-id "$NEW_ID"` (untested combination; `--fork-session` is documented
to mint a new id, and `--session-id` should pin it; verify before relying).

For a caption, read the transcript: `custom-title` / `ai-title` for a name,
first `user` record for the opening message, `gitBranch`, `version`,
`message.model`, and the last record's `timestamp` for "last active".

For Codex: launch normally, discover the id as the newest
`~/.codex/sessions/**/rollout-*.jsonl` created after spawn (its `session_meta`
line carries `cwd` to disambiguate), resume with `codex resume <uuid>`.

## Risks

1. **30-day pruning (high).** Confirmed active with defaults. Without one of
   the mitigations above, priority 1 in the design ("a month-old workspace is
   as usable as one from this morning") fails for Claude sessions.
2. **Undocumented paths.** The slug rule has already changed once (`_` used to
   be kept). Storing the UUID and using `--resume` avoids depending on it;
   only caption reading and the backup mitigation touch the path.
3. **Discovery race.** Watching the directory works but two launches in the
   same cwd within ~100 ms are ambiguous. Assigning the id removes this.
4. **Error signalling.** Unknown id exits 1 with a message; a non-UUID string
   exits 0 with an error message. Parse output, not only the exit status.
5. **`--bare` disables auth here.** Keep launch flags minimal.
6. **Cross-cwd resume is costly** (cache miss) and title lookup is cwd-scoped.
   Always resume in the recorded cwd.
7. **Codex cannot take an id at launch** (as far as `--help` shows); it needs
   the file-discovery path with its own race caveat.

## Test sessions created (safe to delete)

All under
`~/.claude/projects/-Users-sully-code-repos-personal-switchboard-spikes-01-claude-resume-work-a/`:

- `e57dca0c-edbf-4265-9dd1-27e044f0c828` (`--bare` run, auth failed)
- `4f200626-7fd2-4f1c-b6b8-a129e11a3244` (auth check)
- `54eab909-8116-4ebc-9d84-4aaecf07e8ce` (auth check)
- `b3b9fe1f-3ace-4d7e-ad99-1649d638387e` (named `spike-a`; the resume tests)
- `23d0848d-f5f6-4cd9-ba26-4abc19cd76e7` (watcher test)
