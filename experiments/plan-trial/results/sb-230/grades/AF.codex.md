| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| AF | 3 | 2 | 0 | 1 | no |

## Grounding
- `src/adapters/remote.rs:102-106`: found (empty SAN list and CommonName; no EKU or validity override).
- `facts, Settled 5`: found (discusses SAN requirements, but explicitly leaves click-through availability unsettled; it does not establish the plan’s categorical claim).
- `facts, Settled 1`: found (documents Safari’s self-signed WebSocket limitation and calls for a fallback).
- `src/app.rs:3023`: found (`strip_prefix("/api/v1/")` rejects other paths with an unauthenticated 404).
- `docs/remote-client.md:207`: found (incorrectly describes `last_message` as session metadata, exactly the discrepancy the plan identifies).

## Departures caught
- Safari WebSocket failure after click-through: caught; REST polling remains the base, with content watches when the socket works.
- Certificate download is not viable with today’s certificate: caught with a wrong fix; download is rejected, but certificate reissuance is introduced anyway, contrary to the reference.
- Parallel page files consume unauthenticated connection slots: missed; three files retain the four-slot limit, with an unjustified assumption that subsequent authenticated fetches promote the connections. No capacity is reserved for a second device.
- Device naming belongs in page script: caught; `navigator.userAgent` supplies `device_name`.
- Additional departures acknowledged in the reference’s errata: catches the absent excerpt, missing scope read after reload, and token exposure in the cookie-mode pairing body.
- Additional finding: same-site cookies cross port boundaries; the proposed Origin check addresses this without introducing a CSRF token.

## Decisions to overturn
- Remove certificate reissuance, persistence and renewal. Reference decision 1 explicitly keeps the existing certificate and click-through; the supplied facts do not establish that click-through requires this redesign.
- Replace three static files with two and address concurrent devices’ pool capacity. Reference decision 5 rejects the connection-budget assumption underlying this layout.
- Remove unrestricted `wss:` from CSP or restrict it to the actual host. Reference decision 5 confines connections to the serving origin; acknowledging that the script currently uses one destination does not enforce that restriction.

## Over-specification
- Roughly 40% is avoidable: the certificate storage/resolver/renewal subsystem and its tests, repeated contracts and decisions, extensive spike branches, and exhaustive route assertions. Useful detail on cookie authentication and revision handling does not require this much repetition.

## Hand-off
- No: resolve certificate scope, connection capacity and CSP first. Also explicitly require query removal with `history.replaceState` before any fetch; the plan never carries that essential pairing step into implementation instructions.

## Notes
- best: Poll-first updates, HttpOnly cookies and content watches fit the actual browser constraints.
- worst: An uncertain certificate premise expands into a substantial persistence and renewal subsystem that the reference explicitly rejects.