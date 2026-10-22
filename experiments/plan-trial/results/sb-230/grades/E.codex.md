| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| E | 3 | 1 | 0 | 1 | no |

## Grounding
- `control/src/lib.rs:637-688`: found (the complete `SessionView` has no excerpt or `last_message` field).
- `docs/remote-client.md` §4.3 (line 207): found (the text incorrectly describes `last_message` as session metadata supplying the excerpt).
- `src/app.rs:1093-1096`: found (`refresh_captions` and its comment describe excerpts from transcripts for visible cards).
- `client/Sources/SwitchboardUI/BrowserView.swift:741-746`: found (rows show kind, card state and waiting reason).
- `src/app.rs:3660-3665`: found (pairing returns token, scope, server ID and device ID).

## Departures caught
- Safari refuses the socket after certificate click-through: caught with a wrong fix. Treated as a spike contingency; 30-second visible-view polling does not clearly refresh both collections and open content. Initial reads also wait for `hello`, and content reads for `watching`.
- Certificate download requires reissue: caught. Identifies SAN and validity problems and chooses click-through, although it omits the EKU requirement.
- Parallel static requests consume scarce unauthenticated connections: caught with a wrong fix. Three files plus a favicon retain the four-slot pool without explaining capacity for a second device.
- Device naming belongs in the page: caught. Uses `navigator.userAgent` and the existing `device_name` body field.
- Additional departures: correctly identifies the missing wire excerpt, lack of current device scope in `/info`, and loopback invites advertising a LAN address. The excerpt finding justifies departing from the reference’s assumption that the requested page needs no view adjustment.

## Decisions to overturn
- Replace `localStorage` and token subprotocol authentication with the reference’s optional server-ID-named HttpOnly cookie, retaining bearer headers and naming `SameSite=Strict` as the CSRF defence. Port isolation is a real tradeoff, but the reference explicitly rejects script-readable token storage.
- Make short-interval collection and content polling independent of socket establishment, with WebSocket enhancement afterward, as the reference requires.
- Reduce the page to two files and explain remaining connection capacity, or increase the unauthenticated pool as permitted by the reference.

## Over-specification
- Roughly one-third is removable: repeated contract inventories, unchanged schema/action/signal sections, exact implementation signatures, exhaustive lexical JS checks, rigid API-table formatting, and repeated UI/test/doc descriptions.

## Hand-off
- No: authentication storage, Safari startup and refresh behavior, and concurrent page-load capacity require substantive revisions before implementation.

## Notes
- best: Finds genuine protocol/documentation mismatches beyond the reference, particularly the absent session excerpt.
- worst: Its claimed Safari fallback remains dependent on socket milestones and does not reliably deliver the central live-update acceptance behavior.