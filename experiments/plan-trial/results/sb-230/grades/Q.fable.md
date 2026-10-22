| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| Q | 3 | 3 | 1 | 2 | note |

## Grounding
- `control/src/lib.rs:637` (`SessionView`): found (struct at 637; no `last_message` or excerpt field anywhere in the crate)
- `docs/remote-client.md` §4.3 `session.last_response` bullet: found (line 207 says the excerpt is `last_message`, "a field of the session's metadata", which the wire crate never defines)
- `src/app.rs:4105` (`caption_of`): found (fn at 4105, computes the card caption host-side)
- `src/app.rs:3730` (envelope): found (`envelope` emits only `epoch`, `revision`, `body`)
- `src/app.rs:3127` (`authenticate`): found (reads `request.bearer` only, dispatches `DeviceSeen`)
- Spot checks beyond five: `app.rs:3632-3639` `wrong_key`, `3650` `device_name`, `486` revoke drops the queue, `1704-1735` watch frames, `remote.rs:444/519/530/567/623/631/714/770/835-848`, `ui/remote.rs:141/229`, `tests/remote.rs:464`, `Info.swift:113`, `main.swift:108`, `remote-client.md` 306-317 all match.

## Departures caught
- Safari refuses `wss://` after a click-through: caught (framed as unresolved, with a poll fallback after two 5 s failed handshakes).
- Certificate download is not an option with this certificate: caught (no SAN/EKU/825 days; option A chosen, reissue unbuilt).
- Each page file is a connection against the 4-slot pool: caught with a different fix (`Connection: close` on static replies, a `page` rate window, three files rather than two, pool not raised). Reasonable.
- Device name is the page's `device_name`, not a host-parsed `User-Agent`: caught, with the iPad `maxTouchPoints` heuristic.
- Errata: excerpt not on the wire: caught. Scope unknowable after reload behind a cookie: caught (`GET /api/v1/device`). Token in the pair body even with a cookie: caught (body drops `token` on `credential: "cookie"`). Invite address versus loopback listen: not called out; the plan only falls back to the JSON QR when `local_address()` is `None`.
- Found beyond the reference: revoked and unknown tokens are indistinguishable (no tombstone); the page cannot read the SPKI, so the "compare with `/info`" step is theatre and the host's `wrong_key` refusal is the real check (I agree, though the reference keeps the `/info` compare); no all-sessions read; "the host's own HTTP client" is the test helper, not a `src/` client; `Referer` leaks the query before `replaceState`.

## Decisions to overturn
- Socket first, snapshot only after `hello`, polling only after two 5 s timeouts and retried only on reload: on Safari, the device the page is for, every load is blank for ~10 s. The reference says poll first, socket second; I would snapshot at once and let the socket upgrade live updates.
- Skipping `DeviceSeen` for a cookie-authenticated `GET` without `Origin`: in poll mode (Safari again) "seen" never moves except on a send. The reference treats `SameSite=Strict` as the CSRF defence and an `Origin` check as optional; the same-site-other-port threat only bumps a timestamp. Drop the rule.
- Not reversed but noted against the reference: `Path=/api/v1` instead of `/`, "Copy invite" kept as JSON with a separate "Copy link", three static files, and a spike README whose every row says "not run".

## Over-specification
- The "Contract additions" section restates "Actions and effects"; the commit-message and order sections add little; the facts citations and the long trust prose in decision 1 and 2 repeat the facts file; the spike README with unrun rows is ceremony. Roughly a quarter of the plan.

## Hand-off
- Snapshot immediately and treat the socket as an upgrade, and drop the `DeviceSeen`-skip rule; the rest can go as written.

## Notes
- best: every departure grounded to a real line, including the excerpt, the scope read, the token-in-body and the unreadable SPKI, each with the branch it builds and the one it leaves.
- worst: the live-update order leaves Safari blank for ten seconds per load, and the Origin/`DeviceSeen` carve-out buys a stale "seen" for a threat that only touches a timestamp.