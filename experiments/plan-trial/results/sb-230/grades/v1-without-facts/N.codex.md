| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| N | 1 | 2 | 0 | 0 | no |

## Grounding
- `src/core/model.rs:12`: found (`SCHEMA_VERSION` is 16).
- `src/ports/remote.rs:17`: found (`IDENTITY_SCHEMA_VERSION` is 1).
- `src/ports/remote.rs:39-40`: found (`listen` has `#[serde(default)]`, supporting additive compatibility).
- `facts/1/repo/facts.md`: wrong (the file is absent from the supplied tree).
- `facts:46`: wrong (the next distinct line reference points into that absent facts file; its claimed finding cannot be checked).

## Departures caught
- Safari WebSocket failure after certificate click-through: caught, but treated as unresolved; five-second polling provides a fallback, although polling the open content is insufficiently explicit.
- Certificate download is unsuitable with the existing certificate: caught with a wrong fix. The plan introduces certificate reissuance and a download fallback against the reference’s explicit no-reissue decision.
- Static files consume scarce unauthenticated connections: caught with a wrong fix. Three files and four slots leave inadequate room for another device; increasing capacity is deferred to a spike.
- Device naming belongs in page JavaScript: caught; `navigator.userAgent` supplies `device_name`.
- Missing card excerpt, recognized in the reference errata: caught; the proposed `card`/`waiting_reason` substitute is acceptable.
- Missing scope discovery after reload and token exposure in cookie-pair replies, recognized in the errata: caught; `/device` and omitting the token address them.
- LAN invite versus loopback listener, recognized in the errata: missed; refreshing the certificate does not enable LAN listening.
- Additional findings: correctly identifies indistinguishable revoked/unknown tokens and unavailable screens for stopped sessions.

## Decisions to overturn
- Remove served-certificate persistence, renewal, address-triggered rebinding and certificate-download fallback; reference decision 1 explicitly retains the existing certificate and click-through.
- Honor the pairing window’s scope instead of forcing browsers to `read`; reference decision 6 and acceptance require ordinary device scope selection. The cap also relies on a client-controlled `cookie` flag.
- Restore the specified query URL; reference decision 4 chooses `?code=…&key=…&id=…`. Fragment transport has a defensible privacy advantage, but changes the agreed invitation contract.
- Serve constants on the connection thread, not the frame; reference decision 5 explicitly reverses frame-side static routing.
- Use two page files and settle capacity for a second device before implementation; reference departure 3 rejects leaving this to a later spike.
- Name cookies with server identity rather than port, as reference decision 2 specifies.
- Remove the conditional `localStorage` token fallback; reference decision 2 explicitly rejects that storage choice.
- Watch Claude conversation content as well as non-Claude screens, and explicitly poll open-session content during fallback; reference decision 3 requires both.

## Over-specification
- Roughly half is removable: certificate lifecycle machinery, speculative owner-dependent alternatives, repeated contracts across decisions/actions/tests/docs/commit, and low-level implementation prescriptions. These obscure the comparatively small page-and-cookie change.

## Hand-off
- no: several architectural and product decisions conflict with the reference, unavailable “facts” repeatedly justify choices, and essential connection-capacity and live-content behavior remain unsettled.

## Notes
- best: Finds the missing excerpt and reload-scope gaps, and prevents cookie pairing from exposing the token in JSON.
- worst: Expands certificate handling and changes scope semantics while leaving the intended phone experience dependent on follow-up decisions.