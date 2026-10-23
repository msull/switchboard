| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| AA | 3 | 3 | 1 | 2 | note |

## Grounding
- `src/adapters/remote.rs:101-107` (certificate has no SAN, CN only): found. `CertificateParams::new(Vec::<String>::new())` plus the `Switchboard <id>` common name at 102-107.
- `src/adapters/remote.rs:113` (SPKI hash from the key alone): found. Line 112 computes `sha256_hex(&key.subject_public_key_info())`, 113 is the debug assert against the cert. Off by one, still the right spot.
- `src/app.rs:3662` (scope only in the `/pair` reply): found. The reply JSON at 3660-3665 carries `token`, `scope`, `server_id`, `device_id`; no other endpoint returns scope.
- `docs/remote-client.md:169` (no all-sessions listing, `waiting` is its own collection): found. Row 169 is `GET /api/v1/waiting`; sessions are per project at 167.
- `src/adapters/remote.rs:958-970` (`local_address()` is `None` with no network): found. Function at 962-970, doc at 957-960.
- Spot-checked further: `app.rs:3017` (`serve_remote` prefix strip and 404), `3127` (`authenticate` reads only `bearer`), `452` (`dispatch`), `486` (revoke drops subscriber queues), `1869-1886` (`no_conversation` arms), `remote-client.md:312`, `ui/mod.rs:401,420`, `remote.rs:583-590` and `700-705`, `tests/remote.rs:245` and `682`, `script.rs:729-737`, `README.md:277`, `Info.swift:94-117`, `FixturesTests.swift:157-165`, `ui/remote.rs:155-166`, spike numbering (22 is the last). All as the plan says.

## Departures caught
- WebSocket will not open from Safari: caught, with a weaker fix. The plan polls `/waiting` on a 3 s interval and keys refetches on the envelope revision, which makes the acceptance step hold. But it removes the socket from the commit entirely, where the reference keeps it as the upgrade that fails quietly on WebKit and works on Chrome and Firefox.
- Certificate download is not an option: caught. The plan picks the click-through and declines the download. It then adds an in-memory IP-SAN reissue anyway, scored below as a decision.
- Each page file is a connection: caught. Page files answered on the connection thread, outside the frame, in a pool of their own (6) with a 503 when full, with the arithmetic stated (total 22). Three files rather than two, which the separate pool absorbs.
- Device name is the page's to send: caught. The script fills `device_name` from `navigator.userAgent` with the `maxTouchPoints` heuristic for iPad; no host-side parsing.
- Beyond the reference: the excerpt is not on the wire (shows `card`, `waiting_reason`, last response in the session view); nothing returns scope after a reload behind an `HttpOnly` cookie (adds `GET /api/v1/me`); the pair reply would still carry the token in its body (omitted when `cookie: true`); no all-sessions endpoint, so projects then per-project sessions; `local_address()` can be `None` or loopback-only, so the window falls back to code-and-key text; §5's "only as a bearer header" sentence must be amended rather than silently broken; the default referrer policy would leak a query, hence the fragment and `Referrer-Policy: no-referrer`.

## Decisions to overturn
- The in-memory IP-SAN reissue with `tls_config_for`, `remote_cert_ip`, `refresh_remote_certificate()` on every `OpenPairing`, a rebind that drops every paired socket on an address change, and a new `DrawCtx::remote_lan_ip`. The reference's call is no reissue: the click-through is the trust step and Safari and Chrome both offer it on a name mismatch. The plan itself calls the SAN "harmless extra" in the facts, which is not a reason to build bind-time state and a rebind path into the app.
- Shipping the page poll-only with the events socket deferred to a follow-up. The reference is poll first, socket second: the host already serves `/api/v1/events`, `connect-src 'self'` is in the CSP, and the plan's own facts show Firefox honours the exception. Deferring it leaves Chrome and Android devices without live updates for no saving.
- Not counted, but I would also change: the cookie name is a fixed `sb_token` where the reference names it with the server id so two instances on one Mac do not overwrite each other's; and the two "stop conditions" in spike 23 contradict the facts file's "build to the stated branch and never waits". The fragment instead of the issue's `?code=` query is a departure from the reference I agree with: the code never reaches the request line, a log, or a `Referer`.

## Over-specification
- The "Contract additions" and "Commit message" sections restate the body; the "Settled, The world" cross-references on nearly every claim are for the grader, not the implementer; the CSS line (16 px gutters, 44 px tap targets, light and dark) and the favicon `data:,` trick are implementer defaults; the certificate rebind paragraph is the largest single block and is the thing to cut. Roughly a fifth to a quarter of the plan.

## Hand-off
- Drop the SAN reissue and rebind, keep the events socket as a quiet upgrade beside the poll, and build to the branch instead of stopping on spike 23.

## Notes
- best: Every departure caught, including the excerpt, the `/me` scope gap and the token-in-body, with the code in a fragment and `no-referrer` so it never reaches a request line.
- worst: Builds certificate reissue and rebind-on-address-change machinery the click-through makes unnecessary, while dropping the socket the host already has.