| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| V | 3 | 1 | 0 | 1 | no |

## Grounding
- `facts.md`: found (provided text, not a repository file; its Settled section supports the cited facts).
- `src/adapters/remote.rs` 496–502: found (request target is split into path and query parameters).
- `docs/remote-client.md` 302–320: found (pairing, key verification, relay risk, tokens and revocation).
- `control/src/lib.rs` 645–646: found (`card` is explicitly the card’s short state word).
- `src/ui/cards.rs` 197: found (computes `waiting_reason` for waiting cards; this does not establish that every card’s excerpt is that reason).

## Departures caught
- Safari rejects the clicked-through certificate for WebSockets: missed. Treats this as unresolved and proposes accepting another warning, without polling.
- Certificate download is unsuitable with the existing certificate: caught with a wrong fix. Notices certificate deficiencies but retains profile installation and SAN-only reissue fallbacks.
- Parallel page resources exhaust unauthenticated slots: caught. `Connection: close` addresses prolonged occupancy, but three files and a deferred phone measurement leave second-device capacity unsettled. This is a useful mitigation beyond the reference’s two-file prescription.
- Device name must come from page script: caught, using `navigator.userAgent`.
- Missing excerpt field: caught; the reference erratum supports using `card` and `waiting_reason`.
- Missing scope recovery after reload: caught, but `/info` cannot recover scope and a hidden reply box cannot provoke the proposed 403 correction.
- LAN invite while listening only on loopback: caught and addressed.
- Browser pair reply exposes the token in JSON: caught and addressed.
- Additional finding: revocation and identity rotation share a 401; comparing the stored key against `/info` distinguishes them.

## Decisions to overturn
- Replace socket-only updates and “accept the warning again” with polling plus optional WebSocket, as reference decision 3 requires; send content `watch` frames when connected.
- Replace fixed `__Host-sb` with a server-specific cookie name, per reference decision 2, preventing instances on different ports from overwriting each other.
- Remove certificate-download and reissue fallbacks, conflicting with reference decision 1.
- Restore the query URL and immediate query clearing, per reference decision 4. Fragment pairing has a defensible rationale, but silently leaving query invitations uncleared and unusable is unacceptable.
- Settle page-load capacity before hand-off, against reference decision 5’s two-file design or acceptable pool increase.

## Over-specification
- Roughly one-third is removable: repeated trust explanations, repeated owner/run-item tables, unchanged schema inventories, commit wording, and negative contract declarations. These obscure unresolved acceptance blockers.

## Hand-off
- no: Safari live updates cannot work as planned, content subscriptions are missing, and cookie collisions and connection capacity remain unresolved.

## Notes
- best: Concrete connection-thread serving and cookie-only browser pairing preserve the native protocol.
- worst: Extensive detail leaves the principal target browser without working live updates.