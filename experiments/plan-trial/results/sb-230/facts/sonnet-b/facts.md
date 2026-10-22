# Facts for #230 (web control surface)

No spike was run. Every unsettled question is about a browser's behaviour on a self-signed, SAN-less certificate. None can be run here: there is no Chrome or Firefox, and Safari cannot be driven without keystroke injection. Those questions go to the owner or stay Unresolved.

## Settled

- `Authorization: Bearer` is the only credential read. Cookie, `Origin`, `Set-Cookie` and `Sec-WebSocket-Protocol` handling does not exist. Evidence: `src/app.rs:3127-3136` (`authenticate` reads `request.bearer`); `src/adapters/remote.rs:490`.
- `/pair` takes `device_name` from the body, defaults it to `"Device"`, and applies no length or character cap. A UA-derived name is a page-side choice, or a host-side one that needs a new cap. Evidence: `src/app.rs:3650-3656`.
- The certificate has no SANs (CN `Switchboard <server_id>`). The native pin checks the SPKI only. Evidence: `src/adapters/remote.rs:95-127`, `:168-183`.
- The SPKI is key-based. A certificate reissued from the same key still verifies (§5), so SANs can be added without breaking pairings. Evidence: `docs/remote-client.md` §5, as the investigation cites it.
- `HEAD` and `OPTIONS` are not special-cased. The method is passed through as a string and everything outside `/api/v1/` returns 404 JSON. The static branch chooses its own methods. Evidence: `src/adapters/remote.rs:361`, `:507`; `src/app.rs:3023-3025`.
- The rate windows know only `/pair` and `/info`, so page paths are unlimited today. Evidence: `src/adapters/remote.rs:586-587`, `:631`.
- Each request has an 8 KiB head limit. The unauthenticated pool is 4, and the frame drains 8 requests per pass from an inbox of 64. Evidence: `src/adapters/remote.rs:437`, `:578`; `spikes/22-remote-port/README.md` (design paragraph).
- The "one-line excerpt" is not on `SessionView`, which has `card`, `waiting`, `waiting_reason` and others. The Stop-hook `last_message` exists only in `src/adapters/hooks.rs:71` and `src/ports/events.rs:24`. Showing it means an additive `SessionView` field plus fixtures and the Swift decoder. The alternative is to use `card` and `waiting_reason`. Evidence: `control/src/lib.rs:637-666`.
- There is no "who am I" endpoint. `/info` carries no per-device scope, and `/pair` is the only place the scope is returned. With a cookie the script never sees the pair reply, so the plan must add a read (or store scope/name in the page) for the header. Evidence: `src/app.rs:3568`, `:3606-3660`.
- There is no `build.rs` and no `include` list in `Cargo.toml`, so `include_str!` of `web/` needs no packaging change. Evidence: `ls build.rs` (absent); `grep` of `Cargo.toml`.
- Node is on the dev machine (`/opt/homebrew/bin/node`), so the page's JS can get a syntax or logic check outside `cargo test`. CI was not checked. Evidence: `which node`.
- The text pairing path decodes the JSON only, through `PairingInvite.parse`. One edit there covers the CLI, the headset sheet and `SWITCHBOARD_INVITE`. Evidence: `client/Sources/SwitchboardProtocol/Info.swift:93-116`; `main.swift:105-125`.
- **Cookies are not isolated by port.** A cookie set for `https://<ip>:<port>` is also sent to any other port on that IP. `SameSite` ignores the port, as the draft names only scheme and registrable domain. A `Secure; HttpOnly` cookie is therefore sent to every https service on the Mac's address, and another service there can overwrite it. Evidence: https://datatracker.ietf.org/doc/html/draft-ietf-httpbis-rfc6265bis (§1 "cookies for a given host are shared across all the ports on that host"; §4.1.3.2).
- `Domain`-matching applies only to host names, so the cookie must be host-only (no `Domain` attribute) for an IP origin. Evidence: same draft, §5.1.3.
- `HttpOnly` hides the cookie from `document.cookie`. A `Secure` cookie is sent only over https, and Safari lacks the localhost exception, which does not matter on the LAN. Evidence: https://developer.mozilla.org/en-US/docs/Web/HTTP/Guides/Cookies
- `new WebSocket(url, protocols)` takes no headers. Credentials go in the URL, a cookie, or a subprotocol. Evidence: https://developer.mozilla.org/en-US/docs/Web/API/WebSocket/WebSocket
- A subprotocol value must be a non-empty RFC 2616 token, so a base64url token (`A-Za-z0-9-_`) qualifies. A client can only hold the connection if the server echoes one offered value in `Sec-WebSocket-Protocol` (MDN says `protocol` is empty when none is selected). Evidence: https://www.rfc-editor.org/rfc/rfc6455#section-4.1; the MDN page above.
- Browsers MUST send `Origin` on WebSocket upgrades, so a cookie-carrying upgrade can be checked against the host's own origin. A bearer header or subprotocol is not attached cross-site automatically. Evidence: RFC 6455 §4.1 item 8, link above.
- **`connect-src 'self'` does not cover `ws:`/`wss:` in all browsers.** The CSP must name `wss://<host:port>` explicitly. The host is an IP that varies, so the CSP is either built per request from `Host` or the page opens the socket from a same-origin value. Evidence: https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Content-Security-Policy/connect-src (note citing w3c/webappsec-csp#7).

## Unresolved

- Whether Safari (iOS, macOS), Chrome and Firefox let the owner click through a SAN-less, IP-host certificate with a year-4096 expiry. Tried: a search returned only forum posts, which say Chrome has checked SAN since 58 and that an IP needs an IP SAN. No vendor doc on bypass was found, and no browser is drivable here.
- Whether the exception survives an IP change, a port change, a restart or a private tab, and whether `fetch` and `wss://` work after the click-through. Not findable without a device.
- Whether `Secure` cookies are stored and sent after a certificate exception on each browser. The cookie specs say nothing about exceptions.
- Which browsers report the camera app opening an `https://<ip>:<port>/?...` QR, and whether the camera app or safe-browsing previews the URL (and so sees the code) before `replaceState`.
- How many parallel connections a page load uses against the 4-slot unauthenticated pool. This needs a real browser. Mitigation without one: answer static files on the connection thread with `Connection: close`, or count them in a separate pool.
- Whether the QR for the URL is scannable at the dialog's `QR_SCALE`. Needs a phone.
- Whether hand-built responses (`Content-Length`, keep-alive) are read correctly by the three browsers. Only rustls and tungstenite have exercised `respond`.
- Whether `tests/ui.rs` and `src/script.rs` hold assertions on the invite JSON. Not read. A `grep` for `invite|Copy invite|PairingInvite` at plan time lists the places.

## For the owner

- **Trust step: click-through (said in the dialog) or certificate download?** If click-through: say so in the paired-devices dialog and keep the `key`-vs-`/info` comparison as a guard against a stale host only. Add SANs for the current LAN IP by reissuing from the same key, or accept the warning's wording. If download: the host serves the certificate at an extra unauthenticated path, which §5 and the `src/app.rs:3014` comment must name. A SAN-bearing certificate is also needed, and the owner must install and trust it on each device by hand.
- **Cookie or script-held token?** If cookie: the token is hidden from page script, but it travels to every port on the host's IP (see Settled) and needs an `Origin` check and the "who am I" read. If script-held (`localStorage` or a subprotocol value): XSS can read it, but nothing is sent cross-port or cross-site. The WebSocket then needs the subprotocol path (server echoes the value) and the CSP of "no inline script, no external resource" is the defence.
- **Keep the JSON invite beside the URL?** If yes: `Copy invite` and the parser keep both forms and the Swift `parse` accepts both. If no: the URL is the only form and `parse` is rewritten, with fixtures and `README.md` line 277 updated.
- **Is it acceptable for the code to sit in a URL a camera app or browser sees before `replaceState`?** If yes: proceed as the issue says. If no: the QR carries only `id` and `key` and the owner types the code, so the page asks for it.
- **Which devices and browsers must work (phone, iPad, another Mac; iOS and Safari versions)?** The answer narrows the certificate and cookie choices above.
- **Is a signed bundle and a phone available for the acceptance run?** If yes: run scan, warn, see sessions, reply, revoke. If no: the plan can only claim the host-side tests and the Node check.
- **Does the card excerpt need `last_message` (additive `SessionView` field, fixtures on both sides, Swift decoder) or is `card` plus `waiting_reason` enough?** The tree answers the mechanics (Settled), but the choice is the owner's.
