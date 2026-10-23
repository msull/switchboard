| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| W | 3 | 3 | 1 | 2 | note |

## Grounding
- `control/src/lib.rs:637-688` (`SessionView` has `card`, `waiting`, `waiting_reason`, no excerpt): found. The struct spans exactly those lines and has no excerpt field.
- `docs/remote-client.md:201` (last response and card excerpt masked on the frame): found. The redaction bullet is at that line.
- `docs/remote-client.md:207` (the `last_message` sentence): found. The sentence says the excerpt is `last_message` on the session's metadata, which the wire struct never got.
- `src/app.rs:3754-3766` (`refuse` logs method and path, no query): found. `log::info!` prints `request.method` and `request.path` only.
- `docs/remote-client.md:307-308` (one self-signed certificate, SPKI pinned): found. Host identity and the pin are those two bullets.
- Every further reference I opened also lines up: `RemoteReply` at 389, `read_request` at 444, `respond_upgrade` at 519, pools at 567, rate constants at 584-587, `Limits::limited` at 631, `local_address` at 962, `Served::Now` at 277-280, `serve_remote` at 3017, `authenticate` at 3127, `remote_pair` at 3606-3674, `QR_SCALE` at 42, the payload at 160-165, the test helpers at 245, 299, 406, 1406, 1587, 1829, 1960, 3226, the UI test at 7424-7489, `parse` at 114-116, the CLI at 107-125, `BrowserView` at 104-108 and 883, the Swift test at 158-166, the README and design.md lines.

## Departures caught
- WebSocket will not open from Safari behind a click-through: caught. Marked "unresolved, against it", the page polls by default and treats the socket as the enhancement, and the gap goes in design.md.
- Certificate download is not an option with this certificate: caught, with Apple's SAN, EKU and 825-day rules and the CA-shaped trust UI; click-through is the trust step and reissue is a follow-up.
- Each page file is a connection: caught, but with a weak fix. The plan keeps three files and counts them against the four slots, then opens the socket as the fourth stranger, which leaves nothing for a second device's `/info` or `/pair` during the first device's idle window. The reference says two files at most or a larger pool.
- The device name is the page's to send: caught. The script fills `device_name` from `navigator.userAgent`, with the iPad-as-Mac case handled.
- Beyond the reference: the excerpt is not on the wire (caught, with the spec sentence reworded and the gap recorded); the pair reply would carry the token in its body even with a cookie (caught, `transport: "cookie"` omits it); scope after a reload behind a cookie (half caught: the page falls back to a stored scope and a 403 on `send`, though the plan earlier says only the id and key are stored).
- Missed: the invite carries a LAN address while the host may be listening on loopback only.

## Decisions to overturn
- Static files answered from `serve_remote` on the frame via `Served::Now`. The reference reverses a frame-side route for a constant file; the connection thread should answer them, as it already does for 429 and the error bodies.
- Three files, CSS separate. With a four-slot stranger pool and the socket as the fourth connection, the plan's own count leaves no slot for a second device. The reference wants two files (CSS inline under a hash) or `MAX_UNAUTHENTICATED` raised; the plan does neither.
- Lesser, not weighed as a third: the cookie is named `switchboard_token` with no server id, so two instances on one Mac (the `SWITCHBOARD_REMOTE_PORT` case) overwrite each other's cookie. The reference names the cookie by server id.
- Not reversed: the fragment instead of the query. The reference uses `?code=`, but the fragment keeps the code out of any server-side log and `replaceState` clears it the same way, so I would keep the plan's call.

## Over-specification
- The `PAGE_PER_MINUTE` rate window and its test add a mechanism the reference does not need; page loads are already bounded by the pool.
- The `facts: Settled/Unresolved/For the owner` tags on nearly every claim, the Chrome bypass-persistence aside, the README row numbers and the follow-ups section are reading for the grader, not the implementer.
- Roughly 15 percent of the plan.

## Hand-off
- Serve the three static replies from the connection thread, fold the CSS into the HTML so a page load is two connections, and put the server id in the cookie name.

## Notes
- best: every line number checks out, and it found the missing excerpt, the token in the pair body and the Apple certificate rules on its own, with a poll-first page that works on the device it is for.
- worst: static files routed through the frame and a three-file page on a four-slot pool, which the plan's own count shows leaves nothing for a second device.