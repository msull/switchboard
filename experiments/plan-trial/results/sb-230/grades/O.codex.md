| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| O | 3 | 1 | 0 | 1 | no |

## Grounding
- `facts.md`: found (checked against the supplied text, as instructed; its Settled, Unresolved and For the owner sections exist).
- `src/adapters/remote.rs:444`: found (`read_request` splits the request target into path and query and retains both).
- `docs/remote-client.md` §4.3: found (the `session.last_response` paragraph incorrectly describes `last_message` as session metadata).
- `src/ports/events.rs:24`: found (`Stopped` carries `last_message: Option<String>`).
- `src/adapters/hooks.rs:71`: found (`LogLine` contains `last_message: Option<String>`).

## Departures caught
- Safari’s WebSocket failure after certificate click-through: missed as an established constraint; explicitly accepts an unusable page instead of providing polling.
- Certificate download is not viable with the existing certificate: caught with a wrong fix; identifies certificate deficiencies but introduces reissuance contrary to the reference.
- Parallel static-file connections exhaust unauthenticated slots: caught with a wrong fix; ships three files against four slots and leaves connection closing conditional, without establishing capacity for a second device.
- Device naming belongs in the page: caught; uses `navigator.userAgent` and the existing `device_name` body field.
- Missing excerpt on the wire: caught, matching the reference erratum; bounded last-response reads are a reasonable remedy.
- Missing scope discovery after pairing: caught, matching the erratum; `/me` handles reloads and scope changes.
- LAN invite while listening on loopback: caught, matching the erratum; uses loopback and explains the limitation.

## Decisions to overturn
- Accepting failure on Safari instead of polling collections and open-session content; conflicts with reference decision 3 and the target-device acceptance.
- Reissuing certificates, adding identity fields and renewal logic; conflicts with reference decision 1’s explicit no-reissue choice.
- Subprotocol authentication with a token in `localStorage`; conflicts with reference decision 2’s server-specific HttpOnly cookie.
- Three static files with four retained unauthenticated slots; conflicts with reference decision 5 and its two-device capacity requirement.
- Fragment invitations instead of the prescribed query URL; conflicts with reference decision 4. The fragment has a defensible privacy advantage, but it remains a deliberate contract departure under this yardstick.

## Over-specification
- Roughly 40% is removable: repeated Facts qualifications and owner-dependent fallbacks, certificate migration and call-site inventories, exact test recipes, a separate pool spike, cosmetic dimensions, and a commit-contract section repeating earlier decisions.

## Hand-off
- no: The plan knowingly leaves the principal target browser unsupported and requires substantial changes to trust, authentication and connection budgeting before implementation.

## Notes
- best: Carefully identifies the missing excerpt and scope-discovery contracts, with concrete remedies.
- worst: Treats Safari’s live-update failure as an acceptable limitation rather than implementing the reference’s polling fallback.