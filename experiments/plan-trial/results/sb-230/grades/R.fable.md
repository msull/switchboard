| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| R | 3 | 2 | 1 | 2 | note |

## Grounding
- `control/src/lib.rs:637-666`: found (`SessionView` at 637, `card` at 646, `waiting_reason` at 658, no `last_message` field anywhere in the struct)
- `src/adapters/hooks.rs:71`: found (`last_message: Option<String>` on the hook log line struct)
- `src/ports/events.rs:24`: found (`Stopped { last_message: Option<String>, pending }`)
- `src/app.rs:3568-3600`: found (`remote_info`; the body has `server_id`, `server_name`, `spki_sha256`, `capabilities`, `limits`, `pairing_open`, no per-device scope)
- `src/app.rs:3650`: found (`device_name` read at 3650-3655; the scope is in the pair reply at 3662, same function, the plan's line is a dozen off but points at the right place)
- Later refs also checked and real: `src/core/remote.rs:169` (`new_device` trims and takes 64 chars), `remote.rs:623/631` (`Limits` with `pair`/`info` only), `:770/788/835-848` (`connection`, rate check first, pool move and 503 `full`), `:1045/1064` (both named unit tests), `tests/remote.rs:245/426/682/1960`, `tests/ui.rs:7474-7492` (the JSON assertions), `Info.swift:114`, `FixturesTests.swift:161`, `design.md:3339/3571/3585`, `README.md:276-278`, `ui/remote.rs:141/147-155/242/280/73-83`, `ConnectionManager.swift:371` (`deviceName`).

## Departures caught
- WebSocket will not open from Safari on a click-through certificate: missed. The plan lists "whether `fetch` and `wss://` work after the click-through" as an unknown in the design.md gaps, but the page is built around the socket: the list refreshes only on `changed`, with a 30-second refetch for the open content view and nothing for the list. On an iPhone the sessions list never updates without a reload.
- "Offer the certificate for download" is not an option: partly caught. The plan picks click-through and does not change the certificate, but keeps a SAN reissue and `GET /cert.der` as a written fallback rather than saying it does not help (and does not know Safari refuses `wss://` even then).
- Each file of the page is a connection: caught with a different fix. Three files, not two, but every static reply carries `Connection: close` so the unauthenticated slot is freed in milliseconds, and the plan says the slot is counted against the 4-pool while the connection lives. Acceptable.
- Device name is the page's to send: caught. The page fills `device_name` from `navigator.userAgent`; the plan also corrects the facts file's claim that there is no cap (found the 64-char take in `new_device`).
- Errata, the excerpt is not on the wire: caught, with the exact lines; shows `card` and `waiting_reason`.
- Errata, nothing tells a device its scope after a reload: caught; the plan adds `GET /api/v1/me` and argues the stored scope goes stale when the dialog changes it.
- Errata, invite carries a LAN address while the host listens on loopback: partly. The plan handles only the case where `local_address()` is `None` (falls back to `127.0.0.1:<port>`), not a LAN address found while the bind is loopback.
- Errata, token in the pair body even with a cookie: not applicable, the plan does not use a cookie.
- Found that the reference lacks: the per-request `Host` header must be kept for a `wss://` entry in `connect-src` (the plan treats this as a fact; on current browsers `'self'` already covers `wss:`, so this is extra plumbing rather than a real departure).

## Decisions to overturn
- Token in `localStorage` with `Sec-WebSocket-Protocol` for the upgrade. The reference takes an `HttpOnly; Secure; SameSite=Strict` cookie named with the server id and fills `bearer` from it in `read_request`, and names `localStorage` as the one call it would reverse. The plan's argument that cookies ignore ports is real, and the reference's server-id cookie name answers it; the plan's XSS defence is a CSP plus a grep for `innerHTML`. Reverse.
- Socket only, no polling of the collections. The reference's live-update decision is poll first, socket second, because of departure 1. Reverse.
- Not reversed but noted: `GET /api/v1/me` (the reference keeps scope from the pair reply; with a script-held token the page does see that reply, but scope changed in the dialog is a fair reason for an additive read); the third `page` rate window; three files with `Connection: close` rather than two.

## Over-specification
- The `Host`-derived `wss://<host>` CSP entry with a validation regex and a new `RemoteRequest.host` field: not needed on current browsers, about a tenth of the plan across signals, adapters and tests.
- `HEAD` plus 405 with `Allow`, `Referrer-Policy`, control-character stripping in `new_device` with its core test, the Node `app.test.js` suite that greps the source for `.style.` and `cssText` and runs outside CI: an implementer could drop all of it.
- Repeated "this rests on a line under For the owner / Unresolved" pointers to the facts file add words without guidance.
- Roughly a quarter of the plan.

## Hand-off
- Poll `/projects` and the open session's content on a short interval and treat `/api/v1/events` as an enhancement that is skipped when the socket fails to open; Safari refuses `wss://` to a click-through certificate, and the plan's own design.md gap already admits it never tried one.

## Notes
- best: Every departure it does catch is pinned to verified lines, including the wire's missing excerpt, the missing scope read, and a correction to the facts file's own claim about the device-name cap.
- worst: It builds the page around a WebSocket that the target device will not open, so the sessions list on an iPhone is static until a reload.