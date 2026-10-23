| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| AD | 3 | 2 | 0 | 1 | no |

## Grounding
- `facts/1/repo/facts.md`: found (checked against the supplied facts text as instructed; absent from the tree).
- `control/src/lib.rs:637-688`: found (`SessionView` has the named metadata fields and no excerpt).
- `src/ports/events.rs:24`: found (`Stopped` carries `last_message`).
- `src/adapters/hooks.rs:71`: found (`LogLine` contains `last_message`).
- `src/app.rs`: found (`last_message` has no reader here; remote routing and authentication are present).

## Departures caught
- Safari’s socket fails after certificate click-through: caught; polling makes the page usable, although the plan treats the limitation as unresolved.
- Certificate download is not viable with this certificate: caught with a wrong fix; SAN reissuance and a proposed download fallback contradict the reference and omit its EKU and validity concerns.
- Static files consume scarce unauthenticated connections: caught with a wrong fix; closing connections helps, but three files and assumed browser retries do not establish capacity for a second device.
- Device naming belongs in page script: caught; sends a name derived from `navigator.userAgent`.
- Missing wire excerpt: caught, consistent with the reference’s errata; deferring it is acceptable.
- Scope unavailable after reload, loopback/LAN invite mismatch, and token exposure in the cookie pairing response: caught; addressed explicitly.
- Additional departure beyond the reference: distinguishes clearing browser history from exposure in the initial request and scanner, and prevents referrer leakage.

## Decisions to overturn
- Remove certificate reissuance, `Identity.sans`, and its schema migration; reference decision 1 retains the existing certificate and uses click-through.
- Remove certificate download as the socket fallback; reference decisions 1 and 3 use polling.
- Replace three static files and retry assumptions with two files and explicit second-device capacity; reference decision 5 allows increasing the unauthenticated pool.
- Do not stop implementation merely because the cookie fails on a WebSocket upgrade, or prescribe `localStorage` as its remedy; reference decision 2 and the added erratum retain cookie-authenticated polling.

## Over-specification
- Roughly one-third is removable: repeated contract inventories, unchanged-model declarations, exact constructor bookkeeping, extensive spike procedure, and certificate migration machinery outside the reference’s chosen scope.

## Hand-off
- no: certificate lifecycle changes, the fallback policy, and unauthenticated connection capacity need correction before implementation.

## Notes
- best: Independently catches the missing excerpt and closes practical cookie-pairing gaps without exposing the token to script.
- worst: Turns certificate uncertainty into unnecessary identity migration and an unsuitable download fallback.