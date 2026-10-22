| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| A | 3 | 1 | 0 | 1 | no |

## Grounding
- `src/app.rs:3028-3029`: found (the `info` and `pair` arms follow stripping `/api/v1/`).
- `src/adapters/remote.rs:631`: found (the rate limiter selects `PAIR_PATH` and `INFO_PATH`).
- `src/adapters/remote.rs:444`: found (`read_request` retains the query separately from the path).
- `src/adapters/remote.rs:102`: found (certificate construction supplies no SANs and leaves validity at defaults).
- `src/app.rs:3662`: found (the pairing reply includes the device’s scope).

## Departures caught
- Safari refuses the click-through certificate for WebSockets: missed. The spike asks about it, but supplies no polling fallback; the page remains socket-dependent.
- Certificate download is not viable with this certificate: caught with a wrong fix. Download is rejected, but conditional certificate reissue contradicts the reference’s unchanged identity policy and does not resolve Safari’s socket limitation.
- Parallel page connections consume the four unauthenticated slots: caught. Closing unauthenticated connections explicitly addresses retention and second-device access, although three files depart from the reference’s two-file limit.
- The page must supply the device name: caught through `navigator.userAgent`, with no host-side parsing.
- Additional findings: full endpoint names, query exposure in the initial request, scope availability after reload, indistinguishable unauthorized responses, token exposure in the pairing body, and JSON invite compatibility. These are useful observations; fragment pairing and a new device endpoint are design choices, not established necessities.
- The plan disputes the reference’s claim about protection against a relayed code. Its distinction is reasonable: comparing `/info` cannot authenticate a page or endpoint that faithfully relays the genuine response.

## Decisions to overturn
- Restore browser control according to the pairing window’s scope; the read-only cap and renewed permission gates contradict the requested reply workflow and the reference’s device model.
- Implement polling with optional sockets, and send content `watch` messages when connected; the reference explicitly requires both.
- Include the server id in the cookie name; the fixed name permits instances on different ports to overwrite each other’s cookie. Origin checks do not prevent that collision.
- Use the specified query URL rather than silently substituting a fragment. The fragment has a defensible privacy benefit, but conflicts with the owner-checked URL contract.
- Remove conditional certificate reissue and the mandatory owner-operated preimplementation gate; the reference settles click-through with polling and preserves the certificate.
- Reduce the page to two files as the reference specifies. Closing connections is a useful alternative mitigation, but the three-file design still departs from that decision.
- Avoid declaring `/api/v1/device` indispensable; the reference retains scope from pairing, and the plan has not established why persisted non-secret scope cannot support this cut.

## Over-specification
- Roughly one-third to two-fifths is expendable: unchanged schema inventories, empty signals/actions sections, repeated trust and approval discussions, duplicated contract summaries, detailed test mechanics, and certificate backup contingencies. The manual spike gate adds substantial process without resolving the known Safari failure.

## Hand-off
- No: the plan blocks authorized functionality, cannot deliver live updates on the reference’s target Safari devices, and introduces cookie collisions across host instances.

## Notes
- best: Concrete transport analysis, especially connection-slot retention and removing the token from browser pairing responses.
- worst: It leaves the known Safari failure unresolved while reopening the user’s explicit authorization for browser reads and replies.