"""Copy a Claude Code transcript up to (not including) the Nth typed prompt,
under a new session id, beside the original.

Usage: fork.py <src.jsonl> <new-id> <prompt-index, 1-based>
"""
import json
import os
import sys

src, new_id, n = sys.argv[1], sys.argv[2], int(sys.argv[3])
old_id = None
out = []
seen = 0
for line in open(src):
    try:
        o = json.loads(line)
    except json.JSONDecodeError:
        continue  # the last line may be half-written
    old_id = old_id or o.get("sessionId")
    msg = o.get("message") or {}
    typed = o.get("type") == "user" and not o.get("isSidechain") and isinstance(msg.get("content"), str)
    if typed:
        seen += 1
        if seen == n:
            break
    out.append(line.replace(old_id, new_id) if old_id else line)
dst = os.path.join(os.path.dirname(src), new_id + ".jsonl")
with open(dst, "w") as f:
    f.write("".join(out))
os.chmod(dst, 0o600)
print(dst, len(out), "lines; stopped before prompt", n)
