| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| P | 3 | 3 | 1 | 2 | note |

## Grounding
- `agent_excerpt`, `src/ui/cards.rs:558`: found (the fn starts at 558, computes the card's first-line excerpt from the last turn)
- `SessionView` (`control/src/lib.rs:637`): found (struct at 637; has `card`, `waiting`, `waiting_reason`, no excerpt field)
- `docs/remote-client.md:207`: found (the `last_response` bullet, ending with the `last_message` sentence describing a field the code lacks)
- `/pair` reply carries `scope`, `src/app.rs:3660`–`3665`: found (the `json!({token, scope, server_id, device_id})` reply is exactly there)
- `authenticate`, `src/app.rs:3127`–`3156`: found (reads only `request.bearer`; both refusals are 401 `unauthorized`, so revoked and never-paired are indistinguishable as the plan says)
- Spot-checked beyond five: `RemoteRequest` :360, `RemoteReply` :389, `respond` :530, the pool check :702, `qr_texture` :280, the payload :156–172, `tests/ui.rs:7474–7492`, `PairedDevice` :96, schema constants, `local_address` :962, hover text :113, spec :306, `refuse` :3754. All exact.

## Departures caught
- WebSocket will not open from Safari: caught (framed as Unresolved via WebKit 41419, but the page is built to work without it: five-second hello timeout, then polling). Fix is acceptable; see decisions for the ordering.
- Certificate download is not an option: caught (Apple refuses the CN-only cert; a SAN reissue binds to an address; click-through chosen). It keeps the SAN reissue as a conditional fallback rather than ruling it out.
- Each file is a connection: caught (`Connection: close` on the static files, a two-deep request queue, socket opened only after the first authenticated reply, fallback raise to 6). Chooses three files rather than two, mitigated by the close.
- Device name is the page's to send: caught (page fills `device_name` from `navigator.userAgent`, editable; no host-side parsing).
- Errata items: excerpt not on the wire caught (reads `last-response` for the first ten waiting sessions); scope after reload caught (`GET /api/v1/me`); loopback address in the invite caught (listed as known gap); token in the pair body with a cookie caught (omitted when `credential: "cookie"`).
- Beyond the reference: revoked vs never-paired are the same 401 and the page needs a non-secret marker to tell them apart; the URL key check is against `/info`'s own claim, not the peer certificate; `Referrer-Policy: no-referrer` would null the `Origin` on a non-cors POST, hence the "never set `mode`" rule and its test. All verified correct against the Fetch spec's "append a request Origin header" steps.

## Decisions to overturn
- "Copy invite" stays JSON and `PairingInvite.parse` is not taught the URL; the issue explicitly asks that the native text path take the URL too, and the reference makes `parse` accept both and the UI test change with it. Deferring this to an owner branch drops a stated requirement without a code reason.
- Socket first, poll on a five-second hello timeout, with the snapshot fetched only after `hello` (§4.4 ordering). On Safari, the device the page is for, every load shows an empty list for five seconds before the fallback kicks in. The reference polls first and uses the socket when it opens; I agree with the reference.
- Not reversed: three static files instead of two (the `Connection: close` makes it defensible), the SAN-reissue fallback (conditional, not taken), `/me`, `PAGE_PER_MINUTE`, the Origin rule.

## Over-specification
- Fully written alternate branches (the bearer/subprotocol branch, the SAN-reissue fallback, the read-only-browser branch, the UA-table branch, the URL-parser branch), the repeated "Rests on Unresolved line" hedges, the schema-version paragraph confirming no bump, the Contract additions section restating the body, and the commit-message section. Roughly a third of the plan.

## Hand-off
- Teach `PairingInvite.parse` the URL and make "Copy invite" copy it, and fetch the first snapshot immediately rather than after `hello`, so Safari is not blank for five seconds.

## Notes
- best: Every line reference is exact, and the Origin/`Referrer-Policy` interaction is reasoned correctly from the Fetch spec and guarded by a test.
- worst: Hedging in full-text fallback branches and deferring the native URL parser the issue explicitly asked for.