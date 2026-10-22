| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| Q | 1 | 2 | 0 | 1 | no |

## Grounding
- `facts.md`: wrong (no such file in the tree; its evidence and owner questions cannot be checked).
- `control/src/lib.rs:637`: found (`SessionView` has `card`, `waiting` and `waiting_reason`, but no excerpt).
- `docs/remote-client.md` §4.3 (`session.last_response` bullet): found (incorrectly describes `last_message` as session metadata, confirming the identified discrepancy).
- `facts:8`: wrong (the referenced facts file and line are unavailable).
- `src/app.rs:4105`: found (`caption_of` extracts the last nonempty pane line; attributing the agent-card excerpt to it is inaccurate—`agent_excerpt` in `src/ui/cards.rs:558` computes that).

## Departures caught
- Safari WebSocket failure after certificate click-through: caught, but presented as unresolved; polling eventually makes the page usable.
- Existing certificate unsuitable for device trust installation: caught; chooses click-through without certificate reissue.
- Parallel static requests consume unauthenticated slots: caught with a wrong fix relative to the reference’s two-file limit. Closing connections mitigates prolonged occupancy, but three files and four slots leave simultaneous-device loading insufficiently addressed.
- Device name must come from page script: caught.
- Missing wire excerpt: caught; the reference erratum supports the proposed reduced row.
- Scope unavailable after reload: caught; `/device` addresses it, although “cannot learn its scope from anything” overlooks the pair reply.
- Cookie pairing otherwise exposes the token in the response body: caught and addressed.
- LAN invite while listening on loopback: missed.
- Additional findings: indistinguishable revoked/unknown credentials, per-project session reads, and the Rust test client’s limited header parsing.

## Decisions to overturn
- Restore `/info` key comparison before sending the code; reference decision 1 explicitly requires it. Server rejection happens after disclosure.
- Poll immediately while attempting the socket; reference decision 3 says poll first, whereas this plan delays fallback through two failed attempts.
- Make “Copy invite” copy the URL; reference decision 4 retains JSON parsing compatibility, not two competing copy actions.
- Reduce static assets to two and address a second device’s connection budget, following reference decision 5.
- Preserve normal `DeviceSeen` updates. Reference decision 2 keeps authentication handlers unchanged; suppressing ordinary browser reads makes polling devices appear inactive without a proportionate benefit.
- Retain and compare host identity so identity changes receive their required distinct message; a generic ambiguous 401 does not satisfy the reference’s page behavior.

## Over-specification
- Roughly 40% is removable: unavailable owner-question branches, speculative future token fallback, repeated contract inventories, exhaustive unchanged-schema declarations, cookie lifetime machinery, and tests enforcing implementation spelling.

## Hand-off
- no: key verification, identity handling, polling startup, invite behavior and connection budgeting need correction before implementation.

## Notes
- best: Identifies the missing excerpt and reload-scope gaps and provides concrete cookie-authentication tests.
- worst: Deliberately removes the required pre-pair key comparison while surrounding that reversal with extensive speculative detail.