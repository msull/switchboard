| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| J | 3 | 2 | 0 | 1 | no |

## Grounding
- `src/core/model.rs:12`: found (`SCHEMA_VERSION` is 16).
- `src/core/model.rs:612`: found (`VIEWS_SCHEMA_VERSION` is 7).
- `src/ports/remote.rs:17`: found (`IDENTITY_SCHEMA_VERSION` is 1).
- `src/app.rs:4252`: found (`REMOTE_PROTOCOL` is 1).
- `docs/remote-client.md:207`: found (incorrectly describes the card excerpt as `last_message` stored on the record, exactly as the plan reports).

## Departures caught
- Safari rejects the click-through certificate for WebSockets: missed. Calling Safari behavior unverified and scheduling a phone check does not supply the reference’s required polling fallback.
- Certificate download requires a different certificate: caught. The plan identifies missing SANs and other trust requirements and selects click-through, although its conditional reissue conflicts with the reference.
- Parallel page files consume unauthenticated slots: caught with a wrong fix. Closing static connections addresses lingering occupancy, but three files leave less concurrent capacity than the reference’s two-file limit. It also cannot make a page load succeed while four strangers already occupy the slots.
- Device naming belongs in page script: caught; the page supplies `device_name`.
- Additional departure: correctly finds that the documented excerpt is absent from both the record and wire view. Its description of `agent_excerpt` omits activity/user-text fallbacks.
- Additional departure: identifies the missing authenticated scope read after reload; `/device` addresses this, though persisting the pair reply’s scope would be smaller.
- Additional departure: identifies frame-side routing and the loopback invite’s unreachable LAN address.

## Decisions to overturn
- Replace socket-only updates with polling plus optional WebSocket updates, as reference decision 3 requires.
- Remove the conditional SAN reissue; reference decision 1 explicitly preserves the existing certificate and identity model.
- Reduce the page to two files and specify second-device capacity, following reference decision 5.
- Restore excerpt coverage beyond eight waiting Claude sessions; reference decision 6 retains the issue’s session-card requirements. The discovered wire gap is real, but does not justify silently reducing that scope.

## Over-specification
- Roughly 35–45% could be cut: repeated contracts across decisions, implementation, tests, docs and commit-message instructions; exhaustive construction-site edits; unchanged-schema discussion; and detailed cookie-renewal machinery.

## Hand-off
- no: Safari live updates remain unsupported, and certificate, connection-capacity and excerpt decisions need revision before implementation.

## Notes
- best: Finds a real excerpt documentation/code mismatch absent from the reference.
- worst: Treats Safari WebSocket failure as a manual-check risk instead of designing the required fallback.