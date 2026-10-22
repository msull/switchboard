| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| T | 3 | 2 | 0 | 2 | note |

## Grounding
- `client/Sources/SwitchboardProtocol/Info.swift:114-116`: found (`PairingInvite.parse` decoding JSON via `Wire.decode`, exactly those lines)
- `client/Sources/switchboard-remote/main.swift:107-111`: found (`case "pair"`, the one-argument `PairingInvite.parse(rest[0])` path at 110-111; the three-argument form follows at 112-124 as the plan says)
- `client/Sources/SwitchboardUI/BrowserView.swift` (the headset's paste): found (`PairingInvite.parse` at lines 106 and 883)
- `src/ui/remote.rs:160-166`: found (the `serde_json::json!` builder of `{server_id, spki_sha256, code, addresses}`, with `local_address()` just above at 157)
- `control/src/lib.rs:645-646`: found (doc comment "The card's word: `CARD_WORKING`, `CARD_IDLE`, 'waiting on you'" and `pub card: String`)
- Spot-checked beyond five: `app.rs:3023-3029` strip and pre-auth arms, `:3661` `"token": token`, `:486-494` socket close on revoke, `:3150` `DeviceSeen`, `:3361` `ensure_editor`, `:3754` `refuse` logs path only, `remote.rs:567/631-640/704/962`, `ports/remote.rs:95-107`, `core/remote.rs:86-92/145/169`, `tests/remote.rs:657/674/676/2892/3226`, `tests/ui.rs:7487-7500`, `remote-client.md:194/306-312`, `design.md:125-128/3107-3112`. All correct, several to the exact line.

## Departures caught
- WebSocket will not open from Safari: caught with a wrong fix. The plan does not know the WebKit limitation; it gates on a spike (step 0) and names "poll the session view" only as a fallback. The page as designed is socket-first, which the reference says yields a static page on the device it is for. The spike gate means it would be found before code is written, so partial credit.
- Certificate download is not an option: caught. Click-through chosen, no download; the plan notes the SAN-less, year-4096 certificate and that a re-issue must reuse the Keychain key or every pin breaks.
- Each file is a connection: caught. CSS inline, favicon as a data URI, counts `/`, `/app.js`, `/info` as three, measures in the spike, fallback pool of 6.
- Device name is the page's to send: caught (`navigator.userAgent` in the page's script, host unchanged).
- Errata items: excerpt not on the wire (caught: `card` word plus `waiting_reason`); no scope after reload behind a cookie (caught: `GET /api/v1/device`); invite carries a LAN address while the host listens on loopback (caught: a warning when `listen.lan` is off); pair reply carries the token in its body (caught: `carrier: "cookie"` strips it).
- Found by the plan, absent from the reference: the `/info` key comparison is an address check, not server authentication, since a browser has no pin; `?code=` reaches the host's request line and subresource `Referer` unless `Referrer-Policy: no-referrer`; `RevokeDevice` deletes the row so "revoked" and "never paired" are indistinguishable today; `Limits::limited` has no window for static paths. All verified in the tree.

## Decisions to overturn
- Static files answered on the frame (Decision 4). The reference reverses a frame-side route for a constant file; the connection thread should serve them.
- `SameSite=Lax` (Decision 2). The reference calls `Strict` the CSRF defence. The plan's reason for `Lax`, that a top-level navigation from another app would arrive cookieless, costs nothing here: the HTML is static and the page's own same-site fetches carry the cookie either way.
- Cookie named `switchboard_token` with no server id. Cookies ignore the port, so two instances on one Mac overwrite each other's token; the reference names the cookie by server id.
- Socket-first with polling only as a spike fallback (Decision 3 in the reference: poll first, socket second). Overlaps with departure 1 but is also the page's design.
- My own, not in the reference: the `revoked` tombstone changes `RevokeDevice`'s semantics, keeps revoked token hashes forever, adds a downgrade hazard and a versioned-wrapper fallback, and reaches into the dialog and two existing tests. A plain 401 "not paired or revoked" meets the acceptance. The plan does flag this as an owner call.

## Over-specification
- The Contract additions section restates The actions and effects almost line for line; the schema-version inventory appears twice; the commit-message section adds nothing an implementer needs. Seven owner calls each with a fallback design (versioned `devices.json` wrapper, `#` fragment variant with Swift parser changes, re-issue renewal design) are planning for branches the spike should prune. The user-agent detection order and the `?code=` logging analysis could be one sentence each. Roughly a fifth to a quarter of the plan.

## Hand-off
- Poll first and treat the socket as a bonus, serve the two static files on the connection thread, use `SameSite=Strict` with the cookie named by server id, and drop the tombstone for a plain 401.

## Notes
- best: exact, verified line references throughout, and it found every errata departure plus the `/info` address-check and `Referer` leaks on its own
- worst: ships the page socket-first on the one browser where the socket will not open, with polling buried as a spike fallback