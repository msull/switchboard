# Spike 11: a Dispatch service as a Switchboard service session

**Question.** Dispatch serves a lane for a stage (the frontend a tester
opens) as a Switchboard service session launched with an argv:
`$SHELL -lc 'exec "$@"' dispatch-service env BROWSER=none PORT=<port>
npm start`. Does the login shell find `npm`, does the server answer,
and is it gone, port and all, after `tmux kill-session`? Which check
reads a dev server's port as taken? How fast does a readiness probe
fail against a server that is not up yet?

Spike 20 ([`20-automation-shell`](../20-automation-shell/README.md))
measures the automation shell that launches the service today.

**Setup.** tmux 3.7c on a `switchboard-test-spike11` socket, node 24.3,
macOS. `server.js` starts the way react-scripts does: a parent `node`
that spawns a child `node` listening on `$PORT` and waits on it.
`probe.py` binds the way Rust's `std::net::TcpListener::bind` does on
Unix (`SO_REUSEADDR` on) and connects the way a probe does.

## 1. The launch, and what `kill-session` leaves

```sh
./start.sh   # the forking node server in a pane on port 3155
./kill.sh    # kill-session, then the group and the port every 200 ms
./npm.sh <client frontend worktree>   # the same with the real `npm start`
```

The forking server:

```
pane 52051 pgid 52051
52051 52051 52050 node server.js
52062 52051 52051 node -e require('http').createServer(...).listen(...)
http 200
node 52062 ... IPv6 ... TCP *:3155 (LISTEN)
after 200ms: 0 in group, 0 listen lines
```

`npm start` in a client frontend worktree (react-app-rewired over
react-scripts 5):

```
answered 200 after 27s
53240 53240 53239 npm start
53271 53240 53240 node .../.bin/react-app-rewired start
53280 53240 53271 node .../react-app-rewired/scripts/start.js
node 53280 ... IPv4 ... TCP *:3155 (LISTEN)
after 200ms: 0 in group, 0 listen lines
```

The login shell's `exec` hands the pane to `env`, then to `npm`: the
pane's pid is `npm` itself, and PATH resolved from the user's profile.
(The automation shell's `exec` hands over the same way; its PATH is the
pane's.)
Every descendant stays in the pane's process group, and all of it was
gone, port free, within 200 ms of `kill-session`. No grandchild
survived either time; the port-free condition in Dispatch's stop stays,
for a server that daemonizes or forks out of the group.

## 2. Which check sees the port taken

`python3 probe.py` against node servers on port 3157 with each `HOST`
a CRA dev server may be given:

| Server binds | bind 127.0.0.1 | bind ::1 | bind 0.0.0.0 | connect 127.0.0.1 | connect ::1 |
|---|---|---|---|---|---|
| no host (`*`, dual stack) | bound | bound | refused | answered | answered |
| `0.0.0.0` (CRA's default) | bound | bound | refused | answered | nothing |
| `localhost` (node resolves `::1`) | bound | refused | bound | nothing | answered |
| `127.0.0.1` | refused | bound | bound | answered | nothing |
| `::1` | bound | refused | bound | nothing | answered |
| nothing | bound | bound | - | nothing | - |

Under `SO_REUSEADDR` a specific address binds beside a wildcard
listener, so binding only the loopbacks reads CRA's default `*:3155` as
free. Binding `0.0.0.0`, `127.0.0.1` and `[::1]` together refuses every
shape, and binds all three with nothing there. That is
`Repo::port_free`.

## 3. A probe against a server not up yet

```
probe of a closed port: nothing/nothing in 0.3 ms
```

A connect to a closed loopback port is refused at once, so the
readiness probe (`Repo::answers_http`: connect with a 200 ms timeout,
`GET <path> HTTP/1.0`, a status line within 500 ms) costs nothing while
the server compiles. CRA took 27 s to answer on this machine; the
client frontend's `ready.within_secs = 120` leaves room.

**Recommendation.** Launch services as above: values travel as argv
elements, never as shell source, and only the pipeline file's literals
and the port reach them (so no secret may be in `serve.env`). Read a
port as free only when all three binds succeed. Count a service as
stopped only when its session reads gone and its port binds again; the
measurements say that happens within a pass of the kill.
