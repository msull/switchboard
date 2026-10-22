| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| T | 3 | 1 | 0 | 0 | no |

## Grounding
- `client/Sources/SwitchboardProtocol/Info.swift:114-116`: found (the shared parser decodes JSON only).
- `client/Sources/switchboard-remote/main.swift:107-111`: found (single-argument pairing calls that parser).
- `client/Sources/SwitchboardUI/BrowserView.swift`: found (the paste flow calls `PairingInvite.parse`).
- `src/ui/remote.rs:160-166`: found (inline JSON invite construction; however, “with no test” is wrong—`tests/ui.rs:7474` onward tests the copied JSON).
- `control/src/lib.rs:645-646`: found (`card` is documented as a state word, not an excerpt).

## Departures caught
- Safari’s certificate exception does not enable WebSockets: missed as an established limitation; conditional polling covers only the session view, leaving collection updates unspecified.
- Certificate download cannot solve trust with the existing certificate: caught incompletely; identifies missing SAN and long validity, chooses click-through, but omits EKU and leaves reissue as a fallback.
- Parallel page connections exhaust unauthenticated slots: caught with an incomplete fix; uses two files and considers raising the limit, but budgets only one device.
- Device name must come from page JavaScript: caught; explicitly uses `navigator.userAgent`.
- Missing wire excerpt: caught, consistent with the reference erratum; `card` and `waiting_reason` are a justified substitute.
- Scope unavailable after reload, token exposed in the pair body, and LAN invite despite loopback listening: caught, matching the reference errata.
- Additional departure: revocation currently deletes the device, preventing a distinct revoked response. Correctly identified, but the proposed retained-token solution introduces downgrade access restoration.

## Decisions to overturn
- Replace conditional session-only polling with polling of collections and open content, plus optional WebSockets, as reference decision 3 requires.
- Replace `SameSite=Lax` with `Strict`, and namespace the cookie by server ID, as reference decision 2 requires. Origin checks do not prevent cookie collisions between ports.
- Remove the `localStorage` fallback, which reference decision 2 explicitly rejects.
- Serve static constants on the connection thread, not through frame dispatch, per reference decision 5.
- Narrow CSP from unrestricted `wss:` and unrestricted inline styles to the reference’s origin restriction and permitted style hash.
- Replace the unauthenticated API-literal path test: authentication precedes route lookup, so nonexistent API paths also return non-404 responses. Reference decision 7 requires meaningful verification of served embedded resources.
- Reverse retaining usable revoked-token hashes in an unversioned format that older builds accept. The reference preserves existing revocation behavior; the plan explicitly acknowledges its downgrade regression.

## Over-specification
- Roughly half can be cut: repeated contract inventories, schema bookkeeping, exact commit wording, duplicated owner fallbacks, speculative browser gates, and extensive tombstone machinery. These obscure missing collection polling and revision-based response handling.

## Hand-off
- no: core browser behavior remains conditional on a manual spike, and authentication, routing, revocation, and validation decisions need revision before implementation.

## Notes
- best: Identifies the missing excerpt, reload scope gap, and token leakage from cookie-mode pairing responses.
- worst: Treats Safari live updates as unresolved while prescribing substantial unrelated persistence changes.