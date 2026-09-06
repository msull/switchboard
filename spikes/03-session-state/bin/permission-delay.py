#!/usr/bin/env python3
"""Leave a permission prompt unanswered and time when Notification
(permission_prompt) and the OSC 777 desktop-notify sequence arrive."""
import os, pty, select, sys, time, json, signal, re
spike, cwd = sys.argv[1], sys.argv[2]
os.chdir(cwd); log = os.path.join(spike, "hooks.log")
if os.path.exists(log): os.remove(log)
pid, fd = pty.fork()
if pid == 0:
    for k in [k for k in os.environ if k.startswith("CLAUDE")]: del os.environ[k]
    os.environ["TERM"] = "xterm-ghostty"
    os.execvp("claude", ["claude", "--model", "haiku", "--settings", os.path.join(spike, "settings-abs.json")])
T0 = time.time(); seen = set()
def pump(s):
    e = time.time() + s
    while time.time() < e:
        r,_,_ = select.select([fd],[],[],0.2)
        if r:
            try: d = os.read(fd, 65536)
            except OSError: return
            for m in re.finditer(rb'\x1b\]777;[^\x07\x1b]*', d): print(f"  [{time.time()-T0:5.1f}s] {m.group(0)!r}", flush=True)
def ev():
    out = []
    if os.path.exists(log):
        for l in open(log):
            j = json.loads(l); out.append((j["hook_event_name"], j.get("notification_type") or j.get("tool_name")))
    return out
pump(5)
os.write(fd, f"Use the Bash tool to run exactly: touch {spike}/spike-perm-test.txt".encode()); pump(0.5); os.write(fd, b"\r")
t = time.time()
while time.time() - t < 75:
    pump(1)
    e = ev()
    for x in e:
        if x not in seen:
            seen.add(x); print(f"  [{time.time()-T0:5.1f}s] hook {x}", flush=True)
    if ("Notification","permission_prompt") in e: break
os.write(fd, b"\x1b"); pump(1)      # decline
os.write(fd, b"/exit\r"); pump(2)
try: os.kill(pid, signal.SIGTERM)
except Exception: pass
print("events:", ev())
