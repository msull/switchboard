# Spike 26: what a service's second process does when the session stops

**Question.** A lane's service is `$SHELL -lc 'exec "$@"' dispatch-service
bash -c '<worker> & exec <web>'` in a tmux pane. After `tmux kill-session`
(the only stop Switchboard issues, `src/adapters/tmux.rs:406`), is the
background worker gone? And when the web process exits first?

**Setup.** tmux 3.8 on a `switchboard-test-spike26` socket, macOS (Darwin 27),
`remain-on-exit on` as Switchboard sets it, a `hold` session so the server
outlives the pane. `sleep 70x` stands in for the worker and the web process.

```sh
./run.sh   # six cases; output kept in out.txt
```

## Results (out.txt)

| Case | After `kill-session` (or the web exiting) |
|---|---|
| plain `sleep 701 & exec sleep 702`, same pgid | both gone |
| worker and web with `trap "" HUP` | both alive (HUP ignored; the exec'd web inherits the ignore) |
| `nohup` worker | web gone; worker alive, ppid 1, still in the old pgid |
| worker under `setsid` (own session and pgid) | web gone; worker alive, ppid 1 |
| web exits on its own, plain worker | `pane_dead=1`; worker gone within 1 s |

The last case's second variant (web exits, then `kill-session`) found the
worker already gone before the kill, so it adds nothing.

## Recommendation

A plain background process in the pane's group is ended with the pane, in a
stop and when the web process exits. A process that ignores HUP, or leaves
the session (`setsid`, a daemonizing worker), survives, so the stop's
process check must not rest on the pane alone. Not measured: Celery itself,
SIGKILL escalation, tmux 3.7c (spike 13 used 3.7c).
