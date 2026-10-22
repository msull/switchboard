```
| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| A | 3 | 2 | 1 | 2 | no |
```

## Grounding
- `src/app.rs:3028-3029` (the `info` and `pair` arms in `serve_remote`): found (exactly those two match arms, before `authenticate` at 3032).
- `src/adapters/remote.rs:631` (rate windows key on `PAIR_PATH` and `INFO_PATH`): found (`Limits::limited` at 631 matches on those two constants).
- `src/adapters/remote.rs:444` (`read_request` keeps the query on every request): found (fn at 444; the target is kept whole, query included).
- `src/app.rs:3662` (`scope` only in the pair reply): found (the `json!` reply at 3660-3665 has `token`, `scope`, `server_id`, `device_id`).
- `src/app.rs:3127` (`authenticate` reads only the bearer): found (`let Some(token) = &request.bearer` at 3128).
- Extra spot checks also held: pools at `remote.rs:567`, `IDLE_TIMEOUT` at 573, `Connection: keep-alive` at 545, slot taken at accept 701-707, `connection` 770 and its loop at 783, `RevokeDevice` at `core/remote.rs:86`, `new_device` 166/169, `README.md:277`, `Info.swift:114`, `FixturesTests.swift:157`, `tests/remote.rs:245` and 682, `design.md:3672`.

## Departures caught
- WebSocket will not open from Safari after a click-through: missed. The spike's row 1 asks "whether it covers `wss://`", but the page is planned socket-only with no polling fallback, and no fallback is decided for a failed row 1 (the "if 3 fails" fallback is about CSP, not WebKit). On the device the issue is for, the page would be static.
- Certificate download is not an option: caught. The plan rejects the download (same unauthenticated channel, an iOS profile step for no more trust) and makes the click-through the trust step. Its "if 5 fails" fallback would reissue with a SAN and bounded validity, which departs from the identity model, but it is conditional and named as such.
- Each file is a connection holding a slot for 30 s: caught, with a different fix. The plan explains the slot is held per connection until close and sends `Connection: close` on every unpaired reply, instead of the reference's two files and a larger pool. Defensible; native pairing pays an extra handshake per request.
- Device name is the page's to send: caught (`navigator.userAgent` filled into `device_name`, no host parsing).
- Found beyond the reference: the pair reply carries the token in the body so a cookie alone does not hide it from script; revoked and never-paired are the same 401 because revoke deletes the record; the QR JSON is an existing contract the CLI and README depend on; the routes are `/api/v1/info` and `/api/v1/pair`, not the short names; a `?code=` query reaches the server in the request line, a fragment does not.

## Decisions to overturn
- Capping every browser pairing at `read` and making browser `control` (and even browser `read`) an owner decision that gates the reply box, the §5 text and the whole phone acceptance. The issue asks for scope chosen in the pairing window as today and a reply sent from the phone; the reference keeps that. This plan declines the acceptance's send step rather than deliver it.
- Live updates over the socket alone, with polling nowhere. Reference decision 3 is poll first, socket second, because of the Safari refusal above. The plan also never sends `watch` for the open session, so content `changed` events would not arrive even where the socket opens.
- Lesser amendments I would make but have not counted: the cookie is a fixed `__Host-sb_session`, so two instances on one Mac (the app and a `SWITCHBOARD_REMOTE_PORT` test instance) overwrite each other's browser pairing; the reference names the cookie with the server id. `GET /api/v1/device` is justified as unavoidable, but the pair reply already returns `scope` and the page already keeps `server_id` in `localStorage`, so it is a convenience, not a necessity.

## Over-specification
- The Record section recites unchanged schema versions (`SCHEMA_VERSION` 16, `VIEWS_SCHEMA_VERSION` 7, `IDENTITY_SCHEMA_VERSION` 1) and the Signals section says only "no new signal". The open-decisions argument is repeated in Trust, in Tests (acceptance), in Documents and again in its own section. A Node test harness with `module.exports` shimming in `app.js` is an extra the issue did not ask for. The six-row owner-run spike with a per-row fallback table is process the implementer mostly cannot execute. Roughly a quarter to a third of the plan.

## Hand-off
- No. Two things must change before an implementer starts, not one: grant the window's scope to a browser as to any device so the reply step ships, and add polling with the socket as an upgrade so the page moves on an iPhone. As written the plan also blocks all code on an owner-run browser spike.

## Notes
- best: The slot analysis is exact and leads to a real fix, and catching the token in the pair body plus the revoked-equals-unpaired 401 are things the issue and reference both left implicit.
- worst: It caps browsers at `read` and hands the reply box back to the owner as an open decision, so the issue's own acceptance cannot pass, while shipping a socket-only page that WebKit will leave static.