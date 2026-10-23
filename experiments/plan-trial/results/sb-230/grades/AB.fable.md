| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| AB | 3 | 3 | 2 | 2 | note |

## Grounding
- `control/src/lib.rs:637` (`SessionView` has no excerpt field): found. `SessionView` begins at 637 with `card`, `waiting`, `waiting_reason`, no `last_message`.
- `src/ui/cards.rs:558` (the Mac builds its excerpt from the transcript): found. `agent_excerpt(c: &Conversation)` is at 558.
- `client/Sources/SwitchboardUI/BrowserView.swift:152-155` (headset reads `last-response` per session): found. The loop over `conversationHolders.keys` calling `manager.lastResponse(for:)` is at 152-155.
- `src/adapters/remote.rs:95-110` (certificate with no SAN, no EKU, `not_after` 4096): found. `make_identity` at 95 builds `CertificateParams::new(Vec::<String>::new())` with only a CN; the doc comment says year 4096.
- `src/ports/remote.rs:17` (`IDENTITY_SCHEMA_VERSION` is 1): found, exactly. Also checked `model.rs:12` (16), `remote.rs:247` (`spki_sha256_of_cert`), `:359-371` (`RemoteRequest`), `:389` (`RemoteReply`), `app.rs:2920` (`set_remote_port_override`), `action.rs:1248-1257`, `app.rs:3754-3772` (`refuse`), 28 `RemoteReply {` literals, `tests/remote.rs:682`, `tls_config` at 144 with `with_single_cert`, rustls 0.23.45: all correct.

## Departures caught
- WebSocket will not open from Safari: caught. Treated as unresolved rather than settled, but the page is poll-first with the socket as an enhancement and a "Live updates unavailable" header state, which is the build the reference asks for.
- Certificate download is not an option: caught, with a wrong fix. The plan picks the click-through and defers download, which is right, but then reissues the certificate anyway (SAN, EKU, 800 days), the thing the reference says the download would have required and the spec avoids.
- Each page file is a connection: caught. Static replies are answered on the connection thread with `Connection: close`, no rate window, and a burst test that `/pair` still succeeds. Three files rather than two, which `Connection: close` makes tolerable.
- Device name is the page's to send: caught. The page fills `device_name` from the user agent and makes it editable before a Pair tap, with the iPad-reports-as-Mac fact as the reason.
- Beyond the reference: excerpt not on the wire (reads `last-response` for at most 8 waiting Claude sessions, `card`/`waiting_reason` otherwise); no scope read after reload (adds `GET /api/v1/device`); invite prints a LAN address while bound to loopback (dialog says so); pair reply carries the token in its body (`cookie: true` omits it); `SameSite` does not isolate ports on one IP (cookie named by server id).

## Decisions to overturn
- The certificate reissue over the same key: SAN for the LAN IP, serverAuth EKU, 800-day validity, `certificate_facts`, `reissued`, `refresh_certificate` on bind, on every poll while pairing is open and hourly on `Tick`, a `CertSlot` `ResolvesServerCert` swap, `CertificateReissued` action, `certified_address` feeding the QR. The reference's decision 1 is no reissue: the identity model binds nothing to an address or a validity window, and Safari refuses `wss://` regardless. This also contradicts the plan's own stated trust step, since the click-through needs none of it.
- Not reversed but disputed: the `X-Switchboard-Page: 1` header as a precondition for cookie auth. The reference says an `Origin` check is enough and a CSRF token is over-specification; the plan already parses `Origin` for the upgrade, so the header is a second mechanism. I would leave it rather than block on it. The fragment instead of the query string I would keep; it is stronger than the issue's `?code=` and the reference does not forbid it.

## Over-specification
- The whole certificate-reissue thread (departure bullet, Record section, adapter `CertSlot`, app `refresh_certificate`, `DrawCtx.remote_address`, the address-override seam, two test groups) is roughly a quarter of the plan and should not be built.
- The Commit section restates the contract additions already listed; the count of 28 literals, the explicit `wss://<host>` in `connect-src` with its justification, and the per-field `Content-Type` list are detail the compiler and a glance at CSP would supply. Around 30% in total.

## Hand-off
- Drop everything under certificate reissue and keep the certificate as issued; the click-through you already chose is the whole trust step, and the rest of the plan stands.

## Notes
- best: Poll-first page with a `cookie: true` pair reply that omits the token, plus the excerpt, scope-after-reload and loopback-address departures the issue missed, each tied to a verified line.
- worst: A full certificate lifecycle (SAN, EKU, expiry, hourly renewal, hot-swappable rustls resolver) that the spec's identity model deliberately avoids and that buys nothing once the click-through is the trust step.