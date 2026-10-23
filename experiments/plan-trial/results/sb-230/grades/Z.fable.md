| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| Z | 3 | 2 | 1 | 2 | note |

## Grounding
- `control/src/lib.rs:637` (`SessionView`): found (struct at 637 with `card`, `waiting`, `waiting_reason`, `liveness`, `quiet_secs`, `last_activity_ms`; no excerpt field, as the plan says)
- `src/ui/cards.rs:558` (`agent_excerpt`): found (fn at 558, computes the excerpt from a `Conversation` UI-side)
- `src/adapters/remote.rs:95-100` (`CertificateParams::new(vec![])`): found (`make_identity` starts at 95; the empty-SAN `CertificateParams::new(Vec::<String>::new())` is at 102, within the cited function)
- `src/app.rs:3730` (`envelope`): found (`envelope` at 3730 emits `epoch`, `revision`, `body` only; scope is not on the wire, as claimed)
- `src/adapters/remote.rs:962` (`local_address`): found (connected IPv4 UDP socket, unbracketed `IpAddr`)
- Spot-checked beyond five: `IDENTITY_SCHEMA_VERSION` at ports/remote.rs:17, `Pin` verifier 183-200, `new_device` trim at core/remote.rs:169, `wrong_key` at app.rs:3641, `pairing_open` 3596, `capabilities` 3580, `write_fixtures` 1587, `bind_remote` 2936, `connection` 770, `MAX_UNAUTHENTICATED` 567, `IDLE_TIMEOUT` 573, `limited` 631, hover text ui/remote.rs:113, Copy invite ~205, BrowserView.swift:849, main.swift:107, design.md remote section ending before 3585, README 276, CHANGELOG line 9. All exact.

## Departures caught
- Safari refuses `wss://` to a clicked-through certificate: caught. The plan promises polling as the only iOS path, a 4 s socket timeout to fallback, `watch`/`unwatch` over the socket when it opens.
- Certificate download is not an option with this certificate: caught with a wrong fix. The plan sees the no-SAN, no-EKU, year-4096 certificate but answers by reissuing it (SAN, `serverAuth`, 800 days, `refresh_certificate`, `identity.json` fields and a schema bump, rebind on Tick) and serving `/api/v1/certificate`. The reference rejects exactly that: it ties the identity to an address and an expiry §5/§9 avoid, and Safari still refuses `wss://` after click-through. The plan itself admits the download is "untested on iOS" and not the path it promises.
- Each page file is a connection: caught. `Connection: close` on static replies returns the slot at once; pool stays at 4 with a new `assets` rate window. Three files rather than the reference's two, but the close makes that tolerable.
- Device name is the page's to send: caught. `navigator.userAgent` into `device_name`, host retains no header.
- Found that the reference lacks (all in the errata): the excerpt is not on the wire (deferred, shows `card` and `waiting_reason`); nothing tells a cookie device its scope after reload (`GET /api/v1/device`); the invite carries a LAN address while bound to loopback (QR hidden, line in the dialog); the pair reply would still carry the token in its body (omitted with `session: "cookie"`). Also that no `Origin` or `Content-Type` check exists for a cookie-authenticated `send` (`403 origin`), and `Referrer-Policy: no-referrer` for the `?code=` URL.

## Decisions to overturn
- The certificate reissue subsystem and `/api/v1/certificate` download (SAN entries, EKU, 800-day validity, `Identity.addresses`/`not_after`, `IDENTITY_SCHEMA_VERSION` bump, `CertificateReissued` action, rebind on Tick dropping live sockets). The reference's decision 1: click-through is the trust step, no download, no reissue; the spec's identity model binds nothing to an address or validity window. The plan concedes the download is untested on iOS and that polling is what it promises, so the machinery buys nothing the plan relies on.
- Cookie named `sb_session` for every instance. The reference names the cookie with the server id because cookies ignore ports, so two Switchboard instances on one Mac would overwrite each other's token. The plan notes port-blindness toward other services but misses this case.
- Not reversed: cookie transport with `HttpOnly; Secure; SameSite=Strict; Path=/` plus the `Origin` check, bearer kept for native clients, `read_request` filling the session from the cookie, connection-thread static paths, URL invite with `parse` accepting both forms, `GET /device`, deferred excerpt. All match the reference or its errata.

## Over-specification
- The certificate section and its tests (DER byte checks for SAN and OID, `reissue_certificate`, `refresh_certificate` at 90 days, old-file reissue) is the largest cuttable block, roughly 15% of the plan.
- "Contract additions" restates the body; "For the owner" lists six items, three of which repeat earlier paragraphs; the cookie port-blindness cost is stated three times (credentials, dialog, docs). Together roughly 10%.
- The test list and build order are long but an implementer uses them. About a quarter of the plan is cuttable overall.

## Hand-off
- Drop the certificate reissue, SAN/EKU/validity work and the download endpoint, name the cookie by server id, and keep everything else as written: click-through is the trust step and the polling path already covers Safari.

## Notes
- best: every line cited is exact, and it found all four errata departures plus the missing CSRF defence, with clean fixes for each.
- worst: builds a certificate reissue and download subsystem the spec's identity model forbids and that, by the plan's own admission, Safari cannot use for `wss://`.