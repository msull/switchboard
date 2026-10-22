| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| v3-haiku-a | 3 | 2 | 0 | 1 | no |

## Grounding
- `inputs/facts.md`: found (checked against the supplied text as instructed; it contains the cited settled facts, unresolved questions and owner questions).
- `docs/remote-client.md:207`: found (incorrectly describes `last_message` as session metadata).
- `src/ports/events.rs:23-24`: found (`Stopped` carries optional `last_message`).
- `src/core/events.rs:132`: found (the `Stopped` match binds `pending` and ignores `last_message`).
- `src/ui/cards.rs:177-181`: found (agent captions derive from the cached conversation through `agent_excerpt`).

## Departures caught
- Safari rejects the click-through certificate for WebSockets: caught (polling supplies the required fallback, although the plan understates the reference’s finding as unverified).
- Certificate download requires changing the certificate model: caught with a wrong fix (rejects download but introduces certificate reissuance, contrary to the reference).
- Parallel static requests consume unauthenticated slots: caught (two served files and connection closure release slots; the three-stranger test checks remaining capacity).
- Device naming belongs in page JavaScript: caught (`navigator.userAgent` supplies `device_name`).
- Missing card excerpt, acknowledged in the reference errata: caught (correctly rejects the facts file’s claim and uses available metadata).
- Missing scope after reload, acknowledged in the errata: caught (adds authenticated `/device`).
- LAN invite versus loopback listener, acknowledged in the errata: missed.
- Token exposure in the cookie pairing response, acknowledged in the errata: missed.
- Additional findings: identifies indistinguishable revoked/unknown-token responses and qualifies the browser trust and history claims.

## Decisions to overturn
- Remove certificate reissuance, renewal actions and identity fields; reference decision 1 explicitly preserves the existing certificate model.
- Replace default `localStorage` and subprotocol authentication with the server-specific HttpOnly cookie requested by the browser; reference decision 2 chooses that transport and names `SameSite=Strict` as the CSRF defence.
- Explicitly send `watch` for the open session’s `conversation` or `screen`; reference decision 3 requires these subscriptions for content events.
- Update the existing JSON clipboard assertion in `tests/ui.rs` and add cookie pairing/read/revocation coverage; reference decision 7 requires these, but the plan omits them.
- Extend fixtures for the proposed wire view rather than deferring until Swift consumes it; reference decision 7 and the issue require fixture updates when views change.

## Over-specification
- Roughly 35–45% is dispensable: certificate lifecycle machinery, three simultaneous authentication transports, repeated contract summaries, literal-site counts, schema nonchanges and repeated facts-file contingencies. These obscure the smaller implementation the reference recommends.

## Hand-off
- no: certificate and authentication choices need redesign, and content subscriptions plus required regression coverage remain unspecified.

## Notes
- best: Correctly discovers the missing excerpt and scope, and makes REST polling the dependable live-update path.
- worst: Turns uncertain browser assumptions into certificate lifecycle machinery and a script-readable token default despite the reference’s simpler choices.