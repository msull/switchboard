# Facts for issue #230 (web control surface)

No spike was run. Every open question is settled by the tree, settled by a primary source, or needs a phone or the owner. The worktree is clean.

## Settled

Tree (line numbers are in this worktree):
- `SessionView.card` is the card's word ("working", "idle", "waiting on you", "exited (0)"), not a longer excerpt: `control/src/lib.rs:645-646`, example at `:944`. The plan must say what "one-line excerpt" means for the page (`card` plus `waiting_reason`).
- A session entering or leaving `waiting` emits `changed {waiting}` and `changed {session}` from the read-model diff pass: `src/app.rs:2610-2640` (`diff_pass`), tested at `tests/remote.rs:2764`, specified at `docs/remote-client.md:240,247,489`.
- A read-scope device's `send` gets 403, code `scope`, message "this device reads only; raise its scope on the host": `src/app.rs:3166-3173`. No route returns a device's own scope, so the page must store it at pairing or learn it from this refusal (notes §3).
- The certificate has no SAN: `src/adapters/remote.rs:102` (`CertificateParams::new(vec![])`, CN only).
- The validity period is rcgen 0.14's default, 1975-01-01 to 4096-01-01: `~/.cargo/registry/src/*/rcgen-0.14*/src/certificate.rs:83-90`.
- The SPKI hash comes from the key, not the certificate, so a certificate reissued from the same key keeps the pin: `src/adapters/remote.rs:119` and the `debug_assert` at `:120`. No reissue path exists in the tree; `make_identity` always generates a new key, and `load_identity` (`src/app.rs:2782`) clears every device on a mismatch. A SAN or validity fix for existing installs needs new code that rebuilds the certificate from the stored key.
- The invite's JSON form is read in exactly three places. These are the only readers of `PairingInvite.parse`, besides the Swift fixture test `client/Tests/SwitchboardProtocolTests/FixturesTests.swift:161`: `client/Sources/switchboard-remote/main.swift:111`, `client/Sources/SwitchboardUI/BrowserView.swift:106`, and `BrowserView.swift:883`. The type is `client/Sources/SwitchboardProtocol/Info.swift:94`. `ConnectionManager.swift:474` only constructs one.
- `local_address()` returns a single IP, and an IPv6 address would need brackets in a URL: `src/adapters/remote.rs:962-970`; format at `src/ui/remote.rs:158`.
- A root route would not break the two existing tests as written, because neither asserts on `/` (notes §4 item 15; I did not re-read their exact asserts).
- Facts about the host's own behaviour are in the investigation notes §1 and §4 (`respond` has a fixed head, `read_request` keeps no cookie or `Origin`, `respond_upgrade` echoes no subprotocol, a static path would take a pool slot, a revoked device and a rotated key give the same 401). I did not re-verify them here.

Primary sources:
- Subprotocol token: the server MUST NOT send `Sec-WebSocket-Protocol` unless it echoes one value the client offered, and the client MUST fail the connection if the 101 names one it did not offer, or omits it when one was requested. https://www.rfc-editor.org/rfc/rfc6455 (§1.9, §4.1 item 6, §4.2.2). So the 101 must echo the offered value, which means the token (or a token-derived value) appears in the handshake header. It would show up in any proxy or log that records headers.
- Origin: browsers MUST send `Origin` on a WebSocket handshake, and the server MAY validate it and answer 403. Without validation it "will accept connections from anywhere". RFC 6455 §1.3, §4.1 item 8, §4.2.2. Cookies are sent on the handshake like any header (§4.1 item 12). A cookie-authenticated socket therefore needs an `Origin` check in the host: SameSite is a second line, but RFC 6455 puts the cross-origin duty on the server.
- `HttpOnly` hides the cookie from script but it is still sent on `fetch`/XHR; `Secure` sends only over https; `SameSite=Strict` sends only for same-site requests: https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Set-Cookie.
- `history.replaceState` replaces the current entry's URL and adds none; the pre-script URL is not kept as a separate entry: https://developer.mozilla.org/en-US/docs/Web/API/History/replaceState. Caveat: the browser has already requested `/?code=…` from the host, and what it records in its own URL history or autocomplete store is not covered (see Unresolved).
- CSP `connect-src 'self'` does NOT reliably cover `ws:`/`wss:` in all browsers (MDN note, linking https://github.com/w3c/webappsec-csp/issues/7). The plan should list `wss://<host>:<port>` explicitly, or learn the host from `location.host`. Because the page's script is static, the CSP is static too. A `'self'`-only policy plus a same-origin WebSocket URL is therefore not safe to assume. https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Content-Security-Policy/connect-src
- `Cache-Control: no-store` means no cache of any kind may store the response: https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Cache-Control. Put it on the `/?code=…` response and on the page itself (the issue's "no caching across versions" is stricter than `no-cache`).
- Cookies are scoped by host, not port (MDN's page does not state this; it is general knowledge, so treat it as a lead). A cookie set by `https://ip:port` is sent to any other port on the same IP. This matters if another service on the host's address sets or reads cookies. It is an argument for a cookie name with the `__Host-` prefix, which the plan should confirm at https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Set-Cookie (that prefix requires `Secure`, `Path=/`, and no `Domain`).
- Apple lists SAN (DNS name in SAN, not CN) and an 825-day cap for TLS server certificates issued after 1 July 2019: https://support.apple.com/en-us/103769. The page does not say whether this binds a self-signed, user-accepted certificate, so the effect on Safari's click-through is not settled (see Unresolved). The existing certificate fails both on paper (no SAN, validity to year 4096).

## Unresolved

- Whether iOS Safari or Chrome offers a click-through for an IP-literal origin with a certificate that has no SAN and 2000+ years of validity. I tried Apple's TLS requirements page, which states the rules but not how they apply to a self-signed certificate the user accepts. It needs a phone. Under For the owner.
- Whether the accepted exception covers `wss://` to the same origin on iOS Safari and Chrome. I found no primary source, and I have not verified it in a browser. It needs a phone.
- Whether a browser sends the `Secure; SameSite=Strict` cookie on the WebSocket upgrade for a self-signed origin. RFC 6455 allows cookies on the handshake, but I found no browser-vendor source. It needs a browser run.
- Whether iOS Safari or Chrome keeps the `/?code=…` URL in its history store, autocomplete or tab-restore data after `replaceState`. MDN covers only the history entry. No source found; it needs a phone.
- How many concurrent connections iOS Safari and Chrome open to one origin, and how long they hold idle keep-alive ones. Chromium's documented limit is 6 per host, but I did not fetch it; the Safari figure I know only from memory. The pool is four (`src/adapters/remote.rs:567`). It needs a measurement on a phone against the real port.
- Whether the HTTP cache keys on the full URL including the query. This is RFC 9111's rule (the target URI), but I did not fetch it. `no-store` makes the question moot for these responses.
- Whether the user-agent string gives a stable device name. I did not fetch a source. Known from memory only: iPadOS Safari sends a desktop Macintosh UA by default, and Chrome freezes parts of its UA string. The plan should treat the name as a best-effort label the owner can read, not an identity.
- Whether `localStorage` or cookies for an IP-origin page survive iOS storage eviction. No source found; it needs a phone over days.
- Whether a phone camera opens an `https://` URL that has a bad certificate. It needs a phone.
- Whether a session entering `waiting` reaches the page without a reload, and whether `send` arrives in the pane. The host side is settled above; the page is not written yet.

## For the owner

1. Is the certificate click-through the trust step, or does the host offer its certificate for download?
   - Click-through: the plan says in the paired-devices dialog that the first visit shows a warning and that the owner must check the address. It keeps §5's rule by serving only the page, `/info` and `/pair` before pairing. The plan also decides whether to reissue the certificate with the IP as a SAN, because Apple's page makes a no-SAN certificate doubtful.
   - Download: the plan adds an unauthenticated path that serves the certificate (public data). The owner then installs it as a profile and trusts it on the device, which a phone cannot do without leaving the browser. That is more steps, and it must also cover the SAN.
   - Default if no answer: click-through, with a SAN reissue if a phone shows the warning cannot be bypassed.
2. Does the `/pair` reply carry the token in the body as well as the cookie?
   - Yes: native clients keep working unchanged, but the token is readable by page script on the browser path unless the host sets the cookie and strips the body only for a browser request.
   - No (cookie only): the plan needs a distinguishing signal (a request header or a separate path) so the native clients still get the body, and an `Origin` check on the WebSocket. This is the safer default.
3. Does the URL replace the JSON invite everywhere?
   - Yes: the CLI and both headset paste sites (`main.swift:111`, `BrowserView.swift:106`, `:883`) need a URL parser, and the fixtures change.
   - No: the pairing window keeps a JSON "Copy invite" next to the QR URL, and nothing in Swift changes. Cheaper, and the headset has no camera.
4. Should "revoked" and "identity changed" be different messages? The host cannot tell them apart today (same 401 at `src/app.rs:3146`). If yes, the plan adds a code to that refusal, and the page stores the key it paired with so it can compare against `/info`. If no, the page says "pair again" for both.
5. Does the device record gain a field for scope or the page's name? If no, the page stores scope at pairing and learns a change from a 403 `scope`. If yes, a `SCHEMA_VERSION` bump and a new `/me` route are required.
6. Which pool does an unpaired page load count against? Today the four-connection pool (`src/adapters/remote.rs:567`) with no rate window for static paths. If the pool stays, the plan serves the page on the connection thread with `Connection: close`, or raises the pool; if separate, it adds a window for static paths.
7. What does the page show with no code and no token? Plan default: a "pair this device from the Mac" message plus the host name from `/info`, with no sessions.
8. Is the page served while "Allow devices on the local network" is off? The port does not listen on the LAN in that case (`docs/design.md` 3130-3160), so the page would be loopback-only. Plan default: same switch, no separate one.
9. Is the browser's scope chosen in the pairing window as today? Plan default: yes (`src/ui/remote.rs`, no change).
