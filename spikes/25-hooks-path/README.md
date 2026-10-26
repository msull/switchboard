# Spike 25: which git commands run which hook under `core.hooksPath`

Question (issue #302): with `core.hooksPath=.githooks` set once on a clone,
which of Dispatch's git calls and which by-hand commits in a linked worktree
run a hook, and does `-c core.hooksPath=<missing dir>` on a command line
silence them? Plain `git` on the PATH, 2.54.0 (Apple Git-157), macOS.

Not built by the main crate. Throwaway repos under `mktemp -d`; hooks append
their name to a log.

## Commands

```sh
sh spikes/25-hooks-path/probe.sh   # output in probe.out
sh spikes/25-hooks-path/probe2.sh  # a rebase that replays commits, probe2.out
```

## Output (condensed from probe.out and probe2.out)

- `worktree add` (with the setting): `post-checkout` runs, in the new tree;
  `reference-transaction` runs (in both the clone and the new tree).
- `commit` in the linked worktree, `commit --amend`, `commit` from a
  subdirectory: `pre-commit` runs once, with cwd the worktree's top level
  (`.githooks` resolved against the worktree, not the clone). `--amend` also
  runs `post-rewrite`.
- `commit-tree`, `merge-tree --write-tree`: no hook at all.
- `update-ref`: `reference-transaction` only. `checkout --detach`:
  `post-checkout` and `reference-transaction`.
- `rebase main` that replays two commits: `pre-rebase`, `post-checkout`,
  `post-rewrite`; `pre-commit` does not run for the picks.
- `push` with the setting: `pre-push` and `reference-transaction`.
- With `-c core.hooksPath=/nonexistent/x`: `commit`, `rebase --force`,
  `checkout`, `update-ref`, `worktree add` and `push` ran no hook, and no
  error or warning was printed.
- A configured relative `core.hooksPath` naming a directory that does not
  exist: `worktree add` and `commit` ran no hook and printed nothing.
- `git -c core.hooksPath=.githooks clone`: `post-checkout` runs in the new
  clone.

## Recommendation

`git -c core.hooksPath=<missing dir>` on Dispatch's own argv keeps every one
of its calls hook-free; a plain `core.hooksPath=.githooks` in the clone's
config gives by-hand and agent `git commit` in any lane worktree the
project's `pre-commit`.
