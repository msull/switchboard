"""Spike 11 item 2: which probe sees a dev server's port as taken.

Binds the way Rust's std::net::TcpListener::bind does on Unix
(SO_REUSEADDR on), and connects the way a readiness probe does.
"""
import socket
import subprocess
import sys
import time

PORT = 3157


def bind(family, host):
    s = socket.socket(family, socket.SOCK_STREAM)
    s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    try:
        s.bind((host, PORT))
        s.listen(1)
        return "bound"
    except OSError as e:
        return f"refused ({e.errno})"
    finally:
        s.close()


def connect(family, host):
    s = socket.socket(family, socket.SOCK_STREAM)
    s.settimeout(0.2)
    try:
        s.connect((host, PORT))
        return "answered"
    except OSError:
        return "nothing"
    finally:
        s.close()


SERVERS = {
    "node, no host (dual stack *)": None,
    "node HOST=0.0.0.0": "0.0.0.0",
    "node HOST=localhost": "localhost",
    "node HOST=127.0.0.1": "127.0.0.1",
    "node HOST=::1": "::1",
}

for name, host in SERVERS.items():
    env = {"PORT": str(PORT), "PATH": "/opt/homebrew/bin:/usr/bin:/bin"}
    if host:
        env["HOST"] = host
    p = subprocess.Popen(
        ["node", "-e", "require('http').createServer((q,s)=>s.end('ok')).listen(process.env.PORT, process.env.HOST || undefined)"],
        env=env,
    )
    time.sleep(0.6)
    row = [
        ("bind 127.0.0.1", bind(socket.AF_INET, "127.0.0.1")),
        ("bind ::1", bind(socket.AF_INET6, "::1")),
        ("bind 0.0.0.0", bind(socket.AF_INET, "0.0.0.0")),
        ("connect 127.0.0.1", connect(socket.AF_INET, "127.0.0.1")),
        ("connect ::1", connect(socket.AF_INET6, "::1")),
    ]
    print(name)
    for k, v in row:
        print(f"  {k:18} {v}")
    p.terminate()
    p.wait()
    time.sleep(0.2)

print("nothing listening")
print(f"  bind 127.0.0.1     {bind(socket.AF_INET, '127.0.0.1')}")
print(f"  bind ::1           {bind(socket.AF_INET6, '::1')}")
print(f"  connect 127.0.0.1  {connect(socket.AF_INET, '127.0.0.1')}")

# Item 3: a probe against a server not up yet, timed.
t0 = time.monotonic()
r = connect(socket.AF_INET, "127.0.0.1")
r6 = connect(socket.AF_INET6, "::1")
print(f"probe of a closed port: {r}/{r6} in {(time.monotonic()-t0)*1000:.1f} ms")
sys.exit(0)
