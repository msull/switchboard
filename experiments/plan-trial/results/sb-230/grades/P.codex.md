| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| P | 3 | 2 | 0 | 1 | no |

## Grounding
- `facts/1/repo/facts.md`: found (checked against the supplied facts text, as instructed; absent from the base tree).
- `src/ui/cards.rs:558`: found (`agent_excerpt` derives the card excerpt from the conversation).
- `control/src/lib.rs:637`: found (`SessionView` has `card`, `waiting` and `waiting_reason`, but no excerpt).
- `docs/remote-client.md:207`: found (describes a metadata `last_message` field absent from `SessionView`).
- `src/app.rs:3660`–`3665`: found (pairing reply includes token, scope, server ID and device ID).

## Departures caught
- Safari’s click-through does not enable WSS: caught, though presented as unresolved; polling provides a working fallback.
- Certificate download cannot fix today’s certificate: caught with a wrong fix—the conditional SAN reissue/download branch conflicts with the reference’s decision to preserve address-independent identity.
- Browser subresources consume scarce unauthenticated connections: caught with a wrong fix against the reference—three assets, connection closing and per-page queuing replace its two-file limit. Closing connections helps, but the plan does not establish capacity for a second simultaneous browser.
- Device naming belongs in page script: caught; the page derives and submits `device_name`.
- Additional departures, acknowledged in the reference’s errata: missing excerpt, missing returning-device scope endpoint, LAN invite while listening on loopback, and cookie pairing still exposing the token in JSON. All identified; the loopback problem is deferred.
- Additional observations: revoked and unpaired requests share a 401; `/info` reports a claimed key rather than browser-verified TLS identity. The active-relay qualification reasonably narrows the reference’s claim about protection against relayed codes.

## Decisions to overturn
- Remove the certificate reissue/download fallback; reference decision 1 chooses click-through without identity changes.
- Remove the localStorage bearer fallback; reference decision 2 explicitly rejects localStorage token storage.
- Make native text parsing accept URLs and make “Copy invite” copy the URL; reference decision 4 requires this, whereas the default plan leaves native URL pairing unsupported.
- Reduce the page to two assets and explicitly budget simultaneous devices; reference decision 5 and departure 3 reject the three-file arrangement.
- Fetch initial state independently of socket `hello`, with polling as the baseline; reference decision 3 specifies poll first, socket second.

## Over-specification
- Roughly 40% could be cut: fully written alternative implementations, repeated contract inventories, browser-policy exposition, exact retry constants, source-string prohibition tests and sections asserting that nothing changes. These obscure the choices an implementer actually needs.

## Hand-off
- no: Required native URL support is conditional, several fallback branches contradict the reference, and connection budgeting remains unsettled.

## Notes
- best: Identifies genuine wire-model gaps and provides concrete cookie, scope and revocation tests.
- worst: Turns decisions the issue requires settled into extensive owner-dependent implementation branches.