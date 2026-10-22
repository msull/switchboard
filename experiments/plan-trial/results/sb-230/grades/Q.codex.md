| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| Q | 2 | 2 | 0 | 1 | no |

## Grounding
- `facts.md`: found (supplied in the prompt; its claims are evidence to check, not owner decisions).
- `control/src/lib.rs:637`: found (`SessionView` has `card`, `waiting` and `waiting_reason`, but no excerpt).
- `docs/remote-client.md` §4.3 (`session.last_response` bullet): found (incorrectly describes `last_message` as session metadata available on the wire).
- `facts:8`: found (supports the missing-field finding, but its attribution of the card excerpt is inaccurate).
- `src/app.rs:4105`: wrong (the function exists, but `caption_of` extracts a pane’s last nonempty line; the agent-card excerpt is computed by `agent_excerpt` in `src/ui/cards.rs:558`).

## Departures caught
- Safari WebSocket failure after certificate click-through: caught, but described as unresolved; polling eventually provides the required fallback.
- Certificate download requires reissuing the current certificate: caught; correctly chooses click-through without reissue.
- Parallel static requests consume unauthenticated connection slots: caught with a wrong fix relative to the reference. Closing connections reduces occupancy, but three files and four slots still lack a convincing allowance for simultaneous devices.
- Device naming belongs in page script: caught; sends `device_name` from browser-derived information.
- Missing wire excerpt, acknowledged in the reference errata: caught; omitting it is an acceptable departure.
- Missing scope discovery after reload and token exposure in cookie-pair responses, acknowledged in the errata: caught and addressed.
- LAN invite despite loopback listening, acknowledged in the errata: missed.
- Additional findings: correctly identifies indistinguishable revoked/unknown credentials, project-by-project session reads, and the test HTTP helper’s missing response headers. “The page does not compare the key itself” is an unjustified departure, not a code limitation.

## Decisions to overturn
- Restore `/info` key comparison before transmitting the code, as reference decision 1 requires. Server-side rejection occurs after disclosure and is not equivalent.
- Start reads and polling immediately, adding the socket opportunistically, as decision 3 requires; two failed five-second handshake attempts should not gate the initial snapshot.
- Use two static files and explicitly accommodate a second device, following decision 5. Connection closure is useful but does not guarantee admission during concurrent loads.
- Make “Copy invite” copy the URL, retaining JSON parsing compatibility, as decision 4 specifies; remove the redundant copy choices.
- Preserve authenticated-read `DeviceSeen` behavior. Decision 2 keeps authentication and handlers unchanged; suppressing it makes actively polling devices appear inactive.
- Preserve enough nonsecret identity metadata to distinguish an identity change from generic pairing loss, rather than dropping the issue’s explicit identity-changed state.

## Over-specification
- Roughly 40% is removable: repeated contract inventories and implementation ordering, unbuilt alternatives tied to unanswered owner questions, exact helper signatures and literal counts, speculative future token storage, and extensive cookie side-effect machinery. Useful protocol and testing detail is diluted by these repetitions.

## Hand-off
- No: the plan deliberately removes pre-disclosure key verification and still needs corrections to startup behavior, connection capacity, identity reporting and invite UX.

## Notes
- best: Concrete cookie, revocation, revision and content-watch handling, with strong host-side integration coverage.
- worst: Treats sending the code to a mismatched host as equivalent to refusing to send it.