| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| AA | 3 | 2 | 0 | 1 | no |

## Grounding
- `src/adapters/remote.rs:101-107`: found (empty SAN list and a common name).
- `src/adapters/remote.rs:113`: found (SPKI hash computed from the key).
- `src/app.rs:3662`: found (`scope` in the pairing reply).
- `docs/remote-client.md:169`: found (`waiting` endpoint; the surrounding table lists sessions by project, with no all-sessions endpoint).
- `src/adapters/remote.rs:958-970`: found (`local_address()` uses a UDP socket to select an address and can return `None`).

## Departures caught
- Safari WebSocket failure after certificate click-through: caught with a wrong fix—polling is included, but working browsers also lose the socket.
- Existing certificate unsuitable for the proposed download-and-trust alternative: caught with a wrong fix—recognizes missing SAN, but introduces certificate reissue and retains download as a fallback without addressing the reference’s other certificate constraints.
- Parallel page files occupy unauthenticated connections: caught with a wrong fix—three assets and a third pool replace the reference’s two-file solution; admission before the first request remains insufficiently specified.
- Device name must come from page script: caught—uses browser-side UA and touch information.
- Missing excerpt field: caught—correctly uses `card` and `waiting_reason`, consistent with the reference erratum.
- Scope unavailable after reload: caught—adds `/me`.
- Cookie pairing otherwise exposes the token in its JSON body: caught—omits it for cookie pairing.
- LAN invite versus loopback listener: caught—suppresses the QR for loopback-only operation.
- Additional findings: handles absence of a network address and explains query/referrer exposure; project-by-project enumeration is accurate, though not itself a contradiction in the issue.

## Decisions to overturn
- Certificate reissue, address tracking and listener rebinding conflict with reference decision 1: retain the existing identity and use click-through.
- Mandatory device spikes and implementation stop conditions conflict with the reference’s settled click-through/cookie implementation.
- Fixed `sb_token` cookie name conflicts with decision 2’s server-specific name; instances on different ports would overwrite it.
- Deferring the socket entirely conflicts with decision 3’s polling plus opportunistic socket and content watches.
- Fragment-only invitation conflicts with decision 4’s specified query URL and native parsing contract.
- Three files and a new page pool conflict with decision 5’s two files and existing bounded pools, optionally enlarged.
- One globally held revision conflicts with protocol reconciliation by resource; revision-gated content polling also lacks the reference’s unconditional periodic content refresh.

## Over-specification
- Roughly one-third is avoidable: certificate regeneration/rebinding machinery, mandatory spike procedures, repeated contract inventories, duplicated decisions and follow-ups, and low-level implementation/test prescriptions. Required security headers and protocol tests are useful detail.

## Hand-off
- no: several architectural decisions need reversal, especially certificate lifecycle, cookie naming, connection budgeting and live-update reconciliation.

## Notes
- best: Identifies the missing excerpt, reload scope and token-body exposure, with concrete remedies.
- worst: Turns certificate uncertainty into substantial identity machinery and mandatory blockers despite the reference’s simpler settled trust decision.