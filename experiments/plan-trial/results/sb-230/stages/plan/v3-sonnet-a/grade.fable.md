All checks done. Every cited line is real; a few secondary anchors are off by a handful of lines (`bind_remote` 2936 not 2926, `Limits` 623 not 621, `connection` 770 not 763), which is within tolerance.

```
| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| v3-sonnet-a | 3 | 3 | 1 | 2 | note |
```

## Grounding
- `control/src/lib.rs:636-688` (`SessionView`, no excerpt): found (struct spans 636-688, has `card`, `waiting`, `waiting_reason`, no `last_message`)
- `src/bin/switchboard-hook.rs:37` (hook writes `last_message`): found (`("last_assistant_message", "last_message")` at line 37)
- `remote_info`, `src/app.rs:3568`: found (fn at 3568, unauthenticated, `capabilities` list at 3580 with no scope)
- `authenticate` (`src/app.rs:3125-3153`): found (doc comment 3125, fn 3127-3152, reads only `request.bearer`, both refusals are 401 `unauthorized`)
- `src/app.rs:3023` (`/api/v1/` strip, 404 on the frame): found (`strip_prefix("/api/v1/")` at 3023, `refuse(.., 404, "not_found", ..)` at 3024, inside `serve_remote`)

## Departures caught
- Safari refuses `wss://` on a clicked-through certificate: caught. Polling is the default, the socket a run-time upgrade with fallback and a "live updates unavailable" note; acceptance holds on polling.
- Certificate download is not an option: caught with a wrong fix. The plan picks the click-through, correctly, but then reissues the certificate with SANs, `serverAuth` and an 800-day validity, which the reference rejects as binding the identity to an address and an expiry the spec avoids.
- Each page file is a connection: caught, partially. `Connection: close` on static replies and a separate rate window, but three files rather than two, no `MAX_UNAUTHENTICATED` change, and "what the pool leaves for a second device" is deferred to a device run.
- Device name is the page's to send: caught (`navigator.userAgent` plus `maxTouchPoints` into `device_name`, no host parsing).
- Found beyond the reference: the excerpt is not on the wire (handled by reading `last-response` for waiting sessions only); scope has no source after pairing (`GET /api/v1/device`); "revoked" and "never paired" are the same 401 (client-side marker); §5's relay sentence does not hold for a browser; the 404 is decided on the frame after queueing; `crypto.randomUUID` is secure-context only; the Swift `PairingInvite` footprint.
- Missed from the errata: the invite carries a LAN address while the host may be bound to loopback. The pair reply carrying the token in the body alongside the cookie is noted and kept on purpose.

## Decisions to overturn
- Certificate reissue over the same key: `Identity` gains two fields, `IDENTITY_SCHEMA_VERSION` bumps, `CertificateReissued` and `CheckCertificate` enter the core contract, and `bind_remote` reissues and rebinds (dropping connections) when the LAN address moves. The reference says no reissue: the identity model deliberately binds to no address or validity window, Safari still refuses `wss://` by Cockpit's report, and nothing a browser does is proven to need it. Reverse.
- Token transport: three transports (bearer, `token.<t>` subprotocol, cookie) with a run-time probe, and `localStorage` as the default "token mode" when the probe fails. The reference picks the cookie alone with `read_request` filling `bearer` from it, and names `localStorage` as the call to reverse. Reverse to cookie-only; the subprotocol and probe go with it.
- Three static files where two would do is marginal and I did not count it. The `Origin` rule, the `Path=/api/v1/` cookie, `/device`, and `no-store`/`nosniff` on JSON replies I would keep.

## Over-specification
- The certificate section (record fields, version bump, rcgen notes, `certificate_is_stale`, the new action and effect, DER-parsing tests): about 15% of the plan, all removable with the decision.
- The transport probe, subprotocol echo and the extra tests for them: about 8%.
- The "Contract additions" block restates the body, and the Facts/Unresolved citations are threaded through sentences an implementer does not need. Roughly a quarter to cut in total; the rest is dense but load-bearing.

## Hand-off
- Drop the certificate reissue and the transport probe: ship cookie-only auth with `read_request` filling `bearer` from the cookie, keep polling-first, and everything else stands.

## Notes
- best: Polling-first with the socket as a probed upgrade, plus finding the excerpt, scope, revoked-state and relay-sentence gaps the issue did not know about, each with a concrete fix.
- worst: A certificate reissue pipeline that touches the identity record, the schema version, the core contract and rebinding on address change, for a browser outcome no source shows it buys.