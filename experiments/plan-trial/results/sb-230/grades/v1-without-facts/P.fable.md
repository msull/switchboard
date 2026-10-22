Checks done. All five first references are real; a few later citations (spike 23, a `facts.md`, HEAD 380e4d4) are not in this tree, since the base is 7fe8949 and `spikes/` ends at 22. Scoring now.

| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| P | 3 | 3 | 1 | 2 | note |

## Grounding
- `agent_excerpt`, `src/ui/cards.rs:558`: found (the fn at 557-558, computes the card's one-line excerpt UI-side)
- `SessionView`, `control/src/lib.rs:637`: found (struct at 637; `card`, `waiting`, `waiting_reason`, no excerpt field)
- `docs/remote-client.md:207`: found (the `session.last_response` bullet naming a `last_message` metadata field that the code lacks)
- `src/app.rs:3660`–`3665` (scope only in the `/pair` reply): found (the `json!({token, scope, server_id, device_id})` reply)
- `authenticate`, `src/app.rs:3127`–`3156`: found (reads `request.bearer` only; both refusals are 401 `unauthorized`)
- Later citations also check out (`DRAIN_PER_PASS` :580, limits :583-584, `limited` :631, pool accept :700-711, pool move :838-856, `local_address` :962-970, hover text `ui/remote.rs:113`, payload :155-172, `qr_texture` :280, spec :306, `tests/remote.rs:1422`, `tests/ui.rs:7474-7492`, Swift lines). Not in the tree: "spike 23 `README.md:46`", `facts/1/repo/facts.md`, HEAD 380e4d4. The plan was written against a worktree with extra files; the line numbers still match the base.

## Departures caught
- WebSocket will not open from Safari: caught, hedged as "Unresolved, WebKit 41419 suggests it may not", with a polling fallback built in regardless. Caught.
- Certificate download is not an option with this certificate: caught (no SAN, Apple refuses it even as a trusted anchor; reissue binds to an address). Click-through chosen; SAN reissue kept only as a fallback.
- Each page file is a connection: caught, with a different fix from the reference. Three files instead of two, but `Connection: close` on each so the slot frees at once, a two-deep request queue, socket opened only after the first authenticated reply, and a stated fallback of raising the pool to 6.
- Device name is the page's to send: caught (page fills `device_name` from `navigator.userAgent`, editable; host parses nothing).
- Beyond the reference: the excerpt is not on the wire (reads `last-response` for waiting sessions only); no endpoint tells a device its scope after reload (adds `GET /api/v1/me`); the pair reply would still carry the token in its body (omitted when `credential: "cookie"`); the invite carries a LAN address while the host listens on loopback (listed as a known gap); "revoked" and "never paired" are the same 401 (a non-secret marker tells them apart); the key comparison is against the host's own claim and does not stop a relay; a `__Host-` cookie still reaches every HTTPS server on the IP whatever the port.

## Decisions to overturn
- Live updates: the plan opens the socket first, waits for `hello`, and only then fetches `projects`/`waiting`/`sessions`; polling starts after a 5 s timeout or two pre-`hello` closes. On the Safari devices the page is for, every load is blank for 5 s. The reference says poll first, socket second. Reverse: fetch immediately, treat the socket as an upgrade.
- One QR, a URL: the plan keeps "Copy invite" as JSON, adds a "Copy link" button, and leaves `PairingInvite.parse` JSON-only as the "Keep JSON branch". The issue states the native text path takes the URL too, and the reference has `parse` accept the URL and the JSON with the UI test changed. Reverse.
- Not reversed but noted: three static files where the reference wants at most two. With `Connection: close` the pool argument holds, so I let it stand. The `/me` route, the Origin rule, and `PAGE_PER_MINUTE` are additions the reference tolerates.

## Over-specification
- The full bearer-plus-`localStorage` branch "written out in full so the choice can go either way", the host-side user-agent table branch, the read-only-scope-for-cookies branch, the excerpt-on-`SessionView` branch with fixture rewrite, and the SAN-reissue-plus-`/switchboard.cer` fallback are each a second plan an implementer would not build. The schema-version inventory for a change that bumps nothing, and the repeated "Facts, Settled / Unresolved" citations to a file not in the tree, add noise. Roughly a quarter to a third of the text.

## Hand-off
- Fetch the collections before the socket opens and teach `PairingInvite.parse` the URL form; otherwise build the cookie default as written and ignore the alternative branches.

## Notes
- best: catches every departure in the reference plus all four errata ones, and the cookie-ignores-port and Origin-under-no-referrer subtleties are correct and would otherwise bite in production.
- worst: §4.4 followed so literally that the page is blank for five seconds on the device it is for, and the issue's explicit "the native text path takes the URL too" is declined as an owner branch.