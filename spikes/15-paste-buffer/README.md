# Spike 15: a multi-line send as one paste

**Question.** `write_line` typed a message with `send-keys -H`, so each
LF reached Claude Code as typed input and a blank line submitted the
part before it. Can a multi-line message go to a pane as one paste,
bracketed in `ESC[200~` .. `ESC[201~` only when the pane asked for
bracketed paste (DECSET 2004), so a plain process never sees the
markers? What does tmux do to CR and ESC inside the text?

**Setup.** tmux 3.7c on a `switchboard-test-spike15` socket, macOS. Two
panes run `cat -u` in raw mode into a file, so every byte the pane
receives is kept: `b` enables 2004 first, `p` does not.

```sh
S=switchboard-test-spike15
D=$PWD/sp
mkdir -p "$D"; rm -f "$D"/*.out
tmux -L $S -f /dev/null new-session -d -s b -x 80 -y 24 "/bin/sh -c \"printf '\\033[?2004h'; stty raw -echo; cat -u > $D/b.out\""
tmux -L $S new-session -d -s p -x 80 -y 24 "/bin/sh -c \"stty raw -echo; cat -u > $D/p.out\""
sleep 0.5
echo "flag b: $(tmux -L $S display -p -t =b: '#{bracket_paste_flag}')  flag p: $(tmux -L $S display -p -t =p: '#{bracket_paste_flag}')"
for t in b p; do
  printf 'one\n\ntwo\r\nthree\033[201~x' | tmux -L $S load-buffer -b sb-$t -
  tmux -L $S paste-buffer -p -r -d -b sb-$t -t =$t:
done
sleep 0.5
echo "buffers after -d: [$(tmux -L $S list-buffers)]"
for t in b p; do echo "$t (paste -p -r):"; od -c $D/$t.out; done
: > $D/p.out
printf 'a\nb' | tmux -L $S load-buffer -b nr -
tmux -L $S paste-buffer -p -d -b nr -t =p:
sleep 0.3
echo "p without -r:"; od -c $D/p.out
: > $D/p.out
tmux -L $S send-keys -t =p: -H 1b 5b 32 30 30 7e 41
sleep 0.3
echo "p after send-keys -H of ESC[200~A:"; od -c $D/p.out
tmux -L $S kill-server
```

Output (the NULs are `cat`'s file offset after the file was truncated
under it, not bytes the pane got):

```
flag b: 1  flag p: 0
buffers after -d: []
b (paste -p -r):
0000000  033   [   2   0   0   ~   o   n   e  \n  \n   t   w   o  \r  \n
0000020    t   h   r   e   e   ^   [   [   2   0   1   ~   x 033   [   2
0000040    0   1   ~                                                    
0000043
p (paste -p -r):
0000000    o   n   e  \n  \n   t   w   o  \r  \n   t   h   r   e   e   ^
0000020    [   [   2   0   1   ~   x                                    
0000027
p without -r:
0000000   \0  \0  \0  \0  \0  \0  \0  \0  \0  \0  \0  \0  \0  \0  \0  \0
0000020   \0  \0  \0  \0  \0  \0  \0   a  \r   b                        
0000032
p after send-keys -H of ESC[200~A:
0000000   \0  \0  \0  \0  \0  \0  \0  \0  \0  \0  \0  \0  \0  \0  \0  \0
0000020   \0  \0  \0  \0  \0  \0  \0  \0  \0  \0 033   [   2   0   0   ~
0000040    A                                                            
0000041
```

## Findings

- `load-buffer -` takes the text on stdin, so a message never goes on a
  command line and has no argument limit.
- `paste-buffer -p` frames the text in `ESC[200~` .. `ESC[201~` for a
  pane whose `#{bracket_paste_flag}` is 1 and delivers it bare to one
  whose flag is 0.
- `-r` keeps LF. Without it tmux turns each LF into CR, which a plain
  process reads as Enter per line.
- `-d` deletes the buffer after the paste; `list-buffers` is empty.
- CR in the text passes through unchanged. ESC is not passed raw, but
  tmux 3.7c writes it as the two characters `^[`, so `ESC[201~` in the
  text arrives as `^[[201~`: it cannot close the bracket early, but it
  would show as junk in a prompt.
- `send-keys -H` delivers bracket markers to every pane, including one
  that never enabled 2004, which then sees `ESC[200~` as input.
- Claude Code turns on 2004: a three-paragraph message sent this way is
  submitted once and whole, and the transcript records it inside
  `<pasted_content>` tags, as it does any paste
  (`multi_paragraph_send_submits_once` in `tests/gate.rs`, Claude Code
  v2.1.293).

## Recommendation

Send text with a line break as `load-buffer -b sb-send-<host id> -`
then `paste-buffer -p -r -d`, after turning CRLF and lone CR into LF and
removing ESC; send Enter as a separate write after the paste returns.
Text on one line stays typed with `send-keys -H`. Every flag used is
older than tmux 3.2, the minimum.
