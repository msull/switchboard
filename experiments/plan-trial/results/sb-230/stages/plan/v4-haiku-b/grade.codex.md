| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| v4-haiku-b | 3 | 2 | 0 | 1 | no |

## Grounding
- `docs/remote-client.md:201,207`: found (redaction covers excerpts; §4.3 incorrectly describes `last_message` as session metadata).
- `src/ports/events.rs:24`: found (`Stopped` carries `last_message: Option<String>`).
- `src/core/events.rs:133-134`: found (stores `last_stop_at` and `pending_at_stop`, ignoring the message).
- `src/app.rs:888-898`: found (`cold_scrollback` reads pane output and stores a caption in UI state).
- `caption_of` at `:4105`: found (returns the last non-blank, trimmed line).

## Departures caught
- Safari WebSocket failure after click-through: caught; polling provides a working fallback, though the plan understates the reference’s evidence as uncertainty.
- Certificate download is unsuitable with the existing identity: caught with a wrong fix; introduces certificate reissue despite the reference’s explicit no-reissue decision.
- Static files consume unauthenticated connection slots: caught with a wrong fix against the reference; closing connections helps, but three files and deferred concurrency testing do not establish room for a second device.
- Device name must come from page script: caught; uses `navigator.userAgent`.
- Missing wire excerpt: caught; correctly contradicts the supplied facts and uses existing reads, consistent with the reference erratum.
- Scope unavailable after reload: caught; adds an authenticated device endpoint.
- LAN invite while listening on loopback: missed.
- Cookie pairing still exposing the token in JSON: caught; browser replies omit it.
- Additional findings: correctly distinguishes unauthorized from provably revoked and qualifies what URL cleanup and a genuine page’s key comparison guarantee.

## Decisions to overturn
- Remove certificate renewal, identity fields, schema bump and renewal action; reference decisions 1 and 5 preserve the existing certificate and address-independent identity.
- Register `conversation` or `screen` watches for the open session; reference decision 3 requires them. Deferring watches leaves content polling at 15 seconds even when the socket works.
- Reduce static assets to HTML and script and settle concurrent-device capacity; reference decision 5 specifies two files. Connection closure is useful mitigation, but does not justify leaving that capacity question to a follow-up.

## Over-specification
- Roughly 40% is removable: the unnecessary certificate lifecycle, repeated contract inventories, repeated decisions across implementation/tests/docs/order/follow-ups, exact helper signatures and mechanical literal-site edits.

## Hand-off
- no: remove the certificate redesign, complete socket subscriptions and settle pool capacity before implementation. Also correct the claim that another HTTPS service receiving the cookie lacks its token—the cookie contains that token.

## Notes
- best: Checks the excerpt claim against both code and the supplied facts, finding a real contradiction.
- worst: Turns a browser-page task into an unnecessary certificate and identity migration despite the reference’s explicit decision.