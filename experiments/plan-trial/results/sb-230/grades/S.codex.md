| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| S | 3 | 2 | 0 | 1 | no |

## Grounding
- `facts/1/repo/facts.md`: found (checked against the supplied facts text, as instructed; it contains the cited protocol and certificate discussion).
- `src/core/model.rs:12`: found (`SCHEMA_VERSION` is 16).
- `src/core/model.rs:612`: found (`VIEWS_SCHEMA_VERSION` is 7).
- `src/ports/remote.rs:17`: found (`IDENTITY_SCHEMA_VERSION` is 1).
- `control/src/lib.rs:646`: found (`SessionView.card` is the status word; the struct has no excerpt field).

## Departures caught
- Safari rejects the click-through certificate for WebSockets: missed. A phone gate and reconnect loop do not supply the reference’s polling fallback.
- Certificate download is not viable with this certificate: missed. The plan leaves download as a future alternative and adds expiry-only reissue without resolving SAN/EKU requirements.
- Parallel static requests consume unauthenticated slots: caught with a wrong fix against the reference’s two-file limit. Raising the cap and closing static connections mitigate occupancy, but three files and the six-socket test do not establish capacity for a second device.
- Device naming belongs in page script: caught; uses `navigator.userAgent`.
- Missing card excerpt: caught, agreeing with the reference’s erratum.
- Scope unavailable after cookie-based reload: caught; adds an authenticated device endpoint.
- LAN invite while listening on loopback: caught; adds explanatory UI.
- Browser pairing still exposing the token in JSON: caught; omits it from browser replies.

## Decisions to overturn
- Replace socket-only live updates and the successful-WebSocket gate with polling plus optional WebSockets, following reference decision 3.
- Remove certificate expiry storage, automatic reissue and the added dependency, following reference decisions 1 and 7.
- Reduce the static page to HTML and script, following reference decision 5. Explicit connection closure is useful mitigation, but departs from the reference’s prescribed asset bound.

## Over-specification
- Roughly one-third could be cut: repeated contracts and implementation steps, compiler-forced literal updates, extensive owner questions reopening settled choices, and certificate machinery outside the reference’s scope.

## Hand-off
- No: the primary phone browser lacks working live updates, and the certificate lifecycle and static asset decisions need revision before implementation.

## Notes
- best: Identifies several real wire-contract gaps and specifies cookie pairing without exposing the token to script.
- worst: Treats Safari WebSocket support as an owner-run gate instead of designing the polling fallback required by the reference.