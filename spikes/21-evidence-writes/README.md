# Spike 21: which writes reach an evidence directory unasked

Question: an agent stage's evidence directory lives in the attempt
directory, outside the agent's cwd. Dispatch already launches Claude
Code with `--allowedTools Edit(//<attempt dir>/**)` (`write_flags`).
Which ways of putting a file there work without a permission prompt,
and does `--add-dir <evidence dir>` change the answer?

## What was checked (2026-10-09, Claude Code 2.1.295)

`$S` is a throwaway directory under `$HOME/code_repos`. The agent's cwd
is `$S/tree` (a fresh `git init`); the attempt directory is
`$S/data/attempt`, the evidence directory `$EV=$S/data/attempt/evidence`
with `cp/` and `mv/` made in advance so each case measures one write.
Every case is one print-mode turn; a tool that would prompt is denied
and listed in the result's `permission_denials`.

```sh
cd $S/tree
claude -p "<prompt>" --model haiku --output-format json \
  "--allowedTools=Edit(/$S/data/attempt/**)" [extra flags]
```

(`--allowedTools` takes several values, so the rule goes in its `=`
form or the prompt is read as a second rule.)

| Case | Prompt | Today's rule | With `--add-dir $EV` |
|---|---|---|---|
| Write tool | create `$EV/write/a.txt` | written, no prompt (`write/` made too) | not rerun |
| Bash `cp` | `cp $S/tree/src.txt $EV/cp/b.txt` | denied ("needs approval") | denied |
| Bash `mv` | `mv $S/tree/mv.txt $EV/mv/c.txt` | denied | denied |
| Bash `mkdir -p` | `mkdir -p $EV/sub/deeper` | denied ("This command requires approval") | denied |

For information only: `cp` with `--add-dir $EV --permission-mode
acceptEdits` ran unasked. Dispatch does not set a permission mode; an
operator that already runs in `acceptEdits` gets that from its own
`args`.

## The browser case

`@playwright/mcp@0.0.82`, loaded only for the spike's session with
`--mcp-config <file> --strict-mcp-config` and
`--allowedTools=Edit(/$S/data/attempt/**),mcp__playwright__browser_navigate,mcp__playwright__browser_take_screenshot`.
The page was `python3 -m http.server 18721 --bind 127.0.0.1` serving a
one-line `index.html` from `$S/site`. The configs:

```json
// mcp-real.json: what a tester operator has
{"mcpServers":{"playwright":{"command":"npx","args":["-y","@playwright/mcp@0.0.82","--headless"]}}}
// mcp-outdir.json
{"mcpServers":{"playwright":{"command":"npx","args":["-y","@playwright/mcp@0.0.82","--headless","--output-dir","$EV"]}}}
// mcp-elsewhere.json
{"mcpServers":{"playwright":{"command":"npx","args":["-y","@playwright/mcp@0.0.82","--headless","--output-dir","$S/elsewhere"]}}}
// mcp-unrestricted.json
{"mcpServers":{"playwright":{"command":"npx","args":["-y","@playwright/mcp@0.0.82","--headless","--allow-unrestricted-file-access"]}}}
```

The prompt: navigate to `http://127.0.0.1:18721/`, then
`browser_take_screenshot` with `filename` set as below.

| Config | `filename` | Result |
|---|---|---|
| `mcp-real.json` | `$EV/shots/real.png` | refused by the server: `File access denied: … is outside allowed roots. Allowed roots: $S/tree` |
| `mcp-real.json` + `--add-dir $EV` | `$EV/shots/real-add.png` | refused: allowed roots `$S/tree/.playwright-mcp, $S/tree`; an added directory is not a root |
| `mcp-elsewhere.json` | `$EV/shots/elsewhere.png` | refused: allowed roots `$S/elsewhere, $S/tree` |
| `mcp-outdir.json` | `bare.png` | written to `$S/tree/bare.png`: a named file resolves against the workspace root, not `--output-dir` |
| `mcp-outdir.json` | `$EV/shots/outdir-abs.png` | written (information only: no static `--output-dir` names a per-attempt directory) |
| `mcp-unrestricted.json` | `$EV/shots/unrestricted.png` | written, no prompt |

No case was a Claude permission prompt: the server process writes the
file, so the `Edit` rule plays no part. The server confines writes to
its roots (Claude's cwd) and its `--output-dir`.

## Result

- No launch flag changes, and no `OperatorKind::evidence_flags`:
  `--add-dir` cleared none of the prompts. A `Bash(cp:*)` rule is not
  added, because a prefix rule would allow copying anywhere.
- The Write tool reaches `{evidence}` unasked under today's rule, and so
  does any gate-only command.
- Bash `cp`, `mv` and `mkdir` into `{evidence}` ask, so an agent in the
  default permission mode cannot use them there.
- A screenshot reaches `{evidence}` when the tester's Playwright server
  runs with `--allow-unrestricted-file-access` in its operator's
  `--mcp-config`, and the tester passes `{evidence}/<file>` as
  `filename`. That flag also lets the browser read any local file and
  open `file://` URLs, so it is the owner's choice per operator; without
  it, screenshots cannot be kept.
