# Spike 7: Can a Claude Code session be cloned up to a chosen message?

**Answer: yes.** A transcript is a JSONL file whose records carry the
session id in a `sessionId` field and chain by `parentUuid`. Copying the
file up to (not including) the Nth human prompt, replacing every
occurrence of the old id with a fresh UUID, and saving it beside the
original as `<new-id>.jsonl` gives a session that `claude --resume
<new-id>` opens with exactly the truncated history. Both `-p` and the
interactive client accept it, and each session appends to its own file
afterwards.

Run on 2026-09-15 with Claude Code `2.1.273`, `--model haiku`, in the
throwaway directory `~/code_repos/switchboard-spike-clone` (already
trusted, so no trust dialog). Total spend about $0.03.

- Helper: [`fork.py`](fork.py) (the cut-and-rename rule, ~20 lines)
- Evidence: `ids.txt`, `run1..3.json`, `resume-clone.json`,
  `resume-orig.json`

## 1. Build a three-turn session

```
$ ID=$(uuidgen | tr A-Z a-z)
$ claude -p --model haiku --max-turns 1 --session-id "$ID" --output-format json "reply with the single word pong"
$ claude -p --model haiku --max-turns 1 --resume "$ID" --output-format json "now reply with the single word ping"
$ claude -p --model haiku --max-turns 1 --resume "$ID" --output-format json "reply with the single word zebra"
# results: 'pong', 'ping', 'zebra'
```

The transcript (37 lines) has three `user` records with a string
`message.content` (the human prompts). Everything else is `assistant`,
`attachment`, and bookkeeping (`queue-operation`, `last-prompt`,
`atis-latch`, `ai-title`, `mode`). Tool results are also `user` records
but their content is a list, so "a string content" is the test for a
typed prompt (the same rule `adapters/transcript.rs` uses).

## 2. Fork before the third prompt

```
$ NEW=$(uuidgen | tr A-Z a-z)
$ python3 fork.py ~/.claude/projects/<slug>/$ID.jsonl $NEW 3
.../<slug>/013337ee-....jsonl 32 lines; stopped before prompt 3
```

`fork.py` keeps every line up to the third string-content `user`
record, does a plain text replace of the old id with the new one (the id
appears in `sessionId` on most records and nowhere else), and writes
`<slug>/<new>.jsonl`. No `parentUuid` rewriting is needed: the chain is
intact because the cut is a prefix.

## 3. Resume both and ask what was said last

```
$ claude -p --model haiku --max-turns 1 --resume "$NEW" --output-format json \
    "What single word did you reply with in your most recent previous message? Answer with just that word."
# 'ping', session_id 013337ee-..., $0.0027
$ claude -p ... --resume "$ID" ... (same question)
# 'zebra', session_id 5d56bb5b-..., $0.0029
```

The clone remembers the conversation as it stood before the cut; the
original is untouched. The clone's file grew by its own new turn, the
original's did not.

Interactive check on a test tmux socket:

```
$ tmux -L switchboard-test-clone new-session -d -s c -c ~/code_repos/switchboard-spike-clone \
    "claude --resume $NEW --model haiku"
$ tmux -L switchboard-test-clone capture-pane -p -t c
❯ reply with the single word pong
⏺ pong
❯ now reply with the single word ping
⏺ ping
❯ What single word did you reply with ...
⏺ ping
```

## Findings that shape the feature

- **The cut rule.** Drop everything from the chosen typed prompt onward.
  Keep the bookkeeping records that precede it; they are harmless.
- **Rename by text replace.** The old id occurs only as the `sessionId`
  value. A JSON-aware rewrite is safer for the real implementation, but
  a prefix copy plus id substitution is all the provider needs.
- **File mode.** Claude writes its transcripts `0600`; the copy must be
  written with the same mode (the spike's first copy came out `0644`).
- **Placement.** The copy goes in the same slug directory as the
  original. Spike 1 says `--resume` finds ids from any cwd, but keeping
  the copy beside the source means the `cwd` recorded in it and the
  directory agree.
- **Subagents.** A session with subagents also has a sibling
  `<id>/subagents/` directory. It was not copied here and the resume
  did not need it. Agent tool calls before the cut keep their summarized
  results inside the main file.
- **This is a write into `~/.claude/projects/`.** `docs/design.md`
  currently says Switchboard reads transcripts but never copies them.
  Cloning is the one deliberate exception: it creates a *new* transcript
  and never touches an existing one.
- **Codex.** Not tried. Codex rollouts have a `session_meta` first
  record with the id; the same prefix-copy idea probably applies but is
  unverified.
- **The primed message.** The chosen prompt's text is in the record that
  the cut drops; the app already has it from the parsed `Turn.user`, and
  `UiState.input_drafts` pre-fills the message box, so no provider help
  is needed for that half.
