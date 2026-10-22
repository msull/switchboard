| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| K | 2 | 1 | 0 | 0 | no |

## Grounding
- `control/src/lib.rs:637`: found (`SessionView` contains kind, card, waiting and waiting reason, but no excerpt).
- `src/ui/cards.rs:558`: found (`agent_excerpt` derives a transcript excerpt, including activity/prompt fallbacks and truncation).
- `client/Sources/SwitchboardUI/BrowserView.swift`: found (reads last responses for conversation holders; this supports fetching content separately, though not literally every row).
- `src/adapters/remote.rs:95-127`: found (self-signed certificate with empty SANs, no explicit EKU and default validity).
- `src/adapters/bonjour.rs:41`: wrong (the empty address argument is followed by `.enable_addr_auto()`; the plan’s conclusion that this publishes no address record overlooks that configuration).

## Departures caught
- Safari’s WebSocket failure after certificate click-through: missed. Certificate reissue and a subprotocol spike do not provide the required polling fallback.
- Certificate download is incompatible with the existing certificate: caught with a wrong fix. Download is rejected, but certificate reissue and renewal contradict the reference’s decision to retain the identity certificate.
- Parallel static requests occupy the four unauthenticated slots: caught with a wrong fix against the reference. Closing connections mitigates prolonged occupancy, but four files and probes still compete with a second device; the plan does not settle that capacity.
- Device naming belongs in page script: caught, using `navigator.userAgent` and `device_name`.
- Additional findings: correctly identifies the absent excerpt field and lack of a current-device scope read. The Bonjour address-record claim is unsupported; DHCP persistence and cookie exposure are additional design concerns rather than overlooked code requirements.

## Decisions to overturn
- Replace socket-dependent initialization and reconnect with polling-first operation plus optional WebSocket updates, as reference decision 3 requires.
- Remove certificate reissue, daily renewal and renewal-triggered warnings; reference decision 1 retains click-through with no reissue.
- Replace split credentials, `localStorage` and custom authentication headers/subprotocols with the reference’s server-specific HttpOnly cookie mapped into existing bearer authentication.
- Reduce the static surface to two files and explicitly accommodate a second browser, following reference decision 5. Connection closing is a useful mitigation but does not establish equivalent capacity.
- Add an explicit conversation watch for the open agent session; reference decision 3 requires it, whereas the plan specifies a watch only for screens.
- Drop `/api/v1/device` under the reference’s scope-from-pair decision. The plan identifies a legitimate scope-refresh limitation, but does not establish that a new read model is indispensable.

## Over-specification
- Roughly half is removable: certificate lifecycle machinery, split-token protocol, repeated contract inventories, exact helper signatures and test names, and repeated descriptions of the same changes. These details expand mechanisms the reference explicitly rejects.

## Hand-off
- no: The primary Safari acceptance flow lacks working live updates, and certificate, authentication and static-serving decisions require substantial revision before implementation.

## Notes
- best: Concrete repository grounding and careful coverage of URL parsing, device naming and connection-thread static responses.
- worst: Extensive certificate and authentication engineering obscures the missing Safari polling fallback.