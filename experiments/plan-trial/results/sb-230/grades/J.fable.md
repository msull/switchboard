| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| J | 3 | 2 | 2 | 2 | note |

## Grounding
- `src/core/model.rs:12` (`SCHEMA_VERSION` 16): found (line 12 is `pub const SCHEMA_VERSION: u32 = 16;`)
- `src/core/model.rs:612` (`VIEWS_SCHEMA_VERSION` 7): found (exact line)
- `src/ports/remote.rs:17` (`IDENTITY_SCHEMA_VERSION` 1): found (exact line)
- `src/app.rs:4252` (`REMOTE_PROTOCOL` 1): found (exact line, with its doc comment)
- `docs/remote-client.md:207` (excerpt is `last_message` from the Stop hook): found (the `session.last_response` bullet says exactly that)
- Beyond the five, I also opened `src/core/events.rs:132` (Stopped arm reads only `pending`), `src/bin/switchboard-hook.rs:37`, `control/src/lib.rs:637` (`SessionView`, no excerpt), `src/ui/cards.rs:558` (`agent_excerpt`), `src/app.rs:3662`, `3650`, `3754`, `src/adapters/remote.rs:102`, `360`, `444`, `623`, `829`, `src/ui/remote.rs:141/157/160/230/303`, `src/core/remote.rs:86/169`, `tests/remote.rs:426/445/682/1587`, the ten `RemoteReply` literal sites, `INBOX` 64 and `DRAIN_PER_PASS` 8, `INFO_PER_MINUTE` 60, `Info.swift:94/114`, `main.swift:107`, `BrowserView.swift:106,883`. Every one is real and says what the plan says. The "`:1394` neighbourhood" for the strangers test is off (that test is at 3226), the only loose reference I found.

## Departures caught
- WebSocket will not open from Safari over a click-through certificate: **missed**. The plan's "Signals" section rides live updates entirely on `/api/v1/events`; it flags `connect-src` for `wss:` as a CSP risk on old Safari, not the trust refusal, and has no polling fallback. Its real-phone gate checks `/info`, not the socket.
- Certificate download is not an option: **caught**. Click-through is the trust step, no download, no SANs, no reissue, with the reasons (profile install, 398-day validity, EKU, SANs). A gated contingency would add SANs if the phone refuses the click-through; it should not fire.
- Each page file is a connection: **caught with a different fix**. The plan sends `Connection: close` on every static answer so no page load parks a slot in the pool of four, and says why. It still ships three files where two would do.
- Device name is the page's to send: **caught** (departure 5, citing `remote_pair` and `new_device`).
- Found that the reference lacks: the card's excerpt is not on the wire at all (`events.rs:132` drops `last_message`; `SessionView` has no field), so the spec sentence at `remote-client.md:207` is wrong; and the device's scope is returned only by `/pair`, so a cookie-only page reload has nothing to read it from. Both verified. Also that the page cannot be served from `serve_remote` on the frame without taking inbox slots, which the reference agrees with.

## Decisions to overturn
- Live updates by socket only, no polling: conflicts with the reference's "poll first, socket second"; on Safari the page would be static. One reversal.
- Not reversed but noted: a third static file (`app.css`) where the reference inlines CSS; a `Max-Age` refresh on `/device` and the new `/device` endpoint are extra machinery the reference avoids, though `/device` is a defensible answer to the scope departure the plan found.

## Over-specification
- The schema-version preamble, "The record" and "The signals" sections mostly say "no change" and restate spec §4.4; the ten `RemoteReply` literal sites and the `..Default::default()` mechanics are restated code; the `Max-Age` refresh via `/device`, adding `no-store`/`nosniff`/`Referrer-Policy` to every API answer, the 120-a-minute static window with its paragraph of justification, and the contract-additions list duplicating the body are more than an implementer needs. Roughly a quarter of the plan.

## Hand-off
- Add a short-interval poll of `waiting`, the project sessions and the open session's content, and use the socket only when it opens, because WebKit refuses `wss:` to a click-through certificate; fold `app.css` into the HTML.

## Notes
- best: Every one of dozens of line references is right, and it found two real departures the reference missed (the excerpt is not on the wire; scope is unreadable after a cookie-only reload).
- worst: The page's liveness depends on a WebSocket that Safari on the target devices will not open, with no polling fallback.