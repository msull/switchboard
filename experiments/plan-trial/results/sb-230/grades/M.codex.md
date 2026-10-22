| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| M | 3 | 2 | 1 | 1 | note |

## Grounding
- `src/app.rs:3023-3025`: found (the frame rejects paths without the `/api/v1/` prefix).
- `src/ui/cards.rs:176-184,558-574`: found (excerpts come from conversation content or captions; `agent_excerpt` truncates to 160 characters).
- `docs/remote-client.md:254`: found (the watch set includes content keys read within the last minute).
- `src/app.rs:3568-3600`: found (`remote_info` returns host information, without the requesting device’s scope).
- `src/app.rs:3127-3153`: found (missing and unknown tokens both receive `401 unauthorized`).

## Departures caught
- Safari’s WebSocket failure after certificate click-through: caught with a wrong fix. The fallback polls collections but gates content refresh on session timestamps; screen and transcript changes need not move those timestamps. The later 30-second refetch instruction contradicts “never on a timer.”
- Certificate download requires changing the certificate model: missed as an explicit departure. Keeping the certificate and choosing click-through nevertheless matches the reference’s decision.
- Static assets consume scarce unauthenticated connections: caught. Three files depart from the reference, but immediate connection closure addresses its 30-second occupancy concern. I would accept this mitigation rather than automatically require two files.
- Device naming belongs in the page: caught; `navigator.userAgent` supplies `device_name`.
- Missing wire excerpt, acknowledged in the reference’s errata: caught; bounded `last-response` reads avoid adding transcript content to metadata.
- Missing scope discovery after reload, acknowledged in the errata: caught; the authenticated `device` endpoint addresses it.
- Additional findings: frame-side static-path rejection, indistinguishable authentication refusal codes, and limits of `replaceState` are identified. The claim of at most nine watched keys overlooks recently read keys retained after list or selection changes.

## Decisions to overturn
- Replace `localStorage` and token-bearing subprotocols with the reference’s opt-in, server-specific `HttpOnly; Secure; SameSite=Strict` cookie, preserving native bearer authentication.
- Make periodic collection and open-content reads the baseline, with WebSocket updates as an enhancement, following reference decision 3. Remove timestamp gating and resolve the contradictory refresh instructions.

## Over-specification
- Roughly one-third can go: repeated facts-file qualifications, browser standards exposition, exact helper and test names, unchanged schema declarations, and duplicated contract/documentation inventories. The conditional legacy-WebKit CSP mechanism also adds unnecessary implementation scope.

## Hand-off
- Use the reference’s cookie design and specify unconditional polling of visible content before implementation.

## Notes
- best: Concrete, correctly grounded handling of missing excerpts and scope discovery.
- worst: The fallback’s contradictory content-refresh rules undermine the primary Safari use case.