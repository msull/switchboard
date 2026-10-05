# Spike 12: frames while the window is hidden

Question: does the frame loop keep running while the main window is
occluded or minimized? Issue #37 saw the control port go silent after
the Mac slept, and recover only now and then.

Mechanism, read from the crates: the wakes do reach winit (eframe's
repaint callback sends `UserEvent::RequestRepaint`; winit 0.30 delivers
`RedrawRequested` whether or not the window is occluded). While the root
window is occluded or minimized, eframe 0.36 runs no egui pass and calls
only `App::logic` (`eframe-0.36.1/src/native/wgpu_integration.rs`, the
`!show_ui` branch). Switchboard did all its work in `ui`, so a hidden
window answered nothing until it was uncovered. Display sleep and the
lock screen occlude every window, which is how sleep reached it.

Measured on 2026-10-04, macOS 15 (Darwin 24.6), Apple silicon, debug
builds of the commit before the fix and of the fix.

## Setup

```sh
mkdir -p /tmp/sbh
SWITCHBOARD_DATA_DIR=/tmp/sbh SWITCHBOARD_TMUX_SOCKET=switchboard-test-hidden \
  ./target/debug/switchboard &
```

The probe sends one `projects` query and times the reply:

```python
import socket, sys, time
s = socket.socket(socket.AF_UNIX); s.settimeout(10); s.connect(sys.argv[1])
t = time.monotonic()
s.sendall(b'{"op":"%s","kind":"projects"}\n' % sys.argv[2].encode())
buf = b''
try:
    while not buf.endswith(b'\n'):
        c = s.recv(4096)
        if not c: break
        buf += c
    print("reply in %.0f ms" % ((time.monotonic() - t) * 1000))
except socket.timeout:
    print("TIMEOUT after %.0f s" % (time.monotonic() - t))
```

The app is hidden (the same occlusion as Cmd+H; no keystrokes) with

```sh
osascript -e 'tell application "System Events" to set visible of (first process whose unix id is <pid>) to false'
```

and shown again with `true`.

## (a) Before the fix

| State | Reply |
| --- | --- |
| window shown | 1 ms |
| app hidden, 2 s later | TIMEOUT after 10 s |
| shown again | 0 ms |

The freeze is real and needs no sleep; uncovering the window ends it.

## (b) With the tick and `pump` in `App::logic`

| State | Reply |
| --- | --- |
| window shown | 1 ms |
| app hidden, five queries 0.37 s apart | 75, 103, 103, 103, 102 ms |

About 100 ms is eframe's repaint throttle for invisible windows
(`eframe-0.36.1/src/native/run.rs`); a reply never waits for the 1 s
tick because the control socket's wake requests a repaint. Commands
(`space.new`, `project.add`, a shell `session.new`, `session.waiting`)
were answered the same way while hidden.

## Not measured

- **The Dock badge while hidden.** An unbundled debug binary's badge
  could not be read back: `lsappinfo` reported a fixed `StatusLabel`
  that did not follow `session.waiting` on and off. Check it by eye on
  the bundled app: hide it, inject `echo '{"tool_name":"AskUserQuestion"}'
  | switchboard-hook PermissionRequest` for a session, and watch the Dock.
- **The sleep and wake cycle.** Sleeping the machine was left to the
  user. Steps: window unfocused and covered, `pmset sleepnow` (or lock
  the screen), wake, run the probe, inject a hook event, and record the
  reply time and that the badge moved before the window was uncovered.
  The mechanism says it matches (b): sleep only occludes the window.
