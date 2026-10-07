# Spike 14: credentials for one child through `switchboard-env exec`

**Question.** A session's environment sets reach a child only through
`switchboard-env exec -- <command>`, which asks the running app over
`control.sock` with the pane's record id and launch token. Do the id and
token reach a child of the pane? Does `aws-vault exec` work against a
cached session with stdin closed, with and without `-n`? Does a Claude
Code run with Dispatch's launch flags run `<abs>/switchboard-env exec --
true` without a permission prompt?

**Setup.** tmux 3.x on a `switchboard-test-spike14-<pid>` socket, macOS.

## (b) The id and token reach a child of the pane: yes (shell)

```sh
S=switchboard-test-spike14-$$
tmux -L $S new-session -d -s probe \
  -e SWITCHBOARD_RECORD_ID=rec-1 -e SWITCHBOARD_RECORD_TOKEN=tok-fake \
  "sh -c 'sh -c \"echo id=\\\$SWITCHBOARD_RECORD_ID token=\\\$SWITCHBOARD_RECORD_TOKEN\" > b.txt; sleep 5'"
cat b.txt
tmux -L $S kill-server
```

```
id=rec-1 token=tok-fake
```

A grandchild of the pane sees both. Claude Code's Bash tool is one more
child of the same kind and inherits the agent's environment, the way the
hook helper already reads `SWITCHBOARD_RECORD_ID`. The run under
Claude's Bash tool itself is part of (e) and was not made here.

`tests/control.rs` covers the rest of the path end to end with the real
binary: `exec -- env` prints the set's pairs and no
`SWITCHBOARD_RECORD_TOKEN`; with `vault` the child is
`aws-vault exec <profile> -- <command>`; with `sso` and an `aws` that
fails it stops with the `aws sso login --profile <p>` line.

## (d) `aws-vault exec` against a cached session: not run

The check prints a real session's environment, which the implementing
session's permission classifier refused to put in a tool result. The
owner runs it by hand, outside an agent:

```sh
aws-vault exec <profile> -- aws sts get-caller-identity </dev/null
aws-vault exec -n <profile> -- aws sts get-caller-identity </dev/null
```

The binary uses the plain form, as the issue says. If the plain form
prompts and `-n` does not, change `exec` in `src/bin/switchboard-env.rs`
to pass `-n`.

## (e) Dispatch's launch flags and `switchboard-env exec`: not run

Not run for the same reason as (d): the implementing session stopped
short of an agent run whose purpose was to resolve credentials. The
owner runs it once, in a throwaway directory under `$HOME/code_repos`
on a test socket:

```sh
S=switchboard-test-spike14-$$
D=$HOME/code_repos/spike14 && mkdir -p $D && cd $D
tmux -L $S new-session -d -s e -c $D \
  -e SWITCHBOARD_RECORD_ID=x -e SWITCHBOARD_RECORD_TOKEN=y \
  "claude --model haiku --allowedTools 'Edit(//$D/**)' \
   'Run <abs>/switchboard-env exec -- true and say what it printed.'"
sleep 30; tmux -L $S capture-pane -p -t e | tail -20
tmux -L $S kill-server
```

With no app listening the binary prints "Switchboard is not running" and
exits 1, which is enough: the question is only whether the Bash call is
asked about. If the pane shows a permission question, the allow-rule
fallback in the plan comes back (`OperatorKind::env_flags` with one
`--allowedTools Bash(<abs>/switchboard-env exec:*)` rule, an additive
`SessionClone.args`, and a trust-text line); design.md's "Environment
sets" status section lists this as open until then.

## (f) The supervisor's `switchboard-ask` rule: not run

Whether Claude Code with the supervisor's allow rule for
`switchboard-ask` runs it without a permission prompt. The owner runs it
once, in a throwaway directory under `$HOME/code_repos` on a test
socket, with `<abs>` the directory holding the built binary:

```sh
S=switchboard-test-spike14-$$
D=$HOME/code_repos/spike14 && mkdir -p $D && cd $D
tmux -L $S new-session -d -s f -c $D \
  -e SWITCHBOARD_RECORD_ID=x -e SWITCHBOARD_RECORD_TOKEN=y \
  "claude --model haiku --allowedTools 'Bash(<abs>/switchboard-ask:*)' \
   'Run <abs>/switchboard-ask \"merge now or run the checks first? #1\" and say what it printed.'"
sleep 30; tmux -L $S capture-pane -p -t f | tail -20
tmux -L $S kill-server
```

With no app listening the binary prints "Switchboard is not running"
and exits 1, which is enough: the question is only whether the Bash
call is asked about. If the pane shows a permission question, the owner
answers it once per supervisor; nothing else depends on it. Under a
`<abs>` with a space the seed tells the supervisor to type the path
single-quoted while the rule stays bare; repeat the run from such a
directory, prompt quoted the same way, to see whether that form still
matches.
