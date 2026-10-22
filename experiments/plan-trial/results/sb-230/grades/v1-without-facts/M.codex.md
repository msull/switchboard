| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| M | 3 | 2 | 1 | 1 | no |

## Grounding
- `src/app.rs:3023-3025`: found (frame-side prefix check refuses non-API paths with 404).
- `src/ui/cards.rs:176-184,558-574`: found (cards derive captions from conversation or pane information; `agent_excerpt` selects and truncates text to 160 characters).
- `docs/remote-client.md:254`: found (watch set includes explicitly watched keys and content keys read within the last minute).
- `src/app.rs:3568-3600`: found (`remote_info` returns host identity and capabilities, without the requesting device’s scope).
- `src/app.rs:3127-3153`: found (missing and unknown tokens both receive 401 `unauthorized`).

## Departures caught
- Safari WebSocket failure after certificate click-through: caught with a wrong fix. The plan calls it unresolved and supplies fallback, but gates polling of open content on session timestamps rather than periodically reading that content.
- Existing certificate cannot support the proposed certificate-download trust option: missed. Choosing click-through avoids reissue, but the plan never identifies the SAN, EKU and validity constraints.
- Static resources consume unauthenticated connection slots: caught. It closes static connections immediately. I disagree with treating the reference’s two-file limit as mandatory under this alternative; closing addresses the prolonged slot occupancy, although second-device capacity remains insufficiently demonstrated.
- Device naming belongs in the page: caught; explicitly derives `device_name` from `navigator.userAgent`.
- Missing wire excerpt: caught, consistent with the reference’s errata; uses existing redacted content reads.
- Missing scope discovery after reload: caught; adds an authenticated device endpoint.
- Indistinguishable revoked and unknown tokens: additionally caught; explains the page’s inference.
- Frame-side static refusal and browser trust/history limitations: additionally identified.

## Decisions to overturn
- Replace `localStorage` plus token-bearing WebSocket subprotocol with the reference’s opt-in, server-id-named HttpOnly cookie, retaining native bearer authentication and naming SameSite=Strict as the CSRF defence.
- Make collection and open-content polling the baseline, with sockets as an enhancement. The reference requires periodic content reads; session timestamps are not content versions. The plan’s later 30-second refresh instruction conflicts with its explicit “never on a timer” fallback rule.

## Over-specification
- Roughly 35–45% could be cut: repeated contract inventories, unchanged schema declarations, exact helper/test names, extensive standards discussion, speculative legacy-WebKit accommodations, and repeated watch-set accounting. References to unavailable “Settled” and “For the owner” material also obscure the standalone instructions.

## Hand-off
- no: token storage needs reversal, and the contradictory fallback instructions leave the primary target device’s content freshness unresolved.

## Notes
- best: Finds the missing excerpt and scope fields and proposes concrete solutions using existing protocol mechanisms.
- worst: Elaborate browser machinery still leaves the essential polling behavior contradictory.