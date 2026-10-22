| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| H | 3 | 3 | 1 | 2 | note |

## Grounding
- `src/ui/cards.rs` 176–185 picks the caption; `agent_excerpt` 558–575: found (caption chosen at 177–185 from `agent_excerpt` for agents; `agent_excerpt` is exactly 558–574)
- `control/src/lib.rs` 637–660 (`SessionView` has `card`, `waiting`, `waiting_reason`, no excerpt): found (struct at 637; `card` 646, `waiting` 657, `waiting_reason` 658; no excerpt field)
- `src/app.rs` 2962 (port binds loopback with lan off): found (the `if identity.listen.lan` bind choice is at 2962–2966)
- `src/ui/remote.rs` 157 (QR carries `local_address()` regardless of lan): found (157–159, no lan check in the window)
- `src/core/remote.rs` 46–52 (`OpenPairing` has no lan check): found (46–54, uppercases the code, no lan condition)
- Beyond the five: `authenticate` 3127, the 401 text at 3142–3147, the token in the pair body at 3656–3661, the allowlist at 3023–3025, the 8-per-pass drain at 1584–1596, `Limits` 623–640, the 429/503 `continue` at 789–796 and 819–824, `Info.swift` 113–116, `main.swift` 107–112, `ConnectionManager.pair` 471/482, spec lines 123, 306, 309, 312–313, `tests/ui.rs` 7424, `Listen::default` lan false at 66 all hold. The count of ten literal `RemoteReply { … }` sites in `app.rs` (1664, 1679, 2365, 2371, 2376, 2754, 3570, 3723, 3736, 3767) and eighteen return-type matches is exactly right.

## Departures caught
- Safari refuses `wss://` to a click-through certificate: caught conditionally. Spike item 1 asks whether the exception covers `wss://` and step 5 falls back to five-second polling if it does not. It is not asserted, and the page design opens the socket first and waits for `hello` before snapshotting, which is the wrong baseline for the device the page is for.
- Certificate download is not an option: caught. The plan picks the click-through, declines a download, gives the iOS profile-install reason, and notes a re-issue over the same key leaves every pin intact if Safari refuses the leaf outright.
- Each page file is a connection holding an unauthenticated slot: caught with a different fix. Three files plus a favicon 404, but every unauthenticated reply gets `Connection: close`, so a slot is held for one request, and the pool stays at 4 unless spike item 4 shows resets. It also noticed the existing 429 and 503 paths `continue` and let a limited stranger hold a slot for 30 s.
- Device name is the page's to send: caught (page builds `Safari on iPhone` from `navigator.userAgent` and sends `device_name`; host trims to 64).
- Found beyond the reference: the issue's "one-line excerpt" is UI-computed and not on the wire; the code in a query reaches whichever host answers the GET before `replaceState` runs, so the fragment form is safer; the pairing window opens with lan off and shows an unreachable address; `/pair` puts the token in a body a script can read; a 401 cannot distinguish revoked from rotated without comparing keys; nothing lets a device read its own scope; the frame drains only 8 remote requests a pass. All verified against the code.

## Decisions to overturn
- `SameSite=Lax` instead of `Strict`. The plan's reason is wrong: the first navigation from Messages fetches a static page that needs no cookie, and the page's own same-site `fetch` calls carry a `Strict` cookie. The reference's `Strict` is the CSRF defence; the Origin rule can stay as belt and braces.
- Socket first, poll only if the spike says so. The "With a cookie" flow opens `wss://` and waits for `hello` before the first snapshot. The reference makes polling the baseline and the socket the enhancement, because on Safari the socket never opens.
- Not reversed, though it departs from the reference: the fragment instead of the query. The plan's reasoning is sound and the reference's query form is the issue's text, not a code constraint. The new `GET /api/v1/device` endpoint is a small addition the page could live without (scope from the pair reply plus the 403 `scope` path), but it is cheap and I would let it stand.

## Over-specification
- The "Contract additions" block restates the decisions and the Actions section. The schema-version paragraph says at length that nothing is bumped. The exact UI copy strings, the `Max-Age` discussion, and several test descriptions that the existing suite would force (the unchanged 1395 and 1406 tests, `pair.json` not changing) could go. Roughly a fifth to a quarter of the plan.

## Hand-off
- Make polling the page's baseline and the socket an enhancement that never blocks the first snapshot, and use `SameSite=Strict`; Safari will not open `wss://` to a click-through certificate.

## Notes
- best: Every reference checks, including the ten-site literal count, and the fragment-not-query catch is a real improvement over the issue.
- worst: The page waits for `hello` before loading anything, so on Safari it shows nothing until a spike-gated fallback that the plan treats as the exception.