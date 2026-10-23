# Response to review round 1

Every citation in the feedback was re-checked against the worktree at
7fe8949 and holds. All four points changed the plan at
`review/1/repo/plan.md`; the earlier copy under `plan/1` is untouched.

## 1. `PairingInvite.parse` and `Address` (Decision 3)

Accepted. The dependency runs Transport → Protocol
(`client/Package.swift:18-19`), so `parse` cannot reach `Address`, and
`Address.init?` strips brackets rather than adding them.

Changed: **The actions and effects → Decision 3**, the
`PairingInvite.parse` bullet. The authority is now taken as text, the
substring between `https://` and the first `/`, straight into
`addresses`, so the host's brackets around an IPv6 literal survive and
the later `Address.init?` reads them as it does for the JSON form.
`parse` uses nothing from Transport; the bullet says why, with
`HostClient.swift:5` and `Package.swift:18-19`. The three-argument CLI
path at `main.swift:112-118` is named as the one going through
`Address`, unchanged.

## 2. The stranger-pool count (Decision 4)

Accepted. Keep-alive plus the 30 s idle timeout keeps the three load
connections counted, and a socket opened before any authenticated
`fetch` was the fourth stranger with no slot spare.

Changed in three places:

- **Decision 4, "Pool and window"**: states the mechanics from the tree
  (`remote.rs:831-845`, 548, 573, 785) and the design count: three load
  connections, the first authenticated request is `GET /api/v1/waiting`
  over `fetch` and moves its connection to the paired pool, the
  WebSocket opens after that reply as the fourth stranger at worst. A
  speculative extra connection or a second phone can meet the drop at
  `remote.rs:699-705`, and the poll fallback covers a dropped socket.
- **What the page does**, steps 4 and 5: step 4 is now `/waiting` alone
  as the first authenticated request; step 5 opens the socket after
  that reply and before the `projects` and `sessions` snapshots, which
  are the §4.4 snapshot; `/waiting` is refetched if an event's revision
  passes its own.
- **The tests**, `a_page_load_fits_the_stranger_pool`: three
  connections each fetch one page file and are held open, a fourth does
  `GET /waiting` with the cookie and is moved, a fifth upgrades to the
  socket with the cookie and gets `hello` while the three are still
  held.

## 3. The excerpt departure (Where this departs)

Accepted on both counts: `docs/remote-client.md:201` already puts the
excerpt on the redaction path, and `last_message` exists on the hook
event (`src/ports/events.rs:24`, `src/bin/switchboard-hook.rs:37`) and
is discarded by the core, so only "on the record" at 207 is inaccurate.

Changed: **Where this departs from the issue**, first bullet. The reason
is now that no wire view carries the field yet and adding it is a wire
crate change plus masking on the frame, outside this cut; the claim
that it could not be redacted is gone. **Docs, in the same commit**: the
207 edit now reads "the excerpt is not yet on the wire; it will be a
redacted field of the session view, masked as the redaction list above
says", so 201 and 207 agree, and the design.md known-gaps bullet gains
the excerpt. The follow-up at the end already listed it.

## 4. Line references

Accepted; all three corrected in place:

- "Where this departs", third bullet: `305-308` → `307-308`.
- "What the page does", step 1: `remote-client.md:310` → `311`.
- "The tests", first bullet: `(467)` → `tests/remote.rs:464`, the test
  at 426.
- The `HostClient.swift:15-25` reference is gone with the rewrite under
  point 1; the file is now cited with its Transport path.
