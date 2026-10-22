| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| R | 3 | 2 | 1 | 2 | note |

## Grounding
- `control/src/lib.rs:637-666`: found (`SessionView` with `card` at 646, `waiting_reason` at 658, no `last_message`)
- `src/adapters/hooks.rs:71`: found (`last_message: Option<String>` on the hook line)
- `src/ports/events.rs:24`: found (`Stopped { last_message, .. }`)
- `src/app.rs:3568-3600`: found (`remote_info`; no per-device scope in the body)
- `src/app.rs:3650`: found (`device_name` read at 3650, scope only in the `/pair` reply at 3662)
- Spot-checked beyond five: `core/remote.rs:169` (64-char take), `adapters/remote.rs:770/788/623-631/437/578/444/519/835-848/1045/1064`, `tests/remote.rs:245/406/426/682/1960`, `Info.swift:113-114`, `ui/remote.rs:141/242/280`, `design.md:3107/3339/3571/3585`, `README:276-278`, `FixturesTests.swift:161`: all found and relevant.

## Departures caught
- Safari refuses `wss://` behind a click-through certificate: missed. The plan flags the click-through itself as Unresolved but builds live updates on the socket alone, with only the spec's 30-second content refetch; the session list has no polling path, so "moves to the top without a reload" fails on the device the page is for.
- Certificate download not an option with this certificate: partly caught. The plan says download would need a reissue with a SAN and does not build it, but keeps it as a viable later fallback rather than saying the spec's identity model rules it out.
- Each page file is an unauthenticated connection: caught. Counts static replies in the 4-slot pool, mitigates with `Connection: close`, adds a `page` rate window, flags parallel-connection behaviour as untested. Keeps three files instead of two.
- Device name is the page's to send: caught. `navigator.userAgent` filled into `device_name` by the script, no host-side parsing.
- Beyond the reference: the excerpt is not on the wire (shows `card` and `waiting_reason`); scope is unknown after a reload (adds `GET /api/v1/me`, also covers scope changed in the dialog); the 64-char cap sits in `new_device` not `/pair`; no `watch` is sent for the open session's content, which the plan also does not address.

## Decisions to overturn
- Token in `localStorage` plus `Sec-WebSocket-Protocol`. Reference: `HttpOnly; Secure; SameSite=Strict` cookie filled into `bearer` by `read_request`, which also leaves `respond_upgrade` and `authenticate` untouched. The plan's CSP-plus-`textContent` discipline is a premise it admits is not a fact.
- Socket-only live updates. Reference: poll the collections and the open session on a short interval, use the socket when it opens, and send `watch` for `conversation`/`screen` so `changed` for content arrives at all. The plan refetches on `changed` without a `watch`.
- Not reversed: `GET /me` (the issue's "cannot do without one" is met), the `page` rate window, the connection-thread static branch (matches the reference).

## Over-specification
- The `Host`-header parsing and regex for `connect-src wss://<host>` (reference: `connect-src 'self'`), the banned-call lists for `app.js`, control-character stripping in `new_device`, the Node test file that CI never runs, and the commit-body contract list. Roughly a fifth of the plan.

## Hand-off
- Safari will not open `wss://` behind a clicked-through certificate, so poll the lists and the open session on an interval with the socket as a bonus, and keep the token in a `SameSite=Strict` cookie rather than `localStorage`.

## Notes
- best: every reference checks out, and it found the excerpt-not-on-the-wire and scope-after-reload gaps and gave each a concrete, small fix.
- worst: it assumes the events socket opens in a browser and stores the token in `localStorage`, so the page is static on an iPhone and the credential is script-readable.