| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| I | 3 | 2 | 1 | 1 | note |

## Grounding
- `remote_info`, `src/app.rs:3568`: found (the fn is at 3568, returns `spki_sha256` in the info body as the plan says)
- `agent_excerpt`, `src/ui/cards.rs:558`, called at `:178`: found (fn at 558, call at 178; the excerpt is built UI-side from the parsed conversation, exactly the plan's point)
- `SessionView`, `control/src/lib.rs:646`: found (the `card: String` field with the "waiting on you" doc comment is at 646)
- `client/Fixtures/session.json`: found (carries `card`, `liveness`, `notes`, no excerpt field)
- `strip_prefix("/api/v1/")`, `src/app.rs:3023`: found (the 404 `not_found` with `authenticated: false` is the next line)
- Spot-checked beyond five: `RemoteRequest :360`, `RemoteReply :389`, `read_request :444`, `bearer :487`, `respond :530`, pools `:567`, `limited :631`, `connection :770`, pool move `:835`, `local_address :962`, `authenticate :3127`, `remote_pair :3606`, device name default `:3650`, `bind_remote :2924`, 28 `RemoteReply {` literals in app.rs, `tests/remote.rs` 245/406/658/682/1406/1494/1587, `tests/ui.rs:7424` and `:7484-7492`, `src/ui/remote.rs` 110-117/141/160-166/199-203/230, design.md 3107/3573/3585, README 275-277/383/423/437, `main.swift:10-13`. All exact. The best-grounded plan I expect to see.

## Departures caught
- WebKit refuses `wss://` to a click-through certificate: **missed**. The page is designed socket-first with no polling path ("The signals: none new... it opens `wss://`"). The spike would measure the failure, but the plan's answer to a failed row is "the work stops and goes back to the owner", not a page that polls. On the device the issue is for, this plan ships nothing.
- Certificate download is not an option: **caught** by convergence. Picks click-through for different reasons (install burden, device-wide trust) but lands on the reference's answer. Its fallback of adding an IP SAN at serve time if "proceed" is not offered is the address-binding the spec avoids, but it is conditional.
- Each file is a connection: **caught with a weak fix**. Notes the six-connection browser and the pool of 4, but keeps three files, leaves the pool untouched, relies on "the browser retries", and never says what a second device has left while keep-alive connections that only fetched static files sit in the unauthenticated pool for 30 s.
- Device name is the page's to send: **caught** (`device_name` from `navigator.userAgent`, `remote_pair` already takes it).
- Found, not in the reference: the card excerpt is not on the wire (`agent_excerpt` is UI-side; list shows `card` and `waiting_reason`). Real and correct.
- Found, not in the reference: the `/pair` reply must omit `token` for the cookie form, and the page cannot know its scope after a reload.
- Found, not in the reference: `Referrer-Policy: no-referrer` because the page URL carries the code and `/app.js` would otherwise be requested with it as `Referer`. Sharp.

## Decisions to overturn
- **Socket-only live updates.** Reference: poll first, socket when it opens. The plan's page refreshes only on `changed` over the WebSocket and the plan stops the work if the spike's `wss://` row fails, which it will on an iPhone.
- **Step 0: owner sign-off before anything begins.** The issue says "pick one, say why"; the reference decides click-through is the trust step and the dialog says so. The plan makes the same call and then refuses to act on it, so an implementer blocks on day one.
- Borderline, counted under over-specification rather than here: `GET /api/v1/me` with a fixture and a Swift `DeviceMe` nobody native calls. A 401 on any read is the "revoked" probe and scope could ride in the `localStorage` entry the plan already writes. Also "Copy invite" kept as JSON beside a new "Copy link" where the reference makes one button copy the URL, which `parse` accepts anyway.

## Over-specification
- The schema-version paragraph for a change that touches no record ("should an implementer find a device kind necessary after all, the bump is..."); the `grep -o` count of `RemoteReply` literals the compiler finds anyway; the cookie name's 42-character form restated four times and tested three ways; the sign-off and spike gate restated in four sections; the three-part Origin/`Sec-Fetch-Site`/IP-literal-`Host` rule where the reference says `SameSite=Strict` plus an `Origin` check is enough (the IP-literal `Host` rule is redundant with the cookie's own origin binding); the optional desktop certificate-fingerprint line; the `me.json` fixture and Swift type; the bundle.sh/`Cargo.toml` `include` aside; the "nothing names a client" boilerplate; the "Contract additions" commit body that repeats the plan. Roughly a third of the text.

## Hand-off
- Drop step 0 and the stop-on-spike; have the page poll the collections and the open session's content on a short interval and use the socket only when it opens, then build the rest as written.

## Notes
- best: Line-exact grounding throughout, and the cookie design (`__Host-` prefix, server-id name because cookies ignore ports, `HttpOnly; Secure; SameSite=Strict`, `Referrer-Policy` to keep the code out of `Referer`) matches or improves on the reference.
- worst: It plans the WebSocket as the only live path and, when its own spike would reveal Safari refuses it, halts the work rather than polling, so the page it describes would never ship on a phone.