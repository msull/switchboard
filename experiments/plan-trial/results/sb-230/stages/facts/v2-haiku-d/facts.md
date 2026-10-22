# Facts for issue #230 (browser page on the remote port)

No spike. Every question that turns on Safari or iOS behaviour needs a device, and a Chrome or headless run would answer a neighbouring question. Queries are Qn, fetches Fn, listed at the end.

## Settled

Tree (this worktree, 7fe8949):
- The card excerpt is a UI cache, not on the wire or in the record. Captions live in `ui_state.captions`, filled from the transcript (src/app.rs:888-898, 1184-1185; `caption_of` at :4104). No `last_message` or excerpt field in `control/src/lib.rs` or `src/app.rs` (grep). The plan adds an app-layer read-model field, or fetches `last-response` per card as the headset does.
- `make_identity` always generates a new key (src/adapters/remote.rs:95-101), and the key PEM is saved to the secrets store (src/app.rs:2866-2871). A reissue over the same key is therefore possible. rcgen 0.14.10 (Cargo.lock:4034-4035) has `KeyPair::from_pem` (F11). The SAN and validity fields of `CertificateParams` were not checked.
- The current certificate has a CN and no SAN (src/adapters/remote.rs:101, 103-106) and sets no EKU. It fails Apple's SAN and EKU rules (F1). Its validity was not checked against rcgen's defaults, so the notes' year-4096 claim is unverified.
- Every reply is written with `Content-Type: application/json` and `Connection: keep-alive` (src/adapters/remote.rs:530-552, :545). The listener reads no `Cookie`, `Origin` or `Sec-WebSocket-Protocol` (grep of src/adapters/remote.rs and src/app.rs, which finds none).
- No literal `RemoteRequest {` exists in `src/` or `tests/` (grep), so new request fields touch no construction site.
- `MAX_UNAUTHENTICATED = 4` (src/adapters/remote.rs:567), enforced at accept (:702).
- Watches with subscribers survive eviction when the set is over `WATCH_CAP` = 128 (src/content.rs:42; src/app.rs:1787-1789). A page's watches cannot push out a headset's live watch. `WATCH_IDLE` is 60 s (src/content.rs:40).

Standards and vendor documents:
- Apple: TLS server certificates issued after 1 July 2019 need a DNS name in SAN (CN is not trusted), an EKU with serverAuth, and validity of 825 days or fewer (F1, Q2). Whether this applies to self-signed or user-trusted roots is not stated (see Unresolved). The plan builds a reissue that meets all three either way.
- A manually installed root is trusted only after Settings > General > About > Certificate Trust Settings > "Enable full trust for root certificates" is switched on (F12).
- WebKit bug 41419 is open (status NEW; reported 2010 against Safari 5; last vendor-side entry a 2014 Radar import). It reports that a self-signed `wss://` server is refused until the owner sets "always trust", after which it connects (F2, Q3). No source contradicts it. It is one report on an old Safari, so current iOS is untested.
- RFC 6455 §4.2.2: the server echoes one offered subprotocol or none (steps 4, 5.5). §4.1 item 6: a client must fail a response naming a protocol it did not offer (F4). The plan echoes exactly one offered value.
- WHATWG WebSocket: every requested protocol is combined into the handshake's `Sec-WebSocket-Protocol` header (F7).
- RFC 6265 §8.5: cookies do not isolate by port, and servers SHOULD NOT run mutually distrusting services on different ports of one host while storing security-sensitive data in cookies (F3, Q5).
- W3C Secure Contexts §3.1 step 3: an `https` origin is potentially trustworthy by scheme. The text does not mention certificate errors, so per the spec a click-through page is a secure context (F6). Browser behaviour is not checked here.
- MDN: `crypto.randomUUID()` is available only in secure contexts (F5, Q7).
- OWASP WebSocket cheat sheet: validate `Origin` on every handshake; browsers include cookies in WebSocket handshakes, which makes cross-site hijacking possible (F9). RFC 6455 §10.2 was not read.
- MDN: `replaceState()` replaces the current session-history entry and requires the same origin (Q15). It does not reach browser-wide history (see Unresolved).

## Unresolved

U1. Does iOS Safari carry a click-through for `wss://` to a self-signed certificate? Tried Q1, Q3, F2. Bug 41419 is about an old Safari, and the wider searches found only forum posts. Build to: the page does not depend on the WebSocket. When the socket fails it polls `GET` for the revision, and the trust step in O1 is offered.

U2. Does iOS trust a self-signed leaf installed by profile once its full-trust switch is on? Tried Q14. Results are forum answers, plus iOS 18 threads reporting private roots missing from the list (Apple forum thread 764673, not fetched). F12 covers roots only. Build to: the reissued certificate, with the trust step optional until a device run. If the leaf does not show, a local CA is needed (O3).

U3. Connections per host that a browser opens for the page load. Tried Q6. Sources are secondary (Diffusion docs; Blue Triangle): six per host for Chrome and Safari of that era. Current iOS Safari is unknown. Build to: assume at least six. Static `GET`s must not let the page load hold all four unauthenticated slots (src/adapters/remote.rs:567, :702). Either the static paths get their own pool, or they are answered without taking a pool slot.

U4. Does Safari send a `SameSite=Strict` cookie on a same-origin `wss://` handshake? Tried Q8, Q10, F10. Apple forum thread 104488 (June 2018, no reply) reports that Safari omits HttpOnly, Secure, SameSite=Strict cookies on the second `wss` handshake, and that removing HttpOnly fixes it. A tldraw issue (Q10) shows a cross-site Safari handshake sends no cookies, which is a different case. Build to: the plan does not rely on a cookie for the WebSocket until a device run passes. The subprotocol path does not depend on this.

U5. Does a click-through survive reloads, launches and an IP change? Tried Q11: no source found. Build to: the page treats the warning as present on every visit and does not assume the exception persists.

U6. Does a click-through page keep `isSecureContext` true, and do Secure cookies stick, in iOS Safari? The spec says yes (F6); no browser check. Build to: use `randomUUID` only if the device run shows `isSecureContext` true.

U7. Does `<name>.local` resolve and get a trusted certificate in iOS Safari? Tried Q12. Apple support 101903 (lead) warns that `.local` may not resolve through unicast DNS. Build to: the IP-only URL. `.local` is out of the first cut.

U8. Does the URL with the code reach browser-wide history, "recently closed" or tab sync before `replaceState` runs? Tried Q15, which covers session history only. Build to: the plan may claim only that the session entry is replaced.

U9. Do Chrome, Firefox and Safari keep a `Secure` cookie set by an https response with an untrusted certificate, on an IP-literal host? Tried Q9: search snippets only (MDN's Secure rule; Chrome and Firefox 52). Build to: the cookie name carries the `server_id` (RFC 6265 §8.5, Settled above), and the device run decides.

U10. RFC 6455 handshake when a protocol was requested but none is echoed: Q4 turned up a claim that browsers fail and a conflicting client PR. Build to: always echo, which satisfies both readings.

## For the owner

O1. Trust (decision 1). The click-through may not carry the WebSocket (U1). Option A: add a pre-pairing download of the public certificate, and the owner installs it and switches on full trust on the device. Option B: no install step; the page works by polling if the socket is refused. Both branches work if the plan builds the reissue and the polling fallback. Recommend: build both, and say in the paired-devices dialog which one the owner used.

O2. Token transport (decision 2). The cookie works only if U4 passes on a device. The subprotocol token works either way, but keeps the token in page storage, where a script on the origin can read it. Both branches work if the plan builds the subprotocol path; the cookie is added only on a pass.

O3. Local CA (only if U2 fails). A CA key in the Keychain, with the leaf signed by it, lets the owner trust a root with the switch that Apple documents. Yes or no?

O4. Origins to accept on the WebSocket and POSTs (decision 5). Option A: exactly `https://<address>:<port>` as the host advertises it. Option B: also `localhost` on the same port. Both work with the origin check; the choice decides which browsers reach a host by name.

O5. Who reissues the certificate when the route address changes (U5 and the reissue)? Option A: at pairing-window open, if the address differs. Option B: at startup only. Both work; A leaves the trust step to the owner each time.

## Queries and fetches

Queries (WebSearch, standard mode unless noted):
- Q1 "iOS Safari proceed past self-signed certificate warning IP address no subjectAltName"
- Q2 "Apple requirements for trusted certificates in iOS 13 and macOS 10.15 subjectAltName 825 days"
- Q3 "WebKit WebSocket wss self-signed certificate exception not applied bug"
- Q4 "RFC 6455 Sec-WebSocket-Protocol server selects subprotocol browser fails handshake if not echoed"
- Q5 "RFC 6265 cookies do not provide isolation by port"
- Q6 "browser maximum simultaneous HTTP/1.1 connections per host Chrome Safari 6"
- Q7 "MDN crypto.randomUUID secure context availability"
- Q8 "SameSite=Strict cookie sent with WebSocket handshake same-origin wss Safari Chrome Firefox"
- Q9 "Secure cookie over HTTPS IP address host allowed Set-Cookie Secure self-signed"
- Q10 "WebKit bug SameSite cookie not sent WebSocket handshake Safari"
- Q11 "Safari remembers accepted self-signed certificate exception Show Details visit website how long persists iOS"
- Q12 "iOS Safari open https://Mac.local hostname Bonjour .local resolve certificate warning"
- Q13 "Chrome self-signed certificate proceed secure context isSecureContext crypto.subtle works after interstitial" (no direct answer; not cited)
- Q14 "iOS install self-signed server certificate profile not listed Certificate Trust Settings leaf certificate trust"
- Q15 "MDN History replaceState() modifies the current history entry instead of creating a new one"

Fetches (WebFetch): F1 support.apple.com/103769; F2 bugs.webkit.org/show_bug.cgi?id=41419; F3 rfc-editor.org/rfc/rfc6265#section-8.5; F4 rfc-editor.org/rfc/rfc6455#section-4.1; F5 developer.mozilla.org Crypto/randomUUID; F6 w3.org/TR/secure-contexts; F7 websockets.spec.whatwg.org; F8 developer.apple.com/forums/thread/70139 (IP SAN, DTS, lead only); F9 cheatsheetseries.owasp.org WebSocket_Security; F10 developer.apple.com/forums/thread/104488; F11 docs.rs/rcgen KeyPair; F12 support.apple.com/102390; F13 html.spec.whatwg.org nav-history-apis (the replaceState text was past the first 100k characters and was not read; Q15 stands in).
