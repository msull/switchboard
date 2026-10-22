| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| G | 3 | 2 | 0 | 1 | no |

## Grounding
- `control/src/lib.rs:637`: found (defines `SessionView`, including card and waiting fields, without an excerpt).
- `src/ui/cards.rs:176-185`: found (agent captions come from transcripts; other captions come from cached captions, with a host-title fallback).
- `src/ports/events.rs:24`: found (`Stopped` carries optional `last_message`).
- `docs/remote-client.md:207`: found (incorrectly describes the excerpt as `last_message` stored on the record).
- `src/adapters/remote.rs:95-127`: found (`make_identity` uses an empty SAN list, sets no serverAuth EKU, and documents the default 4096 expiry).

## Departures caught
- Safari WebSocket failure after certificate click-through: caught with a wrong fix. Recognizes the risk but leaves it to a spike; initial reads follow `hello`, and fallback polls only waiting and open-session content, not all collections.
- Certificate download cannot work with the existing certificate: caught; chooses click-through without reissuing.
- Parallel static requests exhaust unauthenticated slots: caught with a wrong fix. Closing connections helps, but three files plus favicon and preconnect still leave unreliable capacity for a second device.
- Device name must come from page script: caught; explicitly uses `navigator.userAgent`.
- Additional departure: correctly identifies the missing wire excerpt and inaccurate specification.
- Additional departure: identifies that a query sends the code before the page checks identity. I favor its fragment proposal over the reference’s query, provided that deliberate contract change is accepted.

## Decisions to overturn
- Serve constants through `serve_remote` on the frame; reference decision 5 explicitly requires connection-thread routing.
- Ship three assets plus favicon with reload as the capacity remedy; reference decisions 5–6 require two files and adequate unauthenticated capacity.
- Gate initial snapshots on socket `hello` and incompletely poll on failure; reference decision 3 requires polling collections and open-session content independently of socket success.
- Replace the requested excerpt with a status word and defer it; reference decision 6 retains the requested first-cut view. The discovered mismatch is valid, but does not justify silently dropping that requirement.

## Over-specification
- Roughly one-third could go: repeated contract inventories, unchanged schema/signal sections, literal replacement counts, exhaustive helper mechanics, speculative hostname investigation, and source-text assertions that merely check implementation spelling.

## Hand-off
- no: correct static routing and connection capacity, make initial loading and complete refresh work without WebSocket success, and settle the excerpt requirement before implementation.

## Notes
- best: Finds and verifies the excerpt mismatch that the reference missed.
- worst: Its polling fallback does not reliably deliver the reference’s Safari-first working page.