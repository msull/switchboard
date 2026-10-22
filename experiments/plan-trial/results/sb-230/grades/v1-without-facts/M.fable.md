| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| M | 3 | 2 | 1 | 2 | note |

## Grounding
- `src/app.rs:3023-3025`: found (`serve_remote` strips `/api/v1/` and refuses any other path with `404 not_found`, on the frame, exactly as the plan says)
- `src/ui/cards.rs:176-184,558-574`: found (the caption comes from `agent_excerpt` over the conversation cache for agents, or the pane caption otherwise; `agent_excerpt` cuts at 160 chars; nothing on the wire)
- `src/app.rs:3568-3600`: found (`remote_info`, unauthenticated envelope with `server_id`, `spki_sha256`, capabilities, limits; no device scope)
- `src/app.rs:3127-3153`: found (`authenticate` has two refusals, both 401 `unauthorized`, so revoked and never-paired are indistinguishable as the plan claims)
- `src/core/remote.rs:86-92`: found (`RevokeDevice` retains every device but the id and saves; the hash is gone)
- Secondary checks also held: `RemoteRequest::path` is documented as "without its query" at 362-363, `respond_upgrade` at 519-527 echoes no subprotocol, `connection` at 770 with the rate check at 788 and `upgrade_key` clone at 797, the `json!` payload at `ui/remote.rs:160-166`, `Info.swift:114-116` decodes JSON only, `remote-client.md:254` describes the watch set and 306-318 the §5 lines named.

## Departures caught
- Safari refuses `wss://` on a click-through certificate: caught, but hedged as "Unresolved" rather than known. The plan ships a poll fallback, so the page works, but only after two failed handshakes each followed by a `GET device` round trip on the very device the page is for.
- Certificate download is not an option with this certificate: missed as a departure. The plan picks click-through and leaves `identity.json` untouched, but never says why download collapses (no SAN, no EKU, 4096 expiry, and Safari still refuses the socket).
- Each page file is a connection: caught with a different fix. Three files rather than two, mitigated by `Connection: close` on every static answer so the slot frees on write; acknowledges iOS connection count as unknown and names the raise to 8 as the fallback.
- Device name is the page's to send: caught (`navigator.userAgent`, with the iPad-as-Mac case handled by `maxTouchPoints`).
- Excerpt not on the wire (reference errata): caught, with a `last-response` fetch for the first 8 waiting Claude rows and the watch-set accounting to justify it.
- Scope after reload (errata): caught, adds `GET /api/v1/device`.
- Invite address versus loopback listen (errata): partly caught; falls back to `127.0.0.1` when there is no LAN address, which is honest but does not address a LAN address on a loopback-only listen.
- Beyond the reference: the frame-side 404 costs an inbox slot, so non-API paths are answered on the connection thread; revoked is not distinguishable from never-paired, so the page infers it from a stored token plus unchanged key plus 401; the URL key check only proves what `/info` claims; `replaceState` is tidying and the single-use code is the real guarantee. All verified against the code.

## Decisions to overturn
- Token in `localStorage` with `Sec-WebSocket-Protocol` transport. The reference calls `localStorage` the one reversal; an `HttpOnly; Secure; SameSite=Strict` cookie named by server id, with `read_request` filling `bearer` from it, leaves the page's script with nothing readable and needs no new subprotocol parsing or 101 echo.
- Socket first, poll only after two failures. The reference says poll first and use the socket when it opens. On the target device every load pays two handshake failures and two extra requests before anything refreshes; polling from first paint is simpler and makes the fallback the common path it will be on Safari.
- Not counted: three files instead of two (mitigated by `Connection: close`), the new `device` endpoint (the errata credits it), the 8-row excerpt fetch (errata credits reading `last-response`, though I would cap it less elaborately).

## Over-specification
- The WHATWG, CSP Level 3 and WebKit bug citations with URLs, revision numbers and dates; the repeated "This rests on the For the owner line …" and "Settled/Unresolved" tags pointing at a facts file that is not in the plan; the watch-set arithmetic for the excerpts; the sentence-by-sentence list of §5 lines to reword; the `tchar` and `WindowLocalStorage` IDL asides. Roughly a quarter to a third of the plan.

## Hand-off
- Store the token in an `HttpOnly; Secure; SameSite=Strict` cookie that `read_request` maps to `bearer`, and poll from first paint while the socket is attempted.

## Notes
- best: Serving the page and every non-API path on the connection thread with `Connection: close`, and the honest reading of what the URL key check does and does not prove.
- worst: Choosing `localStorage` after naming its cost, and treating the Safari socket refusal as an open question that the owner's phone will answer at acceptance.