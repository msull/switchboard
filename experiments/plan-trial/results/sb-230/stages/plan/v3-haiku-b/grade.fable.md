| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| v3-haiku-b | 3 | 3 | 1 | 2 | note |

## Grounding
- `docs/remote-client.md` §4.3 (line 207): found (line 207 says the card excerpt is `last_message` from the Stop hook on the record, moving with `changed {session}`; the plan's claim that it is not on the wire matches `SessionView`)
- `src/ports/events.rs:24`: found (`EventKind::Stopped { last_message: Option<String>, pending: Option<StopPending> }` at lines 23-29)
- `src/core/events.rs:132`: found (`EventKind::Stopped { pending, .. }` at 132, sets `last_stop_at` and `pending_at_stop` only; `last_message` is discarded as the plan says)
- `control/src/lib.rs` `SessionView`: found (line 637; `card`, `waiting`, `waiting_reason`, `last_stop_at_ms`, no excerpt field)
- `client/Sources/SwitchboardUI/BrowserView.swift:149-163`: found (lines 149-153 fetch `lastResponse(for:)` per conversation holder; the range is right for the card-line claim)
- Spot checks beyond five also hold: `app.rs:3017/3127/3606/2936/2957/3754`, `remote.rs:95/139/146/292/360/389/444/519/530/567/584/962`, `tests/remote.rs:245/406/658/682/1587/1829/1960`, `Cargo.toml:62`, `model.rs:12/612`, `ports/remote.rs:17/26-42`, `ui/remote.rs:160/201`. Only `:247` is loose (it is `spki_sha256_of_cert`, not the key hash itself).

## Departures caught
- WebSocket will not open from Safari: caught. Polling is the default, the socket an upgrade marked "live", and the plan notes content `changed` needs `watch` frames. It frames it as unresolved rather than known, but the fix is the reference's.
- "Offer the certificate for download" not an option: caught, click-through chosen and no profile. But the plan then reissues the certificate anyway (see decisions).
- Each file is a connection: caught. Two files, `Connection: close` so the slot frees at once, CSS inline, data: favicon, pool left at 4 with the arithmetic stated.
- Device name is the page's to send: caught (D6, `navigator.userAgent` into `device_name`, host cuts to 64).
- Beyond the reference: the excerpt is not on the wire (verified, with the right fix of reading `last-response` keyed on `last_stop_at_ms`, and a spec correction); the device's scope is on no endpoint after pairing (adds `/device`); revoked and never-issued are the same `unauthorized` on the wire; `replaceState` cannot scrub the entry the camera created; the key compare proves a claim only. All verified. Not found: the invite carries a LAN address while the host may listen on loopback.

## Decisions to overturn
- Token in `localStorage` with a bearer header and a `Sec-WebSocket-Protocol` carrier, no cookie. The reference's call is an `HttpOnly; Secure; SameSite=Strict` cookie named by server id, filled into `bearer` in `read_request`, and names `localStorage` as the one call it would reverse. The plan's reasons (cookie storage on an IP host unresolved) are honest but push the token into script reach for the sake of an unverified risk.
- Certificate reissue over the same key with an IP SAN, `serverAuth` EKU, 800-day validity, renewal at bind time, two new `Identity` fields and an `IdentityRenewed` action. The reference says no reissue: it ties the identity to an address and an expiry the spec deliberately avoids, and Safari still refuses `wss://` by Cockpit's report. The plan itself admits the IP SAN is unresolved and the renewal buys nothing the click-through does not.
- Not reversed but noted: CSP `connect-src 'self' wss:` admits any `wss:` host; `'self'` alone is tighter.

## Over-specification
- The whole certificate track: D1 contents, the `identity.json` fields, `IdentityRenewed` and its core tests, `certificate_wants_renewal`, `reissue_certificate`, the DER tag check, the README identity description, build step 1. Roughly a fifth of the plan and all of it would be dropped.
- "Left to the owner" and "Contract additions" largely restate earlier sections; the error-string mapping and polling intervals are more than an implementer needs. Together about a quarter to cut.

## Hand-off
- note: drop the certificate reissue track entirely (keep click-through only, no identity change) and move the token to an `HttpOnly; Secure; SameSite=Strict` cookie named by server id; everything else stands.

## Notes
- best: every citation checks out, and it found and correctly fixed departures the reference missed (excerpt not on the wire, scope after reload, revoked indistinguishable, `watch` needed for content events).
- worst: a large, self-contained certificate reissue and renewal mechanism that binds the identity to an address and expiry the spec avoids, justified by questions the plan itself calls unresolved.