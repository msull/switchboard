| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| Y | 3 | 2 | 0 | 1 | no |

## Grounding
- `src/core/model.rs:12`: found (`SCHEMA_VERSION = 16`).
- `src/core/model.rs:612`: found (`VIEWS_SCHEMA_VERSION = 7`).
- `src/ports/remote.rs:17`: found (`IDENTITY_SCHEMA_VERSION = 1`).
- `src/adapters/remote.rs:101-110`: found (certificate construction supplies an empty SAN list and sets the CN; no EKU or basicConstraints configuration).
- `src/adapters/remote.rs:111`: found (SPKI fingerprint hashes the key’s subject-public-key information).

## Departures caught
- Safari WebSocket failure after certificate click-through: caught, but treated as unresolved; polling fallback addresses it, while the mandatory browser gate can still block delivery.
- Certificate download is unsuitable with the existing certificate: caught with a wrong fix—rejects download but introduces address-dependent certificate reissuance, contrary to the reference.
- Parallel static requests consume unauthenticated slots: caught with a wrong fix against the reference—four assets and connection closing replace its two-file limit without establishing capacity for a second device. Closing connections does mitigate idle-slot retention.
- Device naming belongs in page script: caught; uses `navigator.userAgent` and sends `device_name`.
- Additional departures acknowledged in the reference errata: catches the missing excerpt field, missing scope discovery after reload, LAN invite versus loopback binding, and token exposure in a cookie-mode pair response.

## Decisions to overturn
- Remove certificate reissuance, identity migration, address monitoring and rebinding; reference decision 1 retains the existing identity and click-through.
- Start polling immediately and treat WebSockets as optional; reference decision 3 is poll-first, whereas this plan waits for `hello` or a five-second failure.
- Remove the mandatory owner-device gate that stops implementation if the upgrade lacks its cookie; the reference’s polling fallback explicitly tolerates that failure.
- Reduce the static surface to HTML and script and establish second-device headroom; reference decision 5 specifies two files.
- Explicitly watch `conversation` for the last-response view; reference decision 3 requires it, while the plan specifies a watch only for screens.
- Restrict `connect-src` to the host rather than all `wss:` destinations; reference decision 5 requires same-origin connections.

## Over-specification
- Roughly one-third is removable: repeated contract inventories, certificate lifecycle machinery outside the reference’s scope, detailed mechanical implementation instructions, and repeated browser experiments and follow-up conditions.

## Hand-off
- no: certificate lifecycle changes, a blocking browser gate, and incomplete live-update instructions require substantive revision before implementation.

## Notes
- best: Thorough cookie handling preserves native bearer authentication and addresses reload scope discovery and token-body exposure.
- worst: Converts uncertain browser behavior into an identity redesign and a delivery-blocking prerequisite despite having a polling fallback.