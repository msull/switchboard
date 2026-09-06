#!/usr/bin/env python3
"""(Run from an already-trusted cwd, argv[2]; hooks come from --settings so no
trust prompt or global-config write is needed.)
Drive an interactive `claude` in a pty to provoke permission / question /
idle paths, while recording the raw bytes it writes (to look for BEL, OSC 9
notifications, OSC 133) and watching hooks.log for the hook events.
Usage: drive-interactive.py <spike-dir>
"""
import os, pty, select, sys, time, json, signal

spike = sys.argv[1]
cwd = sys.argv[2] if len(sys.argv) > 2 else spike   # run from a trusted dir
os.chdir(cwd)
log = os.path.join(spike, "hooks.log")
raw_path = os.path.join(spike, "interactive.raw")
if os.path.exists(log): os.remove(log)

pid, fd = pty.fork()
if pid == 0:
    for k in [k for k in os.environ if k.startswith("CLAUDE")]:
        del os.environ[k]   # this spike is driven from inside a Claude session; don't inherit its markers
    os.environ["TERM"] = "xterm-ghostty"
    os.environ["COLUMNS"] = "120"; os.environ["LINES"] = "40"
    os.execvp("claude", ["claude", "--model", "haiku", "--settings", os.path.join(spike, "settings-abs.json")])

raw = open(raw_path, "wb")
buf = b""
T0 = time.time()
import re
SEQ = re.compile(rb'\x1b\](?:0|2|9|777);[^\x07\x1b]{0,120}|\x07')
last_title = None
def timeline(d):
    global last_title
    for m in SEQ.finditer(d):
        x = m.group(0)
        if x == b"\x07":
            print(f"  [{time.time()-T0:6.1f}s] BEL", flush=True); continue
        if x.startswith(b"\x1b]0;") or x.startswith(b"\x1b]2;"):
            if x == last_title: continue
            last_title = x
        print(f"  [{time.time()-T0:6.1f}s] {x.decode('utf-8','replace')!r}", flush=True)
def pump(secs):
    global buf
    end = time.time() + secs
    while time.time() < end:
        r, _, _ = select.select([fd], [], [], 0.2)
        if fd in r:
            try:
                d = os.read(fd, 65536)
            except OSError:
                return
            raw.write(d); raw.flush(); buf += d; timeline(d)

def events():
    if not os.path.exists(log): return []
    out = []
    for line in open(log):
        try:
            j = json.loads(line)
            out.append((j.get("hook_event_name"), j.get("notification_type") or j.get("tool_name") or j.get("reason") or j.get("source")))
        except Exception: pass
    return out

def wait_for(pred, secs, label):
    t0 = time.time()
    while time.time() - t0 < secs:
        pump(1)
        if pred(events()):
            print(f"[{time.time()-t0:5.1f}s] {label}: {events()}", flush=True); return True
    print(f"[{secs}s] TIMEOUT waiting for {label}: {events()}", flush=True); return False

def send(s):
    os.write(fd, s.encode()); pump(0.5)

wait_for(lambda e: ("SessionStart", "startup") in e, 30, "SessionStart")
pump(4)  # let the TUI settle (trust dialog etc.)
print("screen tail:", repr(buf[-400:]), flush=True)

# Phase 1: permission prompt
buf = b""
send("Use the Bash tool to run exactly: touch /Users/sully/code_repos/personal/switchboard/spikes/03-session-state/spike-perm-test.txt"); send("\r")
wait_for(lambda e: any(x[0] in ("PermissionRequest",) or x == ("Notification","permission_prompt") for x in e), 60, "permission prompt")
pump(2)
print("BEL count while waiting on permission:", buf.count(b"\x07"), " OSC9:", buf.count(b"\x1b]9;"), " OSC777:", buf.count(b"\x1b]777;"), flush=True)
os.write(fd, b"\r")  # accept default 'Yes'
wait_for(lambda e: any(x[0]=="Stop" for x in e), 60, "Stop after approval")

# Phase 2: question via AskUserQuestion
n_before = len(events())
buf = b""
send("Use the AskUserQuestion tool to ask me whether I prefer tea or coffee. Do nothing else."); send("\r")
wait_for(lambda e: any(x[0] in ("PermissionRequest","Notification") or (x[0]=="PreToolUse" and x[1]=="AskUserQuestion") for x in e[n_before:]), 60, "question")
pump(3)
print("BEL count while question pending:", buf.count(b"\x07"), " OSC9:", buf.count(b"\x1b]9;"), flush=True)
os.write(fd, b"\r")  # choose first option
wait_for(lambda e: sum(1 for x in e if x[0]=="Stop") >= 2, 60, "Stop after answer")

# Phase 3: idle at prompt -> idle_prompt notification (docs: after ~60s idle)
buf = b""
wait_for(lambda e: ("Notification","idle_prompt") in e, 90, "idle_prompt")
print("BEL count during idle:", buf.count(b"\x07"), " OSC9:", buf.count(b"\x1b]9;"), flush=True)

# Exit
send("/exit"); send("\r")
wait_for(lambda e: any(x[0]=="SessionEnd" for x in e), 20, "SessionEnd")
try: os.kill(pid, signal.SIGTERM)
except Exception: pass
raw.close()
print("FINAL EVENTS:"); [print("  ", e) for e in events()]
