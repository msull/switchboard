| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| R | 3 | 1 | 0 | 1 | no |

## Grounding
- `facts.md`: found (the supplied facts text; its settled/unresolved/owner categories match the citations).
- `control/src/lib.rs:637-666`: found (`SessionView` contains `card`, `waiting`, and `waiting_reason`, but no excerpt).
- `src/adapters/hooks.rs:71`: found (`last_message` in the hook payload).
- `src/ports/events.rs:24`: found (`last_message` in the `Stopped` event).
- `src/app.rs:3568-3600`: found (`remote_info` returns host identity and capabilities, without device scope).

## Departures caught
- Safari’s click-through does not enable WSS: missed; treats browser support as unresolved and waits for socket `hello` before snapshotting. The 30-second content refresh does not provide collection polling.
- Certificate download is not viable with this certificate: missed; proposes a later SAN-only reissue/download fallback without addressing EKU, validity, or persistent Safari WSS restrictions.
- Parallel static requests consume unauthenticated slots: caught with a wrong fix against the reference; three files and connection closing mitigate occupancy, but do not establish capacity for a second device.
- Device naming belongs in page script: caught; derives `device_name` from `navigator.userAgent`.
- Missing card excerpt, acknowledged in reference errata: caught; reasonably substitutes `card` and `waiting_reason`.
- Missing scope discovery after reload, acknowledged in reference errata: caught; adds `/me` and handles subsequent scope changes.
- Additional correction to supplied facts: correctly finds the existing 64-character name cap in `new_device`.

## Decisions to overturn
- Replace localStorage/subprotocol credentials with the reference’s opt-in, server-id-named HttpOnly cookie and explicit SameSite defence.
- Make snapshots and short-interval polling work independently of socket establishment; reference decision 3 requires polling first.
- Explicitly send content `watch`/`unwatch` frames and restore watches on reconnect, as reference decision 3 requires. Periodic reads partially mitigate this omission but do not implement the specified protocol.
- Reduce the page to two served files and explain second-device capacity, following reference decision 5.
- Remove the proposed certificate-download fallback; reference decision 1 retains click-through without reissue.

## Over-specification
- Roughly one-third is expendable: repeated contract/commit summaries, speculative owner fallback branches, exhaustive source-string bans, unrelated device-name sanitization, and implementation details repeated across architecture, tests, and documentation.

## Hand-off
- No: the primary phone-browser acceptance path can stall before its initial snapshot, and live updates lack the required polling fallback.

## Notes
- best: Carefully verifies the missing excerpt and scope-discovery gaps, and corrects the facts file’s device-name claim.
- worst: Treats the decisive Safari WSS limitation as an unresolved acceptance check while making initial loading depend on WSS.