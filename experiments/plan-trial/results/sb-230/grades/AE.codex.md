| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| AE | 2 | 2 | 0 | 1 | no |

## Grounding
- `facts.md`: found (supplied in the prompt; contains the numbered settled facts, unresolved questions and owner questions cited).
- `src/ui/cards.rs:558-574`: found (`agent_excerpt` derives and truncates an excerpt from the transcript).
- `:176-180`: found (in `src/ui/cards.rs`, the caption selects the agent excerpt or pane caption; the host-title fallback follows).
- `control/src/lib.rs:637-678`: found (`SessionView` contains card state and waiting metadata, with no excerpt field).
- `docs/remote-client.md` §4.4: wrong (the `last_message` claim is at line 207 in §4.3; §4.4 describes WebSocket events).

## Departures caught
- Safari click-through does not enable the socket: caught; polling supplies updates and the socket is optional.
- Certificate download is not a workable alternative with the existing certificate: caught with a wrong fix; adds SAN reissuance and incorrectly promises that later certificate installation gives Safari a socket.
- Parallel static fetches consume scarce unauthenticated slots: caught with a wrong fix; closing connections helps, but four assets still leave no guaranteed capacity for a second device. The reference calls for two files and explicit second-device headroom.
- Device naming belongs in page JavaScript: caught.
- Missing wire excerpt: caught; deferral agrees with the reference erratum.
- Missing scope after reload: caught; adds authenticated `/me`.
- LAN invite address while listening on loopback: caught; selects the loopback address and explains its limitation.
- Cookie pairing would otherwise expose the token in JSON: caught; omits it for cookie pairing.
- Additional finding: distinguishes unknown cookie credentials from absent credentials to support the revoked state.

## Decisions to overturn
- Replace the fixed `__Host-switchboard` cookie name with a server-specific name; reference decision 2 prevents instances on different ports overwriting each other.
- Remove certificate reissuance, persisted addresses and the associated schema/action machinery; reference decision 1 explicitly retains the existing certificate and identity model.
- Reduce the four static assets to two and address simultaneous devices; reference decision 5 specifies `/` and `/app.js`.
- Remove the promise that downloading and trusting the certificate restores Safari WebSockets; the reference explicitly rejects that guarantee.
- Restrict `connect-src` to the host rather than every `wss:` destination; reference decision 5 requires same-origin connections.

## Over-specification
- Roughly one-third is removable: repeated contract summaries, mechanical struct-literal counts, certificate migration machinery, repeated owner contingencies, and source-text tests that prescribe implementation spelling rather than behavior.

## Hand-off
- no: cookie collisions, certificate scope expansion and connection-budget choices require substantive corrections before implementation.

## Notes
- best: Poll-first updates, content watches and several verified protocol gaps are handled explicitly.
- worst: It recognizes cookie port sharing but still chooses a cookie name that collides across host instances.