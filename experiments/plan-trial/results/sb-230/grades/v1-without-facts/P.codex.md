| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| P | 2 | 2 | 0 | 1 | no |

## Grounding
- `facts/1/repo/facts.md`: wrong (absent from the tree; its “Settled” claims cannot be checked there).
- `src/ui/cards.rs:558`: found (`agent_excerpt` computes the card excerpt from conversation content).
- `control/src/lib.rs:637`: found (`SessionView` has card and waiting metadata, but no excerpt).
- `docs/remote-client.md:207`: found (describes a `last_message` metadata field absent from the implementation).
- `src/app.rs:3660`–`3665`: found (pairing returns token, scope, server ID and device ID).

## Departures caught
- Safari WebSocket failure after certificate click-through: caught, though framed as unresolved; polling provides the necessary fallback.
- Certificate download cannot solve trust with the existing certificate: caught with a wrong fix (the conditional SAN/EKU reissue and download contradict the reference’s decision to retain the identity model).
- Parallel asset connections exhaust the unauthenticated pool: caught incompletely. Closing connections mitigates occupancy, but three assets and a per-page queue do not establish capacity for a second browser loading simultaneously.
- Device name must come from the page: caught; script supplies `device_name`.
- Reference errata: catches the missing excerpt, missing scope discovery after reload, loopback/LAN invite mismatch, and token exposure in the cookie pairing response.
- Additional observation: correctly identifies that a 401 alone cannot distinguish an unpaired browser from a revoked device.

## Decisions to overturn
- Remove the certificate reissue/download fallback; the reference chooses click-through without changing certificate identity semantics.
- Remove the `localStorage` bearer fallback; the reference explicitly rejects that credential-storage choice.
- Make native text parsing accept URLs and make “Copy invite” copy the URL; preserving JSON-only parsing conflicts with the reference’s unified invite decision.
- Reduce static assets to two, or settle sufficient pool capacity now; the reference requires room for another device, rather than a device-dependent fallback.
- Exempt commands from blanket network-error retries. The reference requires speaking the existing protocol; the plan’s own send workflow requires operation lookup rather than resending.

## Over-specification
- Roughly one-third to nearly half is removable: fully developed alternative implementations, repeated contract summaries, schema inventories, speculative device contingencies, and source-text assertions that constrain implementation without testing behavior.

## Hand-off
- no: settle the competing trust, credential and invite branches, and resolve connection capacity and retry semantics before implementation.

## Notes
- best: Finds the missing excerpt and scope-discovery gaps and proposes concrete, compatible remedies.
- worst: Presents consequential decisions as multiple conditional implementations instead of delivering one settled design.