| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| F | 3 | 2 | 1 | 2 | note |

## Grounding
- `src/ui/cards.rs:177` (`agent_excerpt`): found (line 177-178 picks `conversation.and_then(agent_excerpt)` for agent kinds, captions otherwise)
- `remote-client.md` §4.3 line 207 (`last_message` as session metadata): found (the `session.last_response` bullet ends with exactly that sentence)
- `src/ports/events.rs:24` (`last_message` on the hook event): found (`Stopped { last_message: Option<String>, .. }`)
- `control/src/lib.rs:637` (`SessionView` has no excerpt field): found (struct at 636, fields are id/project/name/kind/cwd/notes/liveness/card/last_exit/last_stop_at_ms/…, no `last_message`)
- `src/app.rs:3662` (`/pair` reply carries `scope`): found (the `json!({"token", "scope", "server_id", "device_id"})` at 3660-3665)
- Also checked: `remote.rs:102` `CertificateParams::new(Vec::<String>::new())`, `app.rs:3023` frame 404, spec line 316, tests at 245/581/1587/1828/2891/3225, `tests/ui.rs:7474`, `core/remote.rs:169`, `app.rs:486`, `main.swift:107`, README 266/383, design.md 3107, and the "28 `RemoteReply {` literals" count. All exact.

## Departures caught
- Safari refuses `wss://` to a click-through certificate: caught with a wrong fix. The plan admits "I have not run this in a browser" and names the risk, but ships the socket alone and says "the plan stops there and the issue is reopened". No polling fallback; the list is static on the device the page is for.
- Certificate download is not an option: caught (no names in `make_identity`, hostname check fails on an IP).
- Each file is a connection: caught. `Connection: close` on page responses, counted against the pool of 4, says what a load holds. Three files rather than the reference's two, mitigated by the close.
- Device name is the page's to send: missed. The plan adds `user_agent` to `read_request` and a host-side `adapters::web::device_name` sniffing table; the page sends no `device_name`.
- Found beyond the reference: the card excerpt is not a wire field (`SessionView` has none, §4.3 wrong), and §5's "host answers with the device's scope" is false (only the pair reply carries it). Both verified true.

## Decisions to overturn
- Socket-only live updates with "stop and reopen" if Safari refuses `wss://`. Reference: poll first, socket when it opens. Safari will refuse, so the page must poll.
- Host-side User-Agent parsing in a new `adapters/web.rs`, with the host retaining a third header. Reference: the page fills `device_name` from `navigator.userAgent` like every client; no host parsing.
- Let stand but disagree with the reference: `GET /api/v1/device` as a new read. Reference keeps scope from the pair reply. The plan's case (scope lowered in the dialog never shows otherwise) is real, and the issue permits a read the page cannot do without, so I would not reverse it. The `X-Switchboard-Web` header guard is extra beyond `SameSite=Strict` but cheap and justified by the port-sharing gap, so I would keep it.

## Over-specification
- "The record" section says nothing changes, with four schema version numbers and a hypothetical bump recipe. "Contract additions" restates decisions 2 and 4 and the tests. The `app.js` lint test, the Safari `connect-src` fallback with a `Host` regex, `getRandomValues` versus `randomUUID`, and `Max-Age` arithmetic are detail the implementer would not need. Roughly a fifth to a quarter of the plan.

## Hand-off
- Add a short-interval poll of the collections and the open session's content as the primary refresh, with the socket as an upgrade when it opens; do not gate on reopening the issue if Safari refuses `wss://`, because it will. Have the page send `device_name` instead of parsing the user agent on the host.

## Notes
- best: Every reference checks out to the line and the count, and it found two real departures the reference missed (no excerpt on the wire, §5's scope claim).
- worst: It plans the events socket as the only live path and says to stop the work if Safari refuses it, which Safari does.