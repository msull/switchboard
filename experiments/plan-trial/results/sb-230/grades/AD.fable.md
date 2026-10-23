| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| AD | 3 | 3 | 2 | 2 | note |

## Grounding
- `control/src/lib.rs:637-688` (`SessionView`, no excerpt field): found (struct spans 637-688 with `card`, `waiting_reason`, `last_stop_at_ms`, `pending`; no `last_message`)
- `src/ports/events.rs:24` (`last_message` on hook events): found (`Stopped { last_message: Option<String>, .. }`)
- `src/adapters/hooks.rs:71` (`last_message` on the hook line): found (field on the hook JSON struct)
- `docs/remote-client.md:207` (spec sentence describing the excerpt field): found (the `session.last_response` bullet ends with "that is `last_message` from the `Stop` hook on the record")
- `src/app.rs:3658-3663` (`remote_pair` replies with scope): found (the `json!({token, scope, server_id, device_id})` reply)
- Also checked: `remote.rs:102` SAN-less params, `bind_remote` at 2936, `short_hash` at 303, `IDENTITY_SCHEMA_VERSION` at `ports/remote.rs:17`, `RevokeDevice` at 486-498, `refuse` at 3754, allowlist test 690-700, 28 `RemoteReply {` sites, README line 18 "81 tests", spike 22 as last. All exact.

## Departures caught
- Safari refuses `wss://` after click-through: caught in substance. Poll-first on `waiting` every 5 s with revision gating, socket as upgrade, "live updates unavailable" text. Stated as unresolved rather than as fact, but the design is right.
- Certificate download is not an option: caught with a wrong fix. The plan takes the click-through as the trust step and defers the download, but claims the certificate "cannot satisfy a browser as made" and adds IP/localhost SANs via a reissue. A click-through needs no SAN; the SAN work serves only the deferred profile route.
- Each file is a connection: caught. Three files served on the connection thread with `Connection: close`, a page rate window, and the pool math stated. Reference wants two files; the plan's `Connection: close` mitigation addresses the same problem.
- Device name is the page's to send: caught (`navigator.userAgent` plus `maxTouchPoints` into `device_name`).
- Extra, not in the reference body: excerpt not on the wire (shows `card` and `waiting_reason`, excerpt deferred); no reader of scope after reload (adds `GET /api/v1/me`); invite offers a LAN address while listening on loopback (UI line); pair reply would carry the token in the body alongside a cookie (omits `token` under `carrier: cookie`); `Referer` leaks the `?code=` URL to subresources (`Referrer-Policy: no-referrer`). All verified.

## Decisions to overturn
- Reissue the certificate with IP and `localhost` SANs at bind, kept in `Identity.sans`, remade when the address changes, with an `IDENTITY_SCHEMA_VERSION` bump. Conflicts with reference decision 1 ("no certificate download, no reissue") and with the spec's identity model that binds nothing to an address. It also re-shows the browser warning on every address change, which the plan admits. Drop it entirely.
- Everything else matches the reference or is an acceptable variant: `HttpOnly; Secure; SameSite=Strict` cookie with `Origin` guard, bearer kept for native clients, `localStorage` rejected, one URL invite with `PairingInvite.parse` taking both forms, static files on the connection thread with the named headers, `web/` embedded via `include_str!`, no new dependency. `Path=/api/v1/` instead of `Path=/` is a defensible choice, not a reversal.

## Over-specification
- The SAN section and its `reissue` function, unit tests, schema bump and `Identity.sans` field (~10%) are unneeded. "The record" section mostly says what does not change. The "Contract additions" list repeats the body, and Facts citations sprinkle through the text without helping an implementer. The step-0 spike's "stop and report" gate is heavier than the repo's spike habit needs. Roughly a fifth could go.

## Hand-off
- Drop the SAN reissue and `Identity.sans`: the click-through needs no SAN, and the spec's identity must not bind to an address. Everything else can be built as written.

## Notes
- best: Found five real departures beyond the issue (no excerpt on the wire, no scope reader, token in the pair body, loopback invite, Referer leak) and fixed each cheaply, with every citation exact.
- worst: Invents a certificate reissue keyed to the Mac's address on a mistaken premise, dragging a schema bump and a recurring browser warning into a plan that otherwise needs neither.