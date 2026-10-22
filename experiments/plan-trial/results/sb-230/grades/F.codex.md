| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| F | 3 | 1 | 0 | 1 | no |

## Grounding
- `src/ui/cards.rs:177`: found (agent captions use `agent_excerpt` over the conversation cache; other captions use UI state).
- `remote-client.md` §4.3 (line 207): found (the spec describes `last_message` as session metadata supplying the card excerpt).
- `src/ports/events.rs:24`: found (`Stopped` contains `last_message: Option<String>`).
- `control/src/lib.rs:637`: found (`SessionView` has card, waiting and timing fields, but no excerpt or `last_message`).
- §5, line 316: found (`docs/remote-client.md` claims later replies carry the device’s scope).

## Departures caught
- Safari cannot open the clicked-through certificate’s WebSocket: missed. The plan acknowledges uncertainty but assumes success and stops implementation on failure; its thirty-second content refresh does not replace socket-dependent initialization or collection updates.
- Certificate download cannot provide the proposed trust path: caught. Missing certificate names and address dependence justify choosing click-through, though EKU and validity constraints are omitted.
- Parallel assets consume scarce unauthenticated connections: caught with a wrong fix against the reference. Closing connections addresses retention, but three files and assumed browser retries leave simultaneous-device admission unresolved.
- Device naming belongs in the page’s pairing body: missed. The plan instead adds host-side User-Agent retention and parsing.
- Additional departure: correctly identifies the missing wire excerpt; deferring it nevertheless drops an explicitly requested feature.
- Additional departure: correctly identifies that scope is returned at pairing rather than on every authenticated response.
- Additional observation: correctly identifies frame-side rejection of non-API paths and routes static assets on the connection thread.

## Decisions to overturn
- Replace socket-gated startup and reopening the issue on Safari failure with the reference’s polling-first implementation and optional socket.
- Use two embedded files and explicitly accommodate a second device, following the reference’s static-path decision.
- Send `device_name` from `navigator.userAgent`, following the reference rather than extending host header parsing.
- Restore the requested list excerpt or explicitly settle its scope reduction; the reference retains the issue’s first-cut page requirements.
- Remove the mandatory custom CSRF header and authentication branching in favor of the reference’s cookie-to-bearer normalization, Strict cookie and optional Origin check.
- Reconsider `/device` against the reference’s retained pairing scope. I disagree with the reference here: the discovered scope-refresh gap makes this small authenticated read defensible, so I would retain it rather than count it as another reversal.

## Over-specification
- Roughly one-third is removable: unchanged schema inventories and speculative migration instructions, repeated contract summaries, brittle source-text lint tests, a test-driven API-table layout, and detailed cookie-lifetime and conditional CSP prescriptions. The additional CSRF machinery also exceeds the reference’s chosen design.

## Hand-off
- No: the plan knowingly leaves the primary iPhone acceptance path contingent on a WebSocket behavior the reference identifies as unsupported, while also changing naming architecture and dropping the list excerpt.

## Notes
- best: Finds two concrete spec/code discrepancies beyond the reference, with accurate anchors.
- worst: Treats Safari’s known WebSocket limitation as a reason to stop rather than designing the required polling fallback.