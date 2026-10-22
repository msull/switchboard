| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| v3-haiku-b | 3 | 2 | 0 | 1 | no |

## Grounding
- `docs/remote-client.md` §4.3 (line 207): found (incorrectly describes the card excerpt as `last_message` on the session record, exactly as the plan reports).
- `src/ports/events.rs:24`: found (`Stopped` carries `last_message: Option<String>`).
- `src/core/events.rs:132`: found (`Stopped { pending, .. }` records the stop time and pending work, ignoring `last_message`).
- `control/src/lib.rs`: found (`SessionView` has `card`, waiting metadata and `last_stop_at_ms`, but no excerpt).
- `client/Sources/SwitchboardUI/BrowserView.swift:149-163`: found (collects last responses through `manager.lastResponse`; relevant support for the proposed excerpt source).

## Departures caught
- Safari WebSocket failure after certificate click-through: caught (described too tentatively as unresolved, but polling makes the page functional).
- Certificate download is unsuitable with the existing certificate: caught with a wrong fix (introduces SANs, expiry and certificate renewal instead of retaining click-through without reissue).
- Parallel page requests consume unauthenticated slots: caught (two files and immediate connection closure address retention; capacity for a second device remains insufficiently explicit).
- Device naming belongs in page script: caught (script supplies `device_name`; no host-side user-agent parsing).
- Missing excerpt on the wire: caught, including correcting the supplied facts file; consistent with the reference’s errata.
- Scope unavailable after reload: caught and addressed with an authenticated device endpoint.
- Indistinguishable revoked and unknown tokens: additionally identified, with appropriate user-facing wording.
- Browser key comparison cannot authenticate an impostor-served page: additionally identified and documented.

## Decisions to overturn
- Certificate reissue, renewal and identity fields conflict with reference decision 1: retain the existing certificate and make click-through the trust step.
- `localStorage` bearer persistence conflicts with decision 2: use an explicitly requested, server-id-named `HttpOnly; Secure; SameSite=Strict` cookie while preserving native bearer authentication.
- Omitting socket `watch` frames conflicts with decision 3: subscribe to the open session’s conversation or screen. Polling preserves functionality, but the advertised socket upgrade cannot deliver content changes as specified.
- CSP `connect-src 'self' wss:` conflicts with decision 5’s same-origin restriction; remove permission to connect to arbitrary secure WebSocket origins.

## Over-specification
- Roughly one-third is avoidable: detailed certificate-renewal contracts and tests, repeated decisions across build order and contract summaries, speculative future options, and exact helper signatures. The certificate subsystem is particularly substantial work the reference explicitly excludes.

## Hand-off
- No: revise credential storage, remove certificate lifecycle changes, and complete socket subscriptions and same-origin CSP before implementation.

## Notes
- best: Precisely catches the missing excerpt and corrects the planner’s own supplied facts.
- worst: Turns browser uncertainty into unnecessary certificate machinery and script-readable token persistence.