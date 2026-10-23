Everything checked is real. I have what I need to score.

```
| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| AF | 3 | 2 | 2 | 2 | note |
```

## Grounding
- `src/adapters/remote.rs:102-106` (`CertificateParams::new(Vec::<String>::new())`, CommonName only): found, exactly those lines in `make_identity`.
- `src/app.rs:3023` (`strip_prefix("/api/v1/")` else 404): found, line 3023-3024 in `serve_remote`.
- `docs/remote-client.md:207` (`last_message` "a field of the session's metadata"): found, the sentence is there verbatim.
- `control/src/lib.rs:637-688` (`SessionView`, no excerpt field): found, struct spans 637-688 and has `card`, `waiting`, `waiting_reason`, no `last_message`.
- `src/ports/events.rs:24` (`last_message` only in the Stop hook payload): found, `Stopped { last_message: Option<String>, .. }`.
- Also checked: `agent_excerpt` at `cards.rs:558`, `authenticate` 3127, `remote_pair` 3606, `tls_config` 144, `local_address` 962, `RemoteStore` trait at `ports/remote.rs:111`, `write_fixtures` 1587, `tests/ui.rs:7424`, `script.rs:730`, README rows 383/423, `GET ops/{op}` exists. `device_name` is at `app.rs:3650`, not 3641, within the same function.

## Departures caught
- WebSocket will not open from Safari: caught; poll is the base, socket relaxes the timer to 30 s, `watch` for content kinds.
- Certificate download not an option: caught; click-through is the trust step, download rejected with reasons. But the plan goes on to re-issue the certificate (see decisions).
- Each page file is a connection: mostly missed. The plan keeps three files, asserts they ride "the same connection a browser then uses", and only defers "raising the 4-slot pool" to a follow-up. It does not say what the pool leaves for a second device.
- Device name is the page's to send: caught, `navigator.userAgent` into `device_name`.
- Found beyond the reference (errata items): excerpt not on the wire with the spec line corrected; no endpoint reports scope after a reload, so `GET /api/v1/device`; the pair reply carrying the token even with a cookie, solved by `delivery: "cookie"`; `SameSite` ignoring the port, solved by an `Origin` check and a per-installation `__Host-` cookie name. The loopback-bind/LAN-address mismatch is only half handled (empty address falls back to 127.0.0.1).

## Decisions to overturn
- The browser-facing certificate re-issue: a persisted `web-cert.json`, a `ResolvesServerCert` that runs `local_address()` on every handshake, SANs for the current address, `ServerAuth` EKU, 397-day validity, re-make on address change or within 30 days of expiry, deletion on key rotation. The reference's decision 1 is no reissue: the click-through works with today's certificate (Safari and Chrome both offer it on a name mismatch too), and the spec deliberately binds the identity to no address or validity window. The plan's premise that a missing SAN blocks the click-through is overstated by its own facts file ("not settled").
- Not reversed but noted: three files rather than two, and `connect-src 'self' wss:` rather than `'self'` (the plan's WebKit reason is acceptable).

## Over-specification
- The certificate machinery (the record section, step 2, the certificate test block, its docs and contract lines) is roughly a sixth of the plan and would be cut with the decision above.
- The "Contract additions" section restates the steps, and the spike step largely duplicates the end-to-end tests and the owner's O1 question. Together about a quarter of the plan is cuttable; the rest (headers, cookie attributes, failure words, test list) an implementer needs.

## Hand-off
- Drop the certificate re-issue and `web-cert.json` entirely and serve today's certificate; fold `app.css` into the HTML so the page is two files, and say what the 4-slot pool leaves for a second device.

## Notes
- best: the cookie design is complete and correct (`__Host-` per-installation name, `delivery: "cookie"` omitting the token, `Origin` on writes and upgrades, clear on 401) and the poll-first live path with `watch` is exactly §4.4.
- worst: a whole persisted-certificate subsystem built on an unsettled premise, binding the identity to an address and an expiry the spec avoids.