| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| E | 3 | 2 | 1 | 2 | note |

## Grounding
- `control/src/lib.rs:637-688` (`SessionView` has no `last_message` or caption field): found (struct spans 636-687; name, kind, card, waiting_reason, notes, no excerpt field)
- `docs/remote-client.md` §4.3 line 207 (says the excerpt is on the wire): found (line 207 is `session.last_response`, which says the card excerpt is `last_message` "a field of the session's metadata", which the view lacks)
- `src/app.rs:1093-1096` (`refresh_captions`): found (doc comment and `fn refresh_captions` at 1093-1096)
- `client/Sources/SwitchboardUI/BrowserView.swift:741-746` (the headset row): found (kind, card, waitingReason in an HStack at 740-746)
- `src/app.rs:3660-3665` (pair reply carries the scope): found (the `json!({token, scope, server_id, device_id})` at 3660-3665)
- Also checked: `ui/remote.rs:156-159` loopback address claim is real, `read_request` keeps only four headers at 481-494, `Limits::limited` at 631-638, pool move at 835-849, `Info.swift:113-116`, `main.swift:12` and `110-111`, `tests/remote.rs:1587` and `1829`, `core/tests.rs:11761`, `tests/ui.rs:7424`, `design.md:3107` and `3573`. All exact.

## Departures caught
- Safari refuses `wss://` to a clicked-through certificate: caught with a wrong fix. The plan hedges it into a spike item ("if a browser refuses the socket") instead of stating it, and the fallback is a 30-second refetch of the visible view. Worse, the connected flow is "open the socket and wait for `hello`, then fetch projects", so on Safari the first list load may not happen until the 30-second refetch, and a waiting session moves to the top up to 30 s late on the device the page is for.
- Certificate download is not an option with this certificate: caught, with the right reasons (no SAN, 4096 expiry, profile install, DHCP moves the address).
- Each file is a connection against a pool of 4: caught with a wrong fix. The plan keeps three files plus a `/favicon.ico` request and says "three files keep a cold load within the pool", but never says what the pool leaves for a second device, and the favicon is a fourth parallel request.
- Device name is the page's to send: caught exactly (`navigator.userAgent` into `device_name`, host unchanged).
- Found by the plan, not in the reference: the card excerpt is not on the wire and §4.3 misdescribes it (verified); nothing tells a device its current scope after `SetDeviceScope`, so `/info` gains a `device` field (verified); the dialog builds a LAN address even when bound to loopback, so a browser on the Mac itself could never pair (verified at `ui/remote.rs:156-159`).

## Decisions to overturn
- Token in `localStorage` via a `Sec-WebSocket-Protocol` value, against the reference's `HttpOnly; Secure; SameSite=Strict` cookie read into `bearer` by `read_request`. The plan's argument that cookies ignore ports is real and the reference's server-id naming does not answer it, but the reference names `localStorage` as the call to reverse and I agree an XSS-readable token is the weaker place.
- Socket-first live updates with a 30-second refetch fallback, against the reference's poll-first, socket-when-it-opens. On Safari the acceptance ("moves to the top without a reload") fails in practice.

## Over-specification
- The `PAGE_PER_MINUTE` window and its test (the pool already bounds strangers), the `Referrer-Policy`/`img-src`/favicon details, the full `ops/{op}` timeout state machine and 16 KiB check copied from §4.5, the browser-matrix spike with phone screenshots, and the closing "Commit" section restating the contract list. Roughly 15 to 20 percent.

## Hand-off
- Load the lists over REST before opening the socket and poll them every few seconds, treating the socket as an upgrade, because Safari will not open `wss://` to a clicked-through certificate; and prefer the cookie over `localStorage`.

## Notes
- best: Every one of dozens of line references is exact, and it found three real departures the reference missed (no excerpt on the wire, scope never told to a device, loopback address in the invite).
- worst: It builds the page around the events socket and defers the Safari `wss://` refusal to a spike, so the first cut would be nearly static on an iPhone.