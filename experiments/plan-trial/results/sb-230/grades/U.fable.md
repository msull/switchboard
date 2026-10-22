| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| U | 3 | 3 | 1 | 2 | note |

## Grounding
- `src/ui/cards.rs:176–180` (agent card takes excerpt from the UI's conversation cache): found (lines 174-179 read `cx.conversations.get(record.id)` and `conversation.and_then(agent_excerpt)` for agent kinds; other kinds take `cx.state.captions`)
- `agent_excerpt` at `cards.rs:558`: found (`fn agent_excerpt(c: &Conversation) -> Option<String>` at 558, reads `c.turns.last()`)
- `src/ports/events.rs:24` (`last_message` only on the hook event): found (`Stopped { last_message: Option<String>, ... }` at 23-24)
- `docs/remote-client.md:207` (spec says the excerpt is "a field of the session's metadata"): found (line 207 ends with exactly that sentence about `last_message` from the Stop hook on the record)
- `src/app.rs:3662` (scope appears only in the `/pair` reply): found (`"scope": device.scope` at 3662 inside the `json!` reply of `remote_pair`)
- Spot checks beyond five, all correct: `MAX_UNAUTHENTICATED` 567, `PAIR_PER_MINUTE`/`INFO_PER_MINUTE` 583-584, accept-drop at 701-704, slot release 719, `while let` at 783, 429 `continue` at 788-794, `respond_upgrade` call at 853, `Limits::limited` 631, `bearer_token` base64url 334-340, `serve_remote` prefix strip 3023, 401 wording 3146, rotation `devices.clear()` 2810-2816, `DRAIN_PER_PASS` poll at 1587, name trim at `core/remote.rs:169`, `RevokeDevice` at 86, `SCHEMA_VERSION` 16 / `VIEWS_SCHEMA_VERSION` 7 / `IDENTITY_SCHEMA_VERSION` 1, `BrowserView.swift:106` and `:883`, `main.swift:110-111`, `tests/ui.rs:7487-7492`, spike 22 last.

## Departures caught
- WebSocket will not open from Safari: caught conditionally. The plan does not state the WebKit limitation as a known fact; it puts "whether `wss://` opens under the exception" in the spike's phone half and gates: if it fails, poll `/api/v1/waiting` and the open session every three seconds, socket later. Since the spike runs before any app change, the implementer ends up at polling, but the plan's default design is socket-first.
- Certificate download is not an option: caught in substance. Click-through chosen with correct reasons (no SAN, browser cannot pin, Chrome needs SAN), download kept only as a spike-gated fallback with the §5 cost named. It does not mention the EKU/validity rules.
- Each file of the page is a connection: caught with a different, defensible fix. The plan sees that `Connection: close` alone does nothing because the loop at 783 reads again, so it returns from `connection` after the page reply to free the slot at once, keeps the pool at 4, and notes the fifth parallel request is still dropped at accept. It ships three files where the reference wants two; the early close mitigates most of the cost.
- Device name is the page's to send: caught (`device_name` from `navigator.userAgent`, no host parsing).
- Found that the reference lacks (errata confirms the first two): the card excerpt is not on the wire (shows `card`/`waiting_reason`, defers the excerpt); no route returns the device's scope (adds `GET /api/v1/device`); the headset parses a pasted invite at `BrowserView.swift:883` rather than typing a code; revoked and rotated share one 401 and the page tells them apart by `/info`; `connection`'s loop needs a return, not a header, to close.
- Missed: the invite carries a LAN address while the host may be listening on loopback (plan only handles the empty-address case).

## Decisions to overturn
- Token in `localStorage` with subprotocol transport. The reference sets an `HttpOnly; Secure; SameSite=Strict` cookie named with the server id and fills `bearer` from it in `read_request`, and names `localStorage` as the call to reverse. The plan's CSP-only defence for a script-readable token is weaker than the cookie.
- CSP built per request from a validated `Host` header (`connect-src 'self' wss://{host}`), with a 400 path and a `host` field on `RemoteRequest`. The reference uses `connect-src 'self'`; if the planner distrusts `'self'` for sockets, `'self' wss:` gets there with no header plumbing. I would drop the Host parsing before implementing; this one is less certain than the first since the facts file pushed the planner there.
- Not counted but noted: socket-first with polling only behind the spike gate. The reference polls first and layers the socket on. The gate reaches polling in practice, so I did not reverse it.

## Over-specification
- Template sections with nothing to say ("Signals", "Actions and effects" preamble) and the recitation of three schema version numbers to conclude no bump is needed.
- The revoked/rotated section narrates the rotation code paths at 2805-2840 at a length an implementer does not need once the rule is stated.
- "Contract additions" repeats the Transport and Routes sections.
- `Referrer-Policy`, the `Host` validation tests and `page_host_is_validated` exist only to support the CSP choice above.
- Roughly a fifth of the plan.

## Hand-off
- Build polling as the baseline before the socket, since Safari refuses `wss://` through a click-through certificate, and replace the Host-built CSP with `connect-src 'self'` so no new header parsing is needed.

## Notes
- best: Every line reference checked is correct, and it found the excerpt-not-on-wire, scope-on-reload, headset-paste and connection-loop departures the issue and reference missed, each with a concrete fix.
- worst: It keeps the bearer token in `localStorage` and spends a header field, a 400 path and three tests on a per-request CSP that `'self'` already covers.