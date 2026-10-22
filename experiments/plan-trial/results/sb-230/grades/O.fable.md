| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| O | 3 | 2 | 0 | 2 | no |

## Grounding
- `read_request` (`src/adapters/remote.rs:444`): found (the fn signature is at 444, request line split follows)
- `docs/remote-client.md` §4.3 `session.last_response` paragraph with the `last_message` sentence: found (line 207, exactly as quoted)
- `src/ports/events.rs:24` (`last_message` on the hook event): found (`Stopped { last_message: Option<String>, ..}` at 24)
- `src/adapters/hooks.rs:71` (`last_message` in the hook line): found (field at 71)
- `SessionView` (`control/src/lib.rs:637`): found (struct at 637 with `card`, `waiting`, `waiting_reason`, no excerpt)
- Beyond five: `cards.rs:558`, `content.rs:827/42`, `app.rs:2781/2810/2836/2866/3650/486`, `ports/remote.rs:17`, `main.swift:107`, `tests/remote.rs:1406/1423/1587/1829/2892`, `core/tests.rs:11769`, `ui/remote.rs:113/141/280`, `design.md:3585`, `README:275`, `Limits` at 621/631, `Pin` at 183: all found. Spike numbering (20, 21, 22 exist) is correct.

## Departures caught
- WebSocket will not open from Safari after a click-through: caught with a wrong fix. The plan names the `wss://`-after-click-through question as unresolved, then "builds for the answer being yes" and explicitly accepts the page being unusable on that browser. No polling fallback; the live list and session view depend on the socket alone.
- Certificate download is not an option: caught with a wrong fix. It picks click-through and no download route, but adds a full certificate reissue (SAN, `serverAuth`, 800 days, two new `Identity` fields, schema bump, startup rollover, a "restart so the certificate names it" line) that the reference rejects as binding identity to address and expiry.
- Each file is a connection: caught. Three files instead of two, pool left at 4, `Connection: close` on file replies as the fallback plus a spike to measure it. Reasonable, though it adds a `PAGE_PER_MINUTE` window nobody asked for.
- Device name is the page's to send: caught. `navigator.userAgent` fills `device_name`, editable before pairing, no host-side parsing.
- Errata: excerpt not on the wire caught (lazy `last-response` per visible Claude row, spec sentence corrected); scope after reload caught (`GET /api/v1/me`); loopback-only listener with a LAN address in the QR caught.
- Found and not in the reference: the query form sends the code to the host in the page request line before the page can compare keys, so the key check is hollow against an impostor; the plan puts the values in the fragment and accepts both forms. I agree with this and would not reverse it despite the reference's query form.

## Decisions to overturn
- Token in `localStorage` via `Sec-WebSocket-Protocol`; reference takes the `HttpOnly; Secure; SameSite=Strict` cookie with `bearer` filled from it and names `localStorage` as the call to reverse. The plan's same-site-ignores-port argument is real but an `Origin` check covers it.
- Certificate reissue with SAN/EKU/800-day validity and the `Identity` schema change; reference says no reissue, click-through is the trust step and the identity model deliberately binds no address or window.
- Socket-only live updates; reference says poll first and use the socket when it opens, since the target device is the one where `wss://` fails.

## Over-specification
- The certificate block (builder, rollover rules, `covers`, three dialog lines) is about a sixth of the plan and goes away with the reversal. The repeated "Unresolved in Facts / For the owner in Facts / fallback built only if the owner asks" framing, the RFC citations with "I have not fetched them", the spike directory and the page rate window are noise an implementer would skip, roughly another tenth. Tests and docs sections are tight and needed.

## Hand-off
- No. Three reversals, one of which (socket-only) leaves the page static on iPhone and iPad, and another rewrites `identity.json` handling; that is more than one line of guidance.

## Notes
- best: the fragment-not-query catch, which the reference missed, plus a precise, fully grounded test list down to helper names and line numbers.
- worst: knowingly shipping without a polling path, accepting "unusable on that browser" for the browser the issue is about.