# Spike 18: keeping spawn values off every argv

**Question.** `spawn` passed each environment value as a `new-session -e
K=V` argument. Does that leave the values readable in a process listing,
and can the same `new-session` reach tmux without a command line?

**Answer.** Yes, it leaked, and yes, it can. The tmux server is forked
from the client that started it and keeps that client's argv for as long
as it runs, so the first `new-session`'s `-e` values (Keychain and `.env`
secrets, `SWITCHBOARD_RECORD_TOKEN`) and its pane command sat in `ps`
until the server exited. The fix starts the server with a bare
`start-server` and sends the unchanged `new-session` line on the
client's stdin through `source-file -`. The client's argv is the fixed
`start-server ; source-file -`, which the server inherits.

Measured on tmux 3.7c (macOS) on a `switchboard-test-spike18` socket,
with a config holding only `set -g remain-on-exit on`.

## Reproduce

```sh
S=switchboard-test-spike18
tmux -L $S -f t.conf new-session -d -s a -e MARK=leak-old-1234 'sleep 30'
ps -o command= -p $(tmux -L $S display -p '#{pid}')
# tmux -L switchboard-test-spike18 -f t.conf new-session -d -s a -e MARK=leak-old-1234 sleep 30
tmux -L $S kill-server
```

## What does not work alone

```sh
echo "new-session -d -s a 'sleep 30'" | tmux -L $S -f t.conf source-file -
# no server running on /private/tmp/tmux-501/switchboard-test-spike18   (exit 1)

tmux -L $S -f t.conf start-server; tmux -L $S ls
# start-server exits 0, then: no server running on ...   (exit-empty)
```

`source-file` cannot start a server, and a bare `start-server` with no
session exits at once. Together in one client they work: `start-server`
starts the server, and `source-file -` creates the session before the
client detaches.

## The stdin form

```sh
echo "new-session -d -s a -e 'MARK=leak-new-5678' 'sleep 30'" \
  | tmux -L $S -f t.conf start-server \; source-file -      # exit 0, cold
ps -o command= -p $(tmux -L $S display -p '#{pid}')
# tmux -L switchboard-test-spike18 -f t.conf start-server ; source-file -
ps -axo command= | grep leak-new | grep -v grep              # nothing
tmux -L $S show-environment -t '=a' MARK                     # MARK=leak-new-5678

echo "new-session -d -s b 'sleep 30'" \
  | tmux -L $S -f t.conf start-server \; source-file -      # exit 0, warm

echo "new-session -d -s a 'sleep 30'" \
  | tmux -L $S -f t.conf start-server \; source-file -
# duplicate session: a   (exit 1): errors still reach the caller
```

## Quoting

Single quotes are not enough: tmux's parser drops a line that starts
(after optional spaces) with `#` as a comment even inside them, so a
multi-line value loses its `#` lines.

```sh
printf '%s\n' "new-session -d -s q -e 'V=x
#c d
y' 'sleep 30'" | tmux -L $S -f t.conf start-server \; source-file -
tmux -L $S show-environment -t '=q' V
# V=x
#
# y
```

Double quotes with escapes keep every value. A line break is written
`\n`, so no literal newline reaches the parser; `\`, `"`, `$` and `~`
(home expansion at a word's start) are escaped with a backslash, and a
backslash before any other character is dropped (the `\;` below).
`#{}`, `%` and `;` mean nothing inside the quotes.

```sh
printf '%s\n' 'new-session -d -s r -e "V=it'"'"'s \"q\" \$HOME \~ ; \; #{session_name}\n# Heading\ny" "sleep 30"' \
  | tmux -L $S -f t.conf start-server \; source-file -
tmux -L $S show-environment -t '=r' V
# V=it's "q" $HOME ~ ; ; #{session_name}
# # Heading
# y
```

`spawn_env_value_round_trips_through_tmux_quoting` in
`src/adapters/tmux.rs` repeats this for an environment value and a pane
command argument, with indented `##` lines as well.

## Versions

Run by hand on tmux 3.7c, and on Ubuntu 24.04's tmux 3.4 in a container
(`docker run --rm ubuntu:24.04`, `apt-get install tmux`) and through the
adapter's integration tests in CI. 3.4 reads the same stdin line to the
same pane environment, with one difference in what it prints back: its
`server_client_print` runs every command's output to a client through
`utf8_stravisx(VIS_CSTYLE…)`, so `show-environment` and
`display-message` show `$HOME` as `\$HOME` (a `$` followed by a name
character; `$` alone or before a space is untouched). The stored value
has no backslash (`env` inside the pane shows `V=x$HOME y`), and
`capture-pane -p` writes its buffer raw, so the test reads the value
from the pane's screen rather than from `show-environment`. 3.5 stopped
the encoding. 3.2 and 3.3 were not run; they rest on
tmux's `CHANGES` ("CHANGES FROM 3.0a TO 3.1": "modify source-file to
support "-" for standard input"), which is below the 3.2 floor in
`MIN_VERSION`. `smoke_test` takes the same path, so a tmux that cannot
read stdin skips the integration tests rather than failing them.

## Caveat

A server started by an older build keeps its leaked argv until it exits,
which only killing it (and every session on it) brings about. Values
that appeared in it should be rotated.
