| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| N | 3 | 3 | 0 | 1 | no |

## Grounding
- `src/core/model.rs:12` (`SCHEMA_VERSION` is 16): found, exactly that line.
- `src/ports/remote.rs:17` (`IDENTITY_SCHEMA_VERSION` is 1): found.
- `src/ports/remote.rs:39-40` (`listen` added under `#[serde(default)]`): found, lines 39-40 are the attribute and the field.
- `src/core/remote.rs:64` (the window closes on the first pairing): found, the `Paired` arm with that comment sits at 65-69, one line off.
- `src/ui/cards.rs:558-574` (`agent_excerpt`): found, the function spans 557-574 with the 160-char cap.
- Later anchors spot-checked (`app.rs:3142-3148` 401, `remote.rs:835-848` pool move, `:573`, `:701-705`, `:785`, `app.rs:3650`, `core/remote.rs:166`, `ui/remote.rs:157-172` and `:218`, `app.rs:2920`/`2936`/`2951-2956`, `action.rs:1257`, `remote-client.md:306-317`, README 277/383/401/437, `FixturesTests.swift:157`, `time` 0.3.55 in the lock) all correct.

## Departures caught
- WebSocket may not open from Safari: caught. Marked Unresolved, page falls back to 5 s REST polling when the socket fails before `hello`, Spike 23 records the device result. Socket-first rather than the reference's poll-first, but the page works either way.
- "Certificate for download" is not an option with this certificate: caught with a wrong fix. The plan sees the SAN/EKU/validity problem but answers it by issuing a second, address-bound served leaf, and keeps download as a CA:TRUE fallback; the reference collapses to click-through with no reissue.
- Each page file is a connection: caught. Counts three files plus `/info` against the pool of 4, names the 30 s idle hold, moves cookie-bearing static loads to the paired pool, keeps 4→8 as fallback. Three files where the reference wants at most two.
- Device name is the page's to send: caught (`navigator.userAgent`, iPad heuristic, `device_name` in the body).
- Beyond the reference: the excerpt is not on the wire (shows `waiting_reason`/`card`/notes); scope after a reload needs a who-am-I read (`GET /api/v1/device`); the pair reply must omit `token` when a cookie is set; cookies ignore ports so another HTTPS service on the Mac's IP receives the cookie and SameSite alone is not the CSRF defence; `screen` is refused for a stopped session; revoked vs identity-changed is indistinguishable at the host's 401. All verified against the tree. Not caught: the invite carrying a LAN address while the host listens on loopback.

## Decisions to overturn
- The served leaf: a second certificate with the routed IP as SAN, serverAuth EKU, 800-day validity and random serial, kept in `identity.json`, reissued and rebound (dropping live connections) whenever the pairing window opens on a changed address, plus an `address_source` injection and a direct `time` dependency. Conflicts with reference decision 1 (click-through is the trust step, no reissue) and the spec's identity model that binds nothing to an address or a validity window.
- Every browser pairs `read` whatever the window says, owner raises by hand. Conflicts with the issue (scope chosen in the pairing window as today), reference decision 6 (scope from the pair reply, send box under `control`) and the acceptance path, which the plan rewrites to add a manual raise.
- Static files answered on the frame, with a new `page` rate window and cookie-authenticated `GET /app.css` moving its connection to the paired pool. The reference names a frame-side route for a constant file as the call it would reverse; serving constants on the connection thread removes the pool-move machinery too.
- Not reversed though it departs from the reference: the code in the URL fragment rather than the query (better: the code never reaches a request line), and the custom `X-Switchboard-Page` header on top of the Origin check (belt-and-braces, folded into over-specification).

## Over-specification
- The whole served-certificate thread (record, signals, actions, rebind, serial reasoning, core tests, the restart test) is work the implementer should not do.
- A fallback paragraph after nearly every decision ("if the owner says otherwise, build X"), many restating facts.md options.
- The Fetch-standard derivation of `Origin: null`, the rcgen serial-derivation footnote, the Commit section repeating the contract additions already stated in the body.
- Roughly a third to forty percent of the plan.

## Hand-off
- No. Two of the three reversals (the served leaf and the read-only cap) reshape the record, the adapter, the dialog text and the acceptance flow, so the plan needs a rewrite rather than a line of guidance.

## Notes
- best: Every later anchor is exact, and it found the holes the reference missed (excerpt off the wire, scope after reload, token in the pair body, cookie shared across ports) with concrete tests for each.
- worst: It invents an address-bound served certificate that is reissued and rebound from the pairing window, contrary to the spec's identity model and to the simplest reading of the trust decision.