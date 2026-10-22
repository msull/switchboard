| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| B | 1 | 1 | 0 | 2 | no |

## Grounding
- `model.rs`: wrong (no such root-relative file; `src/core/model.rs` contains the durable model and schema version 16).
- `action.rs`: wrong (no such root-relative file; `src/core/action.rs` contains the state-machine contract and dispatcher).
- `ports/`: wrong (no such root-relative directory; `src/ports/` contains the capability interfaces). These three are recognizable shorthand, but fail the stipulated relative-path check.
- `control/src/lib.rs:637`: found (`SessionView` has kind, card, waiting status and reason, but no excerpt).
- `src/ui/cards.rs:558`: found (`agent_excerpt` derives transcript text, with activity/user fallbacks, and truncates at 160 characters).

## Departures caught
- Safari rejects the click-through certificate for WebSockets: missed. Waiting for `hello` before fetching collections prevents initial loading; thirty-second content refresh does not provide the reference’s collection polling fallback.
- Certificate download cannot supply the proposed trust route: caught. Identifies absent SANs and address binding, although omits EKU and validity constraints.
- Parallel static-file connections exhaust the unauthenticated pool: missed. Three files retain the four-slot pool without accounting for a second device; a request-rate window does not solve connection occupancy.
- Browser device name must come from the page: caught. Uses `navigator.userAgent` to populate `device_name`.
- Additional departure absent from the reference: correctly identifies the missing session excerpt. Reading existing content avoids a new view, but limiting excerpts to waiting Claude sessions reduces the requested coverage.

## Decisions to overturn
- Socket-gated initial loading: replace with collection and open-content polling plus opportunistic WebSockets, as reference decision 3 requires.
- Three static files with four unauthenticated slots: consolidate to two files and address second-device capacity, following reference decision 5.
- Static responses inside `serve_remote`: move constants to the connection thread, as reference decision 5 requires; avoiding state access still leaves frame-side routing.
- Fragment-only invites: conflicts with reference decision 4’s query URL. The fragment has a defensible security advantage, but this changes the expressly selected URL contract without supporting its query form.
- Waiting-only excerpts: conflicts with reference decision 6’s requested session-card coverage. The missing field warrants an implementation adjustment, not silently narrowing that coverage.

## Over-specification
- Roughly one quarter is expendable: empty record/signals/actions sections, repeated contract and header inventories, exact helper/type designs, and the prescribed commit message. Most security and integration-test details are useful.

## Hand-off
- no: the primary phone-browser flow can stall before showing sessions, and connection capacity and static routing need redesign before implementation.

## Notes
- best: Concrete cookie pairing, native compatibility, revocation tests, and discovery of the missing excerpt field.
- worst: Claims the remaining issue assumptions hold while missing the Safari WebSocket failure that breaks the central acceptance flow.