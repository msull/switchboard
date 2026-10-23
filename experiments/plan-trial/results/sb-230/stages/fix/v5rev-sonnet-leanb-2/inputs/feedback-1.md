# Correctness review, round 1 (head 47f139e)

`cargo test --locked --test remote` passes (53). The host side matches the plan. I found no schema, effect or auth defects. The cookie name, the Origin rule, the page fast path, the pool and window, `/me`, and the pair transport all behave as planned. The findings are in the page's refresh logic and one QR case.

## 1. The open session's text is never refreshed by the poll (web/app.js `tick`, `refresh`, `changed`)

- Content replies carry the revision of their own key (`observe` → `envelope_at`, src/app.rs ~1655). `/waiting`'s revision therefore moves only when the waiting content changes.
- `tick` reads only `/waiting` and redraws only when that mark moves. `/sessions/{id}/last-response` and `/screen` have separate keys.
- Failing state: a session is open and its agent or shell keeps producing output while the waiting set stays the same. A shell, a service, or a Claude session mid-turn all do this. The poll sees no change, so the text on screen stays frozen until the user goes back and reopens it.
- The plan's "a different one refetches the list and the open session" holds only when the waiting set happens to move.
- The socket path has the same hole. The plan relies on "the poll keeps [the content key] warm" for the 60 s watch (§4.4 item 3). That poll never reads the session key. After the first minute the key leaves the watch set and no `changed` frame arrives for it.
- Fix direction: while a session is open, `tick` also reads the session's content route, which renews the watch, and compares its revision with the one held. Alternatively, the page redraws the open session on every tick.

## 2. A refresh redraw wipes the reply box while the user types (web/app.js `changed` → `drawSession`)

- `drawSession` replaces all of `mainEl`. `changed()` runs on any `/waiting` move or any `changed` frame, including one for a different session.
- The text survives in `state.draft`. The textarea loses focus, though, so the phone keyboard drops mid-sentence.
- The `status` node held by the pending `send` is detached. A later "Sent. …" or error message is written to an element that is no longer on screen.
- Finding 1's fix makes this fire more often, so both need to be handled together. The simplest fix is to update only the `pre` text in place when the box is focused or non-empty.

## 3. The QR names loopback with no notice when the port listens on the network but no local address is found (src/ui/remote.rs ~164)

- `host` is `None` when `local_address()` finds nothing, for example with no route. `mac_only` is false.
- The URL then becomes `https://127.0.0.1:<port>/#…`. `address` is empty, so the window shows no address line and no Mac-only line.
- The user sees a QR that cannot work from a phone and no explanation. The plan's "127.0.0.1 otherwise, with a line saying the port is Mac-only" covers only the `listen.lan == false` case.
- Low severity. Show a "no network address found" line in that case.
