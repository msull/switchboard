| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| L | 3 | 2 | 1 | 2 | note |

## Grounding
- `make_identity` (`src/adapters/remote.rs:95-127`): found (P-256 key, `CertificateParams::new(Vec::new())`, CN `Switchboard <id>`, no SAN, lines 95-127 exactly)
- `agent_excerpt` (`src/ui/cards.rs:558`): found (`fn agent_excerpt(c: &Conversation)` at line 558, computed in the UI from the parsed transcript)
- `SessionView` (`control/src/lib.rs:637-688`): found (struct spans 637-688; no excerpt or `last_message` field, so the plan's departure is correct)
- `client/Sources/SwitchboardUI/BrowserView.swift:106,883`: found (`PairingInvite.parse` called at 106 in `bootOnce` and at 883 in `pair()`)
- `client/Sources/switchboard-remote/main.swift:111`: found (`invite = try PairingInvite.parse(rest[0])` at 111; usage line at 12)
- Spot checks beyond five: the 28 `RemoteReply {` literals in `app.rs` and 2 in `remote.rs`, `bind_remote` 2936, `tls_config` 2957, `RevokeDevice` arm 486, `authenticate` 3127, `remote_pair` 3606, `remote_info` 3568, `http` 245, `subscribe_at` 1960, `FixturesTests.swift:157`, `design.md:250-254` and `3573`, `IDENTITY_SCHEMA_VERSION` 1 all match.

## Departures caught
- WebSocket will not open from Safari: caught with a wrong fix. The plan has a polling fallback after two socket failures, so the page works, but it attributes the risk to cookie-on-upgrade rather than WebKit refusing `wss://` to a clicked-through certificate, and it ships socket-first so the device the page is for lands in fallback mode by accident rather than design.
- Certificate download is not an option: caught (rejects download, correct reasons) but with a wrong fix. It goes on to reissue the certificate with SANs, EKU and 397-day validity on the premise that today's certificate cannot be clicked through at all. Self-signed certificates without a SAN are proceedable in Safari and Chrome (the 825-day and SAN rules govern trust-store trust, not the warning). The spike has no decision rule for "legacy click-through works, drop the reissue".
- Each page file is a connection: partially caught. The plan puts the files in the unauthenticated pool and queues API calls, but the browser fetches `/app.js` and `/app.css` in parallel outside the script's queue, the plan keeps three files, and it does not say what the pool leaves for a second device.
- Device name from the page's script: caught (`navigator.userAgent`, iPad via `maxTouchPoints`).
- Found beyond the reference: no one-line excerpt on the wire (verified, `SessionView` has none); scope is unrecoverable after reload behind an `HttpOnly` cookie, so a `/device` probe is needed; the code in the query hits the request line of the first `GET` before any key comparison, so it goes in the fragment. All three are real and well reasoned.

## Decisions to overturn
- The certificate reissue machinery (`certificate_names` on `Identity`, schema bump, `CertificateReissued` action, rebind on `OpenPairing`, a builder with SAN per LAN address and 397-day expiry). The reference keeps today's certificate and the click-through as the trust step; a reissue ties the identity to an address and an expiry §5 and §9 avoid, and its premise is likely false.
- The fixed cookie name `__Host-switchboard`. The reference names the cookie with the server id because cookies ignore ports and two instances on one Mac overwrite each other's.
- Minor, not counted: socket-first with poll on failure (reference polls first); three files instead of two.

## Over-specification
- The whole certificate section (builder, `certificate_current`, fingerprint line, reissue triggers, its tests, its spike half) is roughly a fifth of the plan and goes away with the reversal. The "Contract additions" block restates the adapter and frame sections. Many exact UI strings and the per-error page wording are a spec an implementer would write themselves. About a quarter to a third could be cut.

## Hand-off
- Drop the certificate reissue and keep today's certificate with the click-through as the trust step, make polling the page's default and the socket an upgrade, and put the server id in the cookie name.

## Notes
- best: every line reference checks out, and it found three real departures the reference lacks (no excerpt on the wire, scope after reload, code in the fragment).
- worst: a large reissue subsystem built on the unverified premise that the current certificate cannot be clicked through, with a spike that has no rule to abandon it.