# Issue #230 facts (browser page on the remote port)

Searches: WebSearch (standard) then WebFetch. Queries are under each claim. No spike: every browser question needs a browser or device, which this environment does not have. Spec and RFC claims were settled by reading the text.

## Settled

1. Chrome 58+ ignores the subject CN and takes identity only from subjectAltName; a CN-only certificate gives NET::ERR_CERT_COMMON_NAME_INVALID.
   - https://textslashplain.com/2017/03/10/chrome-deprecates-subject-cn-matching/ ; https://community.rsa.com/s/article/Google-Chrome-reports-NET-ERR-CERT-COMMON-NAME-INVALID-due-to-missing-Subject-Alternative-Names-when-accessing-the-RSA-Identity-Router-Single-Sign-On-Portal (vendor KB)
   - Query: "Chrome ERR_CERT_COMMON_NAME_INVALID self-signed certificate without subjectAltName Common Name ignored"

2. Apple's requirements for TLS server certificates (iOS 13, macOS 10.15): DNS name in subjectAltName; validity 825 days or fewer (issued after 1 July 2019); EKU with id-kp-serverAuth; RSA ≥ 2048; SHA-2. Violating connections "will fail."
   - https://support.apple.com/en-us/HT210176 (fetched, verbatim)
   - Query: "Apple HT210176 requirements for TLS server certificates subject alternative name 825 days iOS 13"
   - The tree's certificate meets none of them: `src/adapters/remote.rs:102-110` sets no SAN, no EKU, default validity.

3. RFC 6455: a client fails the connection if the server names a subprotocol it did not offer. A missing Sec-WebSocket-Protocol header is not a failure. The server may send the header only with a value chosen from the client's list, which is a token.
   - https://www.rfc-editor.org/rfc/rfc6455 §4.1 item 6, §4.2.2 (fetched)
   - Query: "RFC 6455 WebSocket Sec-WebSocket-Protocol server must echo subprotocol client fail the connection"

4. base64url characters are legal in a subprotocol value (RFC 6455 §4.1 item 10 and §4.2.2 ABNF `token`).
   - https://www.rfc-editor.org/rfc/rfc6455 (fetched)

5. RFC 6455 §10.2: a server that serves only certain origins SHOULD check the Origin header on the handshake and answer 403 otherwise.
   - https://www.rfc-editor.org/rfc/rfc6455.txt §10.2 (fetched, offset 100000)

6. The WebSocket constructor takes only `url` and `protocols`; script cannot set handshake headers.
   - https://websockets.spec.whatwg.org/ (fetched)

7. HttpOnly cookies are omitted from script-visible cookie APIs; Secure cookies go only over a secure channel; cookies do not isolate by port.
   - https://datatracker.ietf.org/doc/html/rfc6265 §4.1.2.6, §4.1.2.5, §8.5 (fetched)

8. Secure contexts: https, wss, file, loopback and localhost origins. Non-loopback IP literals are not listed. The page is silent on user-accepted certificate errors.
   - https://developer.mozilla.org/en-US/docs/Web/Security/Secure_Contexts (fetched)

9. `crypto.randomUUID` is available only in secure contexts; Baseline since March 2022.
   - https://developer.mozilla.org/en-US/docs/Web/API/Crypto/randomUUID (fetched)

10. Tree: `make_identity` (`src/adapters/remote.rs:95-127`) generates a new key on every call (line 101) and takes no key argument. Reissuing over the existing key needs a signature change.

11. Tree: `RemoteRequest` is built only in `read_request` (`src/adapters/remote.rs:506`); no test builds it literally. It is in `adapters/`, outside CLAUDE.md's additive contract.

12. Tree: no `last_message` in `model.rs`, `control/`, or `src/app.rs` (grep). The card excerpt is not a record field the page can read. `ports/transcript.rs` and `ports/events.rs` also match and were not read.

## Unresolved

1. Whether Apple's TLS rules (fact 2) bind a certificate the user trusts by hand. Apple's page is silent.
   - Query: as fact 2.
   - Plan line: issue to all of Apple's rules regardless (SAN, EKU serverAuth, RSA 2048+, ≤ 825 days). This works whether or not they bind manual trust.

2. Whether Safari applies a click-through exception to `wss://`.
   - Queries: "WebSocket wss self-signed certificate exception not applied Safari WebSocket certificate error accepted in page"; "Safari iOS 'visit this website' self-signed certificate exception IP address invalid certificate warning".
   - Lead only: WebKit bug 41419 is NEW (opened 2010-06-30) and is about logging a failed wss connection, not the exception rule. A 2011 webkit-dev post and WebKit bug 158345 (snippet only) say wss still fails after acceptance.
   - Plan line: do not depend on the click-through carrying to wss. A SAN-bearing certificate installed and trusted works in either case.

3. Whether browsers accept an upgrade that offers a subprotocol but gets no echo. The W3C bug 23576 says the RFC is unclear on this.
   - Query: "RFC 6455 WebSocket Sec-WebSocket-Protocol server must echo subprotocol client fail the connection".
   - Plan line: echo the chosen value from the client's list. This is RFC-correct and works either way.

4. Whether Secure and SameSite=Strict cookies work on an IP-literal host, and whether a current Safari `new WebSocket()` sends them.
   - Query: "WebSocket handshake cookies SameSite Strict same-origin new WebSocket sent cookie Safari WebKit bug".
   - WebKit bug 232666 (strict cookies not sent over WebSocket on restored tabs) is RESOLVED, CONFIGURATION CHANGED; fixed version unconfirmed. Apple forum thread 104488 reports the same failure: a lead only.
   - Plan line: verify on a device. If it fails, the WebSocket takes the subprotocol transport, which needs no cookie. Build the subprotocol path for the WebSocket either way.

5. Whether Chrome and Safari allow a click-through of a SAN-less certificate on a bare IP, and what the interstitial shows. Only forum and vendor-support pages were found.
   - Queries: "Chrome interstitial bypass ERR_CERT_COMMON_NAME_INVALID 'Proceed to' unsafe thisisunsafe bypass allowed"; the Safari query above.
   - Plan line: the design must not depend on the click-through. A SAN-bearing certificate trusted on the device works in every case.

6. Whether Firefox requires a SAN. Results were mixed and had no Firefox primary source.
   - Query: "Firefox SEC_ERROR_INVALID_CERT commonName ignored subjectAltName required self-signed certificate Firefox 58".
   - Plan line: a SAN-bearing certificate satisfies every browser, so the plan does not depend on this.

7. Whether iOS Safari opens at most six HTTP/1.1 connections per host, and how long it keeps an idle one. Only a secondary vendor guide gives six; no Safari idle timeout was found.
   - Query: "iOS Safari maximum simultaneous HTTP/1.1 connections per host six connections keep-alive idle timeout".
   - Plan line: tolerate a dropped connection at accept with a retry, and count the page's static files against the unauthenticated pool's budget. This works at any limit.

8. Whether a Chrome click-through leaves the page a secure context. The spec defines https as secure but is silent on certificate errors. Not checked in a browser.
   - Plan line: do not rely on `isSecureContext` after a click-through.

## For the owner (device facts only)

Q1. On the iPhone, does the warning for the current certificate (CN only, no SAN) let you proceed, and do `fetch` and `new WebSocket('wss://…')` then work?
- Yes: the click-through suffices for the first cut.
- No: reissue with SAN, EKU and ≤ 825 days; install and trust it.
- Both: a SAN-bearing certificate installed and trusted. Build that.

Q2. With a SAN-bearing certificate on the LAN IP, does a cookie set by `/pair` reach the WebSocket handshake on iOS Safari, macOS Safari, Chrome and Firefox?
- Yes: the cookie can carry the token to the WebSocket.
- No: the WebSocket takes the token as a subprotocol value.
- Both: the subprotocol transport for the WebSocket.

Q3. Does the `.local` (Bonjour) name resolve from iOS Safari to the host?
- Yes: the URL may use the `.local` name.
- No: the URL uses the LAN IP.
- Both: the certificate's SAN carries both the IP and the `.local` name.

Q4. Does a page load from iOS Safari get its files through without a connection dropped at accept?
- Yes: no change.
- No: the page retries a dropped connection, and the static paths are checked against the unauthenticated pool.
- Both: retry plus the static-path check.

Q5. Does the URL with the code stay out of Safari history and iCloud Tabs once `history.replaceState` runs?
- Yes: the address bar is the only trace.
- No: the owner clears history after pairing. The code is single-use and dies with the window.
- Both: the single-use code.

Q6. Does the device keep the click-through across launches, and does it survive the host's IP changing?
- Yes: accepted once.
- No: the page says so on its next load and the owner accepts again.
- Both: the page's "identity changed" text covers an IP change.
