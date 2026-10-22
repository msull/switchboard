| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| H | 3 | 1 | 0 | 1 | no |

## Grounding
- `src/ui/cards.rs` 176–185: found (agent captions come from `agent_excerpt`; other captions use pane captions or host titles).
- `agent_excerpt` 558–575: found (in `src/ui/cards.rs`; derives a shortened first line from the last answer, activity, or user prompt).
- `control/src/lib.rs` 637–660: found (`SessionView` includes `card`, `waiting`, and `waiting_reason`, but no excerpt).
- `src/app.rs` 2962: found (`listen.lan` selects all interfaces versus loopback).
- `src/ui/remote.rs` 157: found (the pairing window uses `local_address()` to construct the JSON invite address).

## Departures caught
- Safari rejects click-through-certificate WebSockets: missed as a settled constraint. The spike asks about it, but polling is conditional on the CSP check, while initial snapshots wait for WebSocket `hello`.
- Certificate download cannot provide the proposed trust path with this certificate: caught with a wrong fix. It notices certificate limitations and rejects download, but permits certificate reissue instead of retaining the identity design.
- Parallel asset connections consume the unauthenticated pool: caught with a wrong fix against the reference. It closes connections promptly but chooses three files and leaves second-device capacity unresolved pending a spike.
- Device naming belongs in browser script: caught; `navigator.userAgent` supplies `device_name`.
- Additional departures identified: missing list excerpt, loopback-only pairing invites, indistinguishable revocation/rotation responses, and no endpoint for recovering current device scope. These are useful findings, although their proposed remedies are not all necessary.
- Additional concern identified: query parameters reach the host before script clears them. This motivates a fragment URL, but conflicts with the reference’s chosen query contract.
- The existing JSON token response is an implementation change needed for cookie pairing, not evidence that the issue’s requested browser flow is impossible.

## Decisions to overturn
- Replace socket-gated initialization with immediate reads and polling, using WebSockets opportunistically; the reference requires usable Safari updates without a socket.
- Remove conditional certificate reissue; the reference explicitly retains the existing certificate and click-through trust step.
- Use `SameSite=Strict`; the reference names it as the CSRF defence. A cross-site initial navigation does not justify Lax for subsequent same-origin API calls.
- Restore the query-form invite specified by the issue and reference. The fragment argument is reasonable, but changes an explicitly settled contract.
- Reduce the page to HTML and script and settle concurrent-device capacity; the reference specifies at most two files.
- Preserve the requested list excerpts rather than moving them exclusively into the session view; the reference retains the issue’s first-cut presentation.
- Drop the new `/api/v1/device` read model unless its necessity is established; the reference keeps scope from pairing.

## Over-specification
- Roughly 40% could be cut: repeated decisions across implementation, UI, ordering, and contract sections; schema bookkeeping for unchanged models; literal-constructor counts; exhaustive test names and assertions; and lengthy justification of static assets containing no secrets.

## Hand-off
- no: Safari initialization can remain blocked indefinitely, and several settled reference decisions are replaced or deferred to an owner-run spike.

## Notes
- best: Strong code grounding, including useful discoveries about excerpts, scope recovery, and loopback invites.
- worst: Extensive detail leaves the central Safari live-update requirement unresolved.