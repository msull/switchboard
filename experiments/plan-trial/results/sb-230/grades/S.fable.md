| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| S | 3 | 2 | 0 | 2 | note |

## Grounding
- `SCHEMA_VERSION` 16 (`src/core/model.rs:12`): found (line 11 is the const, value 16, one line off)
- `VIEWS_SCHEMA_VERSION` 7 (`src/core/model.rs:612`): found (exactly there, value 7)
- `IDENTITY_SCHEMA_VERSION` 1 (`src/ports/remote.rs:17`): found (exactly there, value 1)
- `SessionView` has `card` and no excerpt (`control/src/lib.rs:646`): found (`pub card: String` at 646, no excerpt or `last_message` field)
- `agent_excerpt` computes the excerpt UI-side (`src/ui/cards.rs:558`): found (`fn agent_excerpt(c: &Conversation)` at 558)
- Secondary refs also hold: `app.rs:3650` device_name, `:3662` scope in the pair reply, `core/remote.rs:166-169` `new_device`, `BrowserView.swift:883`, `ConnectionManager.swift:482`, `main.swift:111`, `core/tests.rs:11942/11955`, `action.rs:972`, `Cargo.lock:4869` time 0.3, README 277/383, CHANGELOG 9, design.md 3107.

## Departures caught
- WebSocket will not open from Safari under a click-through certificate: missed. Live updates are socket-only; the gate records "the events socket opens" on the phone run but no polling fallback is planned and no remedy is named if it fails. The page is static on the device the issue is for.
- Certificate download is not an option with this certificate: caught with a wrong fix. The plan picks the click-through, but keeps a host-served certificate download as the hard-fail fallback and adds a 398-day expiry plus a startup reissue to court browser trust rules, which ties the identity to a validity window the spec avoids.
- Each page file is a connection: caught. Pool raised to 8, `Connection: close` on static answers, Chrome's six-per-host limit named with two slots left for `/pair`, a test for it. Ships three files rather than two, but close handles it.
- Device name is the page's to send: caught. `navigator.userAgent` composed in script, host unchanged at `app.rs:3650`.
- Found beyond the reference: the card excerpt is not on the wire (errata confirms); scope and host name unreadable after a cookie reload (adds `GET /api/v1/device`); the pair reply would still carry the token in its body (browser pairing omits it); the invite is useless while the LAN checkbox is off (a pairing-window line); `Referrer-Policy` so the code is not leaked on subresource fetches.

## Decisions to overturn
- 398-day certificate expiry plus a startup reissue and a new `time` dependency. Reference: no reissue, no new Rust dependency, the identity binds nothing to a validity window; a reissue may also re-prompt every browser yearly, which the plan itself admits.
- Socket-only live updates. Reference decision 3: poll the collections and the open session's content first, use the socket when it opens.
- A session cookie with no `Max-Age`, so every browser start pairs again and adds a device record the host cannot dedupe (the plan lists this under known costs). The reference is silent on cookie lifetime; this is my call, from the acceptance test that revokes "the phone" as one lasting device.

## Over-specification
- Roughly a fifth to a quarter. The certificate expiry and reissue machinery (record field, startup block, read-only gate, tests) is a whole feature the issue did not ask for. The `PairedDevice.browser` flag and "Browser" label buy little. The schema-version preamble, the empty Signals section, the owner questions written twice (inline and again as a list), and the Contract additions list restating the body could all go. The user-agent parsing order and the four-way wording table are more than an implementer needs.

## Hand-off
- Drop the certificate reissue and the `time` crate, poll the reads on a short interval and treat the socket as an upgrade since Safari refuses `wss://` on a click-through certificate, and give the cookie a `Max-Age` so a browser stays one device.

## Notes
- best: a precise cookie design (`__Host-` name per server id, `Origin` check on state changes and the upgrade, no token in the browser pair body) and four real departures the issue missed.
- worst: it builds the live page on a WebSocket Safari will not open, and pays for browser trust with a yearly certificate reissue the spec's identity model was designed to avoid.