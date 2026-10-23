| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| Z | 3 | 2 | 0 | 1 | no |

## Grounding
- README: found (README.md documents the remote port, Copy invite and script commands).
- `docs/design.md`: found (the remote-client status section starts at line 3107).
- `facts.md`: found (the supplied facts text has the named headings; it is not owner-checked evidence).
- `control/src/lib.rs:637`: found (`SessionView` includes the listed status fields and has no excerpt).
- `src/ui/cards.rs:558`: found (`agent_excerpt` derives a shortened line from a conversation).

## Departures caught
- Safari WebSocket failure after click-through: caught with a wrong fix. Polling exists, but open-session content is fetched only when `/waiting`’s revision changes; content must be polled independently.
- Certificate download is not a usable alternative with the existing certificate: caught with a wrong fix. The plan introduces certificate reissue, expiry management and download instead of retaining click-through as the reference directs.
- Static files consume unauthenticated connection slots: caught. Closing static connections addresses slot retention and explicitly considers another device. Unlike the reference’s two-file limit, three files with immediate closure are defensible, although browser concurrency remains unverified.
- Device name must come from page script: caught through `navigator.userAgent` and the existing `device_name` body field.
- Missing wire excerpt: caught; deferral agrees with the reference erratum.
- Missing scope after reload, LAN invite while bound to loopback, and token exposure in cookie-pair response bodies: caught and addressed.
- Additional finding: cookie credentials reach other HTTPS ports on the same host; correctly acknowledged, but instance cookie-name collisions remain unresolved.

## Decisions to overturn
- Remove certificate reissue, identity-schema additions, renewal/rebinding and `/certificate`; these conflict with reference decision 1’s explicit no-download, no-reissue choice.
- Name the cookie using the server id instead of fixed `sb_session`; reference decision 2 prevents two installations overwriting each other’s credentials.
- Poll collections and open-session content directly, with the socket supplemental; reference decision 3 does not gate content refresh on the collection revision.
- Explicitly update the existing copied-invite JSON assertion in `tests/ui.rs`; reference decision 7 requires its conversion to a URL assertion, while the plan specifies only QR visibility testing there.

## Over-specification
- Roughly 40% is expendable: certificate lifecycle machinery and its tests, repeated contract summaries, repeated uncertainty discussions, unchanged-schema bookkeeping, and byte-pattern certificate checks. These obscure the comparatively small browser feature.

## Hand-off
- No: certificate scope, cookie isolation and polling correctness need substantive revision before implementation.

## Notes
- best: Thoroughly identifies missing wire data and preserves native pairing compatibility while keeping browser tokens out of script.
- worst: Turns an uncertain browser-trust premise into a certificate lifecycle subsystem contrary to the owner-checked decision.