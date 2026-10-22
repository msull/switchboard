| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| D | 3 | 2 | 0 | 1 | no |

## Grounding
- `docs/remote-client.md` §4.3: found (claims the excerpt is hook `last_message` stored as session metadata).
- `control/src/lib.rs:637`: found (`SessionView` has no excerpt or `last_message` field).
- `src/adapters/hooks.rs:71`: found (`last_message` is an optional hook log field).
- `src/ui/cards.rs:558`: found (`agent_excerpt` derives text from the cached conversation, with activity/prompt fallbacks and a 160-character limit).
- `src/app.rs:3127`: found (`authenticate` reads only the bearer token and returns the device; it does not attach scope to replies).

## Departures caught
- Safari WebSocket failure after certificate click-through: caught, but treated as an unverified possibility; polling starts only after two failures, contrary to the reference’s poll-first decision.
- Certificate download is not viable with this certificate: caught; identifies missing SAN and chooses click-through, though omits the reference’s EKU and validity constraints.
- Static files consume scarce unauthenticated connections: caught with a wrong fix against the reference; three files plus connection closure and assumed browser retries replace the two-file limit without establishing capacity for a second device.
- Device name belongs in the page’s pairing body: caught with a wrong fix; adds host-side User-Agent parsing instead of having the page send `device_name`.
- Additional: correctly identifies the missing excerpt field, missing scope discovery on subsequent requests, and LAN-address invites under loopback-only listening. These are useful findings absent from the reference; the scope endpoint is justified despite the reference’s simpler scope handling.

## Decisions to overturn
- Start polling immediately and use the socket opportunistically, following reference decision 3; do not gate the initial snapshot on socket success.
- Use two page files and establish second-device capacity, following reference decision 5.
- Derive and send `device_name` in JavaScript, following reference departure 4.
- Remove the conditional switch to token storage in `localStorage`, which reference decision 2 explicitly rejects.
- Revisit excerpts only for waiting Claude sessions: the missing field is real, but silently deferring all other row excerpts conflicts with reference decision 6 and the requested first cut.

## Over-specification
- Roughly one-third could go: repeated contracts across decisions, implementation bullets, steps and contract additions; unchanged schema/action inventories; mechanical constructor instructions; extensive source-string guards and user-agent cases. These obscure the browser behavior that still needs settling.

## Hand-off
- No: live-update startup, connection capacity, device naming and the credential fallback need revision before implementation.

## Notes
- best: Finds concrete spec/code mismatches beyond the reference, particularly excerpts and scope discovery.
- worst: Elaborate implementation detail masks unresolved behavior on the primary Safari target.