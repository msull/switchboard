| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| R | 2 | 1 | 0 | 1 | no |

## Grounding
- `facts.md`: wrong (absent from the checked-out tree; several decisions depend on it).
- `control/src/lib.rs:637-666`: found (`SessionView` has `card`, `waiting_reason`, and no excerpt or `last_message`).
- `src/adapters/hooks.rs:71`: found (`last_message` on the hook input).
- `src/ports/events.rs:24`: found (`Stopped` carries `last_message`).
- `src/app.rs:3568-3600`: found (`remote_info` exposes host identity and capabilities, but no device scope).

## Departures caught
- Safari rejects the click-through certificate for WebSockets: missed. General uncertainty about certificate warnings does not identify this failure; initialization waits for socket `hello`, with no collection polling fallback.
- Certificate download is unsuitable with the existing certificate: missed. The deferred SAN-only reissue proposal omits the reference’s EKU, validity, identity-model and continuing Safari objections.
- Parallel static requests consume unauthenticated slots: caught with an incomplete fix. Closing connections reduces occupancy, but three assets and four slots leave second-device capacity unresolved.
- The page must supply the device name: caught; derives it from `navigator.userAgent`.
- Missing card excerpt, acknowledged in the reference errata: caught; `card` and `waiting_reason` are an accepted alternative.
- Missing scope discovery after reload, acknowledged in the errata: caught; `/me` also handles scope changes.
- Additional finding: correctly locates the existing device-name cap in `new_device`; this corrects the unavailable facts document, rather than the issue.

## Decisions to overturn
- Replace `localStorage` credentials with the reference’s opt-in, server-id-named HttpOnly cookie and `SameSite=Strict` defence.
- Replace socket-dependent startup with snapshots and short-interval polling, using the socket when available.
- Explicitly send and restore content `watch` subscriptions, as reference decision 3 requires.
- Reduce the page to two assets and settle second-device capacity, following reference decision 5. `Connection: close` is useful mitigation, but does not establish that simultaneous loads fit.

## Over-specification
- Roughly one-third is removable: repeated contract summaries, unavailable “facts” classifications, exact helper signatures, speculative owner alternatives, exhaustive source-string bans, and unrelated device-name sanitization. Concrete routing, authentication and integration-test details remain useful.

## Hand-off
- no: the proposed live-update design fails the reference’s primary target-browser constraint, and several implementation decisions need replacement.

## Notes
- best: identifies the missing excerpt and scope-discovery contracts with concrete code evidence.
- worst: treats browser compatibility as an acceptance uncertainty while making successful WebSocket establishment a prerequisite for the page.