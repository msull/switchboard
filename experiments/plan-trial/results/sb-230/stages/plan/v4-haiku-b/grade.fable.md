```
| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| v4-haiku-b | 3 | 3 | 1 | 2 | note |
```

## Grounding
- `docs/remote-client.md:201,207` (§4.3 says the excerpt is `last_message` on session metadata): found (line 207 ends the `session.last_response` bullet with exactly that sentence)
- `src/ports/events.rs:24` (`last_message` is a field of the Stop event only): found (`Stopped { last_message: Option<String>, pending }` at 23-29)
- `src/core/events.rs:133-134` (core keeps `last_stop_at` and `pending_at_stop`, drops the text): found (the `Stopped` arm sets exactly those two and ignores `last_message`)
- `src/app.rs:888-898`, `caption_of` at `:4105` (card caption is the last line of the pane log held in UI state): found (`cold_scrollback` writes `ui_state.captions`; `fn caption_of` is at 4105)
- `control/src/lib.rs:637` (`SessionView` has no excerpt): found (struct at 637, fields `card`, `last_stop_at_ms`, `waiting`, `waiting_reason`, no excerpt)
- Also verified: 28 `RemoteReply {` sites in `app.rs`, `authenticate` at 3127-3153, `refuse` at 3754-3772, `IDENTITY_SCHEMA_VERSION = 1` at `ports/remote.rs:17`, `new_device` at `core/remote.rs:166`, the four named tests in `tests/remote.rs` at 682/1829/2892/3226, `BrowserView.swift:106,883` calling `parse`, `Info.swift:114-116`, and spikes end at 22 so `23-browser-page` is the next number.

## Departures caught
- WebSocket will not open from Safari: caught, as "unknown" rather than a known WebKit limitation, but the build is the reference's (polling at 2 s is the default, the socket an enhancement with a "live updates unavailable" state).
- Certificate download is not an option: caught, click-through is the trust step and download is a follow-up. But the plan adds a certificate reissue anyway (see decisions).
- Each file of the page is a connection: caught, page replies answered on the connection thread with `Connection: close` so the slot is freed at once; page is three files, not the reference's two.
- Device name is the page's to send: caught, `device_name` built by the script from `navigator.userAgent`, no host parsing.
- Found beyond the reference: the excerpt is not on the wire (reads `last-response` on `last_stop_at_ms` change, amends §4.3 l.207); scope is not served after a reload (new `GET /api/v1/device`); the pair reply would carry the token in its body (`transport: "cookie"` omits it); "revoked" and "never paired" are the same 401 so the page says "no longer paired", not "revoked"; the URL key does not stop a relay that copies it; `replaceState` runs after the request line was sent so the code may be recorded; the facts file's RemoteReply count was wrong (28, not 15). The loopback-vs-LAN invite mismatch is not named.

## Decisions to overturn
- **Certificate reissue with an IP SAN, `serverAuth` and 365-day validity, renewed at `bind_remote`, with two new `Identity` fields, an `IDENTITY_SCHEMA_VERSION` bump and `AppAction::CertificateRenewed`.** The reference says no reissue: it ties the identity to an address and an expiry the spec deliberately avoids (§5, §9), and Safari still refuses `wss://` by the reports the facts file itself cites. A large block of machinery for no gain.
- **Three static files (`/`, `/app.css`, `/app.js`).** The reference wants at most two, CSS inline in the HTML under a hash. Small, but it is one more unauthenticated connection per load against a pool of four.
- Not counted but would adjust: while the socket is open the open session's content is polled only every 15 s and `watch` frames are deferred, so a screen on Chrome lags; the reference sends `watch` for the open session. Also the 30-day `Max-Age` silently signs a browser out while the device stays listed as paired.

## Over-specification
- The whole certificate section (Identity fields, renewal rule, `renewed_certificate`, rcgen caveat, its unit and core tests) is cut if the decision is reversed, roughly a sixth of the plan. The "facts, Settled / Unresolved N / For the owner X" cross-citations throughout, departure 10's count correction, the "Contract additions" restatement, the acceptance-by-whom list and the nine follow-ups add noise an implementer skips. About a quarter to a third overall; the test list, the nine-step page script and the header table are the parts that earn their length.

## Hand-off
- Drop the certificate reissue and everything attached to it (Identity fields, schema bump, `CertificateRenewed`, bind-time renewal) and inline the CSS so the page is two files; the rest is implementable as written.

## Notes
- best: Found the excerpt-not-on-wire, scope-after-reload and token-in-pair-body gaps from the tree and built polling-first with concrete, named tests against the existing helpers.
- worst: Reissues and renews the certificate with an address-bound SAN and expiry, adding a schema bump and a core action, against the spec's identity model and without unblocking Safari's socket.