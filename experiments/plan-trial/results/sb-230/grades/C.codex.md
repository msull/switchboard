| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| C | 3 | 1 | 0 | 2 | no |

## Grounding
- `src/app.rs:3660`: found (pairing reply includes token, scope, server ID and device ID).
- `src/app.rs:3730`: found (envelope contains epoch, revision and body).
- `control/src/lib.rs:637`: found (`SessionView` has the cited state fields and no message excerpt).
- `src/adapters/remote.rs:770`: found (connection handler performs rate checks and queues requests to the frame).
- `src/app.rs:3023`: found (rejects paths outside `/api/v1/`).

## Departures caught
- Safari rejects the clicked-through certificate for WebSockets: missed; initial reads wait for `hello`, with no polling fallback.
- Certificate download is not viable with the existing identity: caught; identifies missing SAN and chooses click-through, although omitting EKU and validity constraints.
- Parallel page files consume unauthenticated slots: caught with a wrong fix against the reference. Closing connections reduces idle occupancy, but three files still consume slots while active; second-device capacity is not settled.
- Device naming belongs in browser script: missed; explicitly adds host-side User-Agent parsing instead.
- Additional departure: correctly identifies missing scope information on subsequent replies and proposes authenticated `/info` metadata.
- Additional departure: correctly identifies the missing list excerpt, but drops the requested feature instead of resolving it.
- Additional observations: connection-thread static serving and URL invitations agree with the reference.

## Decisions to overturn
- Replace socket-dependent startup and reconnect-only recovery with polling plus optional WebSockets, per reference decision 3.
- Namespace the cookie by server ID, per decision 2; `switchboard_token` collides across instances sharing an address because cookies ignore ports.
- Use two embedded files and explicitly account for concurrent devices, per decision 5. Connection closing is useful mitigation but does not eliminate pool admission.
- Generate `device_name` from `navigator.userAgent`, per departure 4; avoid expanding host-side request handling for this.
- Restore the requested list excerpt, per decision 6. The absent wire field is a legitimate omission in the reference’s implementation guidance, but silently replacing the excerpt with other metadata does not satisfy the issue.

## Over-specification
- Roughly one quarter is cuttable: repeated contract inventories, empty “signals/actions/UI” framing, schema assurances, exact helper signatures, and tests that mirror those helper declarations.

## Hand-off
- No: the primary phone-browser flow can stall before displaying sessions, and cookie isolation and connection capacity require substantive corrections.

## Notes
- best: Concrete, accurate repository anchors and a well-developed cookie authentication test flow.
- worst: Misses Safari’s WebSocket limitation and makes the initial session reads depend on that socket.