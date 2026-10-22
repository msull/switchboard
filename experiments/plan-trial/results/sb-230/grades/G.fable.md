| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| G | 3 | 3 | 1 | 2 | note |

## Grounding
- `control/src/lib.rs:637` (`SessionView` has `card`, `waiting`, `waiting_reason`, no excerpt): found (struct starts at 637; fields as described, `trust_question` at 662 also as the plan says)
- `src/ui/cards.rs:176-185` (excerpt computed in the UI from transcript or caption poll): found (lines 177-185 do exactly that)
- `src/ports/events.rs:24` (`last_message` lives only on the Stop hook event): found (`Stopped { last_message, .. }` at 23-24; `model.rs` has no `last_message`, so the record claim holds)
- `docs/remote-client.md:207` (says the excerpt is `last_message` on the record, which is wrong): found (line 207 says exactly that; the code disagrees)
- `src/adapters/remote.rs:95-127` (`make_identity`: no SAN, no EKU, far expiry): found (empty SAN list, CN only, `self_signed`)
- Later refs spot-checked and also correct: `Info.swift:113-116`, `main.swift:111`, `BrowserView.swift:106` and `883`, `ui/remote.rs:160-169`, `remote.rs:304-312`, `remote.rs:564-573`, `631` (`Limits::limited`), `701-707` (accept drop), `app.rs:3023`, `3636-3644`.

## Departures caught
- Safari refuses `wss://` under a click-through: caught, but hedged. The plan does not state the WebKit limitation as a fact; it files WSS-under-exception as a spike question for the owner. It does make the page work without the socket (reconnect with backoff, poll `/waiting` and the open session every 10 s while disconnected), which is the part that matters. Polling covers only `/waiting` and the open session, not the project and session collections, so a new session would not appear on Safari until a reload. Socket-first rather than poll-first.
- Certificate download is not an option: caught, with the right reasons (no SAN, no `serverAuth` EKU, expiry 4096, DHCP-moved address).
- Each file is a connection: caught, with a different fix from the reference. The plan keeps three files plus a 204 favicon rather than two files, and instead adds `Connection: close` on unauthenticated static replies so a slot is held for one request rather than 30 s, moves cookie-bearing static loads to the paired pool, and states in §4.1 what the pool leaves for a second device. Sound, though more mechanism than the reference's "two files".
- Device name is the page's to send: caught (page fills `device_name` from `navigator.userAgent`, host parses nothing, cap at `core/remote.rs:166`).
- Found beyond the reference: the code in the URL's **query** reaches whoever answers at the address on the first GET, before any key check; the plan uses the **fragment**. Verified reasoning, and I agree with the plan over the reference here.
- Found beyond the reference: the card excerpt is **not on the wire** and `remote-client.md:207` is wrong about it. Verified; the plan substitutes `waiting_reason` or the `card` word and corrects the line.
- Found beyond the reference: `trust_question` on `SessionView` means a send to a pane at the trust prompt goes nowhere; the page hides the box while it is true. Verified at `lib.rs:662`.

## Decisions to overturn
- **Static files routed through `serve_remote` on the frame thread** (decision 4, "The adapter and the app"). The reference reverses a frame-side route for a constant file, and I agree: a constant reply should be written on the connection thread, and the plan's only reason for the frame hop is the cookie-to-paired-pool move, which can wait until the page's first API call.
- **A new `GET /api/v1/device` read plus a `device` capability and fixture.** The reference keeps scope from the pair reply with no new read model. The plan's reason (scope is lost on reload) is real, but the page already keeps `{server_id, spki_sha256, server_name}` in `localStorage` and can keep scope beside them; any existing read serves as the revoked probe. The issue says no new read unless the page cannot do without one, and it can.
- Not reversed, noting disagreement with the reference: the fragment URL instead of the query. The plan is right that the query is sent before the key can be compared.

## Over-specification
- "The record" section (schema versions that do not change) and "The signals" section (restates existing events) say nothing an implementer needs.
- The closing "Contract additions" list repeats the body almost item for item.
- The certificate SHA-256 fingerprint line in the dialog, the `.local` hostname digression and its spike row, `client/README.md`, and the Origin check for `/pair {cookie:true}` are extras beyond the issue.
- The pool tests that hold three TLS connections open inside the 5 s handshake deadline are timing-dependent and more than the suite needs; the `Connection: close` and cookie-pool tests could be one test each.
- The spike README with an empty results table for the owner is a process artefact rather than plan content.
- Roughly a quarter to a third of the plan.

## Hand-off
- Answer `/`, `/app.js` and `/app.css` on the connection thread instead of in `serve_remote`, and drop `/api/v1/device` in favour of storing scope beside `server_id` in `localStorage`.

## Notes
- best: every reference checks out and it found two real departures the reference missed, the query-vs-fragment leak and the excerpt not being on the wire.
- worst: it treats Safari's `wss://` refusal as an open spike question rather than a known fact, so the page is socket-first and its fallback polls only `/waiting` and the open session.