```
| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| Q | 3 | 3 | 1 | 2 | note |
```

## Grounding
- `control/src/lib.rs:637` (`SessionView`, no excerpt field): found (struct starts at 637 with `card`, `waiting`, `waiting_reason`; no `last_message`)
- `docs/remote-client.md` §4.3 `session.last_response` bullet calling `last_message` "a field of the session's metadata": found (line 207, exactly as described)
- `src/app.rs:4105` (`caption_of`): found (fn at 4105, the card's caption from the last non-empty pane line)
- `src/app.rs:3730` (envelope carries only `epoch`, `revision`, `body`): found (`envelope` at 3730 builds exactly those three)
- `src/app.rs:3127` (`authenticate` reads only `request.bearer`): found (3127, bearer-only, dispatches `DeviceSeen` at 3150)
- Also checked: `src/core/remote.rs:72-84` `DeviceSeen`/`:86` `RevokeDevice`, `app.rs:3632-3639` `wrong_key`, `:3650` `device_name`, `:3754` `refuse`, `:486` revoke drops the queue, `:1704` `subscriber_frame`, `adapters/remote.rs:623` `Limits`, `:714` the only log line, `:829` 504 literal, `:835-848` pool move, `tests/remote.rs:464` `/other` 404, `ui/remote.rs:141`/`:229`, `Info.swift:113`, `main.swift:108`, README `:277`, `design.md:3107`, `spikes/22-remote-port`. All correct.

## Departures caught
- Safari refuses `wss://` on a click-through certificate: caught, though framed as "unresolved" rather than known; the plan builds a poll fallback (socket first, poll after two silent attempts) so the page works either way.
- Certificate download is not an option: caught (no SAN/EKU/825-day rule, a profile install per device; option B rejected).
- Each page file is an unauthenticated connection: caught, with a different fix: `Connection: close` on static replies plus a `page` rate window, three files instead of the reference's two. The close-on-static fix is sound and arguably better than counting files.
- Device name is the page's to send: caught (departure 7, `navigator.userAgent` plus a touch-points heuristic for iPad).
- Beyond the reference: the excerpt is not on the wire (departure 1); the page cannot learn its scope after a reload behind a cookie, fixed by `GET /api/v1/device` (departure 2); the pair reply would otherwise carry the token in the body, so the cookie pairing omits it; revoked and unknown tokens are the same 401 with no tombstone (departure 3); `SameSite` ignores ports so another port on the same IP is same-site; `Referer` would leak the code to the subresource requests, fixed with `Referrer-Policy: no-referrer`. Not found: the invite carries a LAN address while the host may listen on loopback.

## Decisions to overturn
- Skipping `DeviceSeen` for every cookie-authenticated `GET` without `Origin`, so "seen" never moves in poll mode. The reference holds `SameSite=Strict` plus an `Origin` check as sufficient; a cross-port `GET` that only bumps `last_seen` is not a threat worth a stale dialog.
- The page posts the code without first comparing the URL's key against `/info`. The reference and the issue compare first so a wrong genuine host never sees the code; `/info` does expose `spki_sha256` (`app.rs:3581`), so the check costs one request. Low stakes since the code is bound to the key, but I would restore it.
- Minor, not counted: "Copy invite" stays JSON with a second "Copy link" button, where the reference makes the one invite the URL and changes the UI test at 7474.

## Over-specification
- The `facts:NN` citations point at a file not in the tree, so an implementer cannot follow them; the repeated "owner question N is open, here is the branch not built" passages, the commit-message section with the attribution reminder, the spike README of all-unrun device checks, and the record section that says nothing changes. Roughly a quarter of the plan.

## Hand-off
- Drop the `DeviceSeen` skip and rely on `SameSite=Strict` plus the `Origin` rule; have the page check the URL key against `/info` before posting the code; make Copy invite copy the URL.

## Notes
- best: Every cited line is real, and it found the excerpt, scope-after-reload, token-in-body and `Referer` leaks on its own, each with a concrete fix.
- worst: The cookie `GET` without `Origin` rule adds a stale "seen" column to defend against a same-IP-other-port attacker that `SameSite=Strict` already covers well enough.