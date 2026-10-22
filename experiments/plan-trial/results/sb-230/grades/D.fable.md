| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| D | 3 | 3 | 2 | 2 | note |

## Grounding
- `docs/remote-client.md` §4.3, excerpt is `last_message` from the `Stop` hook "a field of the session's metadata": found (line 207 says exactly that; the record has no such field, so the plan's departure is real)
- `control/src/lib.rs:637` `SessionView`: found (struct at 637, has `card`, `liveness`, no excerpt field)
- `src/adapters/hooks.rs:71` `last_message` on the hook event: found (field at line 71 of the hook event struct)
- `src/ui/cards.rs:558` `agent_excerpt`: found (fn at 558, strips `#-*>` from the first line as the plan describes)
- `src/app.rs:3127` `authenticate` returns the device and nothing reaches the reply: found (fn at 3127 reads only `request.bearer`)
- Also checked: `remote.rs:360` `RemoteRequest`, `remote.rs:102` empty SAN list, `ui/remote.rs:157/171` unconditional `local_address()`, `app.rs:3023` prefix strip, `remote.rs:567` pools, `ServerConnection.swift:232` scope from pair reply, spec line 316 "host answers with the device's scope". All as the plan says.

## Departures caught
- WebSocket will not open from Safari: caught in effect. The plan does not assert the WebKit limitation; it says the trust claims are unverified, puts a spike first, and ships a polling fallback that it names as "the answer to a browser that will not open `wss:` under a clicked-through certificate". An implementer following it gets a working page on iPhone, with a short delay before polling kicks in.
- Certificate download is not an option: caught, with the right reason (no SAN, name mismatch, address changes). Click-through is the trust step.
- Each file is a connection: caught. Six browser connections against a pool of four, answered with `Connection: close` on the connection thread. Three files rather than the reference's two, but the fix holds.
- Device name is the page's to send: caught with a different fix. The plan knows the host drops headers and chooses to read `User-Agent` host-side rather than have the page fill `device_name`. Scored under decisions.
- Found beyond the reference, all verified: the card excerpt is not on the wire (spec §4.3 claims a field the record lacks); no endpoint tells a paired device its scope (spec §5 line 316 is false in code); the QR carries the LAN address even when the listener is loopback-only.

## Decisions to overturn
- Host-side `device_label(user_agent)` in the core, plus a `user_agent` field on `RemoteRequest` and a table of user-agent tests. The reference has the page send `device_name` from `navigator.userAgent` like every other client; no header, no parser. One call reversed.
- Borderline, not reversed: `GET /api/v1/device`. The reference keeps scope from the pair reply; the plan's argument (a cookie-loaded page has no pair reply, dialog changes never reach it) is sound and the endpoint is small. I would keep it, but note it is the "new read model" the issue said to avoid unless unavoidable, and storing scope in `localStorage` at pair time would have served the first cut.
- Socket-first then poll after two failures, versus the reference's poll-first: functionally equivalent, not reversed.

## Over-specification
- The "Contract additions" section restates the decisions and the adapter list verbatim. "The record", "The signals" and "Actions and effects" exist to say nothing changes. The `PAGE_PER_MINUTE` window and its 429 test are a mechanism the existing pools already bound. The user-agent label test matrix goes with the reversed decision. Roughly a fifth to a quarter of the plan.

## Hand-off
- Drop the host-side User-Agent parsing and have the page post `device_name` from `navigator.userAgent`; and treat polling as the primary path on Safari, since WebKit refuses `wss://` to a clicked-through certificate regardless of what the spike finds for cookies.

## Notes
- best: Three verified departures the reference missed (excerpt not on the wire, no own-scope endpoint, QR carrying a LAN address on a loopback-only listener), each with a specific fix and a spec correction.
- worst: It does not know Safari refuses the socket under a clicked-through certificate, so the key constraint of the issue is hedged into a spike rather than designed around from the start.