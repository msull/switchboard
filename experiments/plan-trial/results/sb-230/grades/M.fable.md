The fields exist. I have enough to score.

| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| M | 3 | 3 | 2 | 2 | note |

## Grounding
- `src/app.rs:3023-3025`: found (the `strip_prefix("/api/v1/")` arm and the `404 not_found` refusal in `serve_remote`, exactly those lines)
- `src/ui/cards.rs:176-184,558-574`: found (the caption branch choosing `agent_excerpt` for agents and the caption or host title otherwise, and `agent_excerpt` with its 160-character cut)
- `src/app.rs:3568-3600`: found (`remote_info`, unauthenticated envelope with `server_id`, `spki_sha256`, limits, `pairing_open`)
- `src/app.rs:3127-3153`: found (`authenticate`, both refusals carry the code `unauthorized`, as the plan says)
- `src/core/remote.rs:86-92`: found (`RevokeDevice` retains everything but the id and saves; nothing of the token survives)
- Spot checks beyond five: `remote.rs:362-363` (path without query), `:519-527` (`respond_upgrade`), `:530-552` (`respond` with the 30-ish literal reason codes), `:567` (`MAX_UNAUTHENTICATED`), `:770`, `:788`, `:797`, `app.rs:3762-3766` (the refusal log line), `model.rs:12`, `ports/remote.rs:17,96-107`, `remote-client.md:254,306,309,312,313,317,318`, `tests/ui.rs:7424,7486-7492`, `tests/remote.rs:245`, `README.md:276-279,383`, `main.swift:12,111`, `Info.swift:113-116`, `design.md:3573`, `ui/remote.rs:160-166`, `SessionView.last_stop_at_ms`/`last_activity_ms`. Every one lands where the plan says.

## Departures caught
- Safari will not open `wss://` behind a click-through: caught. The plan calls it the question the plan turns on most, and ships a polling fallback (5 s on projects and sessions, socket retried once a minute, content refetched only when `last_stop_at_ms` or `last_activity_ms` moves). It frames it as unresolved rather than as a known WebKit behaviour, but the page works without the socket, which is what matters.
- Certificate download is not an option: caught in effect. The plan picks the click-through with reasons, keeps the certificate untouched, and says what the key check does and does not buy. It does not say outright that the download path collapses.
- Each page file is a connection: caught with a different fix. Three files rather than two, but every static reply carries `Connection: close` so a slot is freed as the bytes are written, with `MAX_UNAUTHENTICATED` 4 to 8 named as the fallback if the device run shows drops. That is a fair answer to the pool concern.
- Device name is the page's to send: caught. `device_name` from `navigator.userAgent`, with the iPad-reporting-as-Mac touch-point check.
- Errata: the card excerpt is not on the wire: caught, with the right reasoning (content on a metadata view skips redaction), handled by `last-response` for the first 8 waiting Claude Code rows and the watch-set cost counted.
- Errata: scope after reload: caught, adds `GET /api/v1/device`.
- Errata: invite carries a LAN address while listening on loopback: only half caught. It falls back to `127.0.0.1` when `local_address` is empty but does not consider a LAN address in the URL for a loopback-only listen.
- Beyond the reference: the 404 for a non-API path is decided on the frame after taking an inbox slot; `authenticate` cannot distinguish revoked from never paired, so the page infers it; the URL-versus-`/info` key check proves only what `/info` claims; `replaceState` covers the session entry only and the single-use code is the real guarantee. All verified against the tree.

## Decisions to overturn
- Token in `localStorage` sent as a `Sec-WebSocket-Protocol` value. The reference picks an `HttpOnly; Secure; SameSite=Strict` cookie named with the server id and calls `localStorage` the call to reverse. The plan's case is coherent (no CSRF surface, no `Origin` check, two instances isolated by origin, no reliance on `Secure` cookies behind a click-through), so I weigh it as one reversal rather than a defect. Everything else matches the reference: click-through trust, one URL QR with `invite_url` in the core, static files on the connection thread, CSP without inline script, no new dependency, the `watch` on the open view.

## Over-specification
- Roughly a fifth to a quarter. The WHATWG, CSP Level 3 and WebKit bug citations with revision numbers, the `WindowLocalStorage` IDL argument, the repeated "this rests on the For the owner line …" tags, and the per-sentence list of spec lines to amend are more than an implementer needs. The 8-excerpt budget with its 9-of-128 watch-set accounting is also more mechanism than the first cut warrants, though it is at least justified.

## Hand-off
- Note: swap the token transport for the cookie the owner prefers (`read_request` fills `bearer` from a `SameSite=Strict` cookie named with the server id), keep the rest as written.

## Notes
- best: every line reference is exact, and it found four real departures the reference lacks (frame-side 404 costs an inbox slot, revoked is indistinguishable from never paired, the key check only trusts `/info`, `replaceState` is tidying not a guarantee) and handled each.
- worst: it treats the Safari `wss://` refusal as an open question to be settled at acceptance rather than the known behaviour it is, so the fallback path is built as insurance instead of the primary path, and the token lands in `localStorage` against the owner's call.