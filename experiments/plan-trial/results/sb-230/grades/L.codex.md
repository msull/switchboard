| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| L | 3 | 1 | 0 | 1 | no |

## Grounding
- `src/adapters/remote.rs:95-127`: found (`make_identity` uses empty SANs, sets no EKU or validity overrides, and creates the identity).
- `src/ui/cards.rs:558`: found (`agent_excerpt` derives a shortened first line from the conversation).
- `control/src/lib.rs:637-688`: found (`SessionView` includes waiting metadata and card state, but no excerpt).
- `client/Sources/SwitchboardUI/BrowserView.swift:106,883`: found (both invite bootstrapping and manual pairing call `PairingInvite.parse`).
- `client/Sources/switchboard-remote/main.swift:111`: found (the single-argument pairing path calls `PairingInvite.parse`).

## Departures caught
- Safari rejects the clicked-through certificate for WebSockets: caught with a wrong fix. Polling exists, but the plan misidentifies the certificate problem, requires reissue, and starts with the socket instead of polling.
- Certificate download is not viable with the existing identity design: caught with a wrong fix. Download is rejected, but mandatory reissue introduces the address and expiry coupling the reference rejects.
- Parallel static-file connections consume the four unauthenticated slots: missed. Three assets retain the four-slot limit; serializing subsequent JavaScript requests does not serialize browser asset loading or reserve capacity for a second device.
- The page must supply the device name: caught. It uses `navigator.userAgent` and retains the host’s length limit.
- Additional findings: correctly identifies the absent excerpt field, the lack of a current-device scope endpoint, and per-project session reads. The reference overlooks the excerpt mismatch.
- Additional finding: a query transmits the code before the page verifies identity. I favor the fragment here despite the reference’s query decision; it limits inadvertent disclosure, although an impostor page can still read it.

## Decisions to overturn
- Remove certificate reissue, identity-schema changes, renewal machinery and associated actions; reference decision 1 retains the existing certificate and click-through trust.
- Name the cookie per server identity; reference decision 2 avoids collisions between installations sharing a hostname. `__Host-` does not isolate ports.
- Reduce the page to two assets and explicitly budget connections for another device; reference decision 5 addresses the four-slot constraint.
- Start polling independently of WebSocket success; reference decision 3 makes polling primary. Ensure opening a session fetches immediately in polling mode rather than depending on `watching`.
- Restore an actual excerpt strategy rather than silently substituting card state and age; reference decision 6 retains the requested excerpt. The plan correctly exposes a gap in the reference, but its replacement still drops requested behavior.

## Over-specification
- Roughly 40–45% is avoidable: certificate lifecycle implementation, the mandatory standalone spike, repeated contract inventories, exact helper signatures and literal counts, and brittle source-text tests. Useful protocol and security decisions are buried among these prescriptions.

## Hand-off
- No: certificate migration, cookie collisions, connection budgeting and polling initialization need correction before implementation.

## Notes
- best: Precise repository grounding exposes the missing excerpt and reload-time scope gaps.
- worst: An incorrect certificate premise drives extensive identity changes while the known Safari limitation remains secondary.