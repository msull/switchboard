# Facts for #230 (browser page on the remote port)

Worktree at 7fe8949. No spike: the mechanics are settled by standards, the rest needs a device or an owner. Nothing committed; git status empty.

**Queries** (WebSearch, standard mode): Q1 "iOS Safari self-signed certificate … IP address without subjectAltName"; Q2 "Chrome removed common name fallback certificate subjectAltName required ERR_CERT_COMMON_NAME_INVALID"; Q3 "WebSocket Sec-WebSocket-Protocol token authentication browser server must echo subprotocol RFC 6455 section 4.2.2"; Q4 "Apple HT210176 requirements for trusted certificates iOS 13 … 825 days"; Q5 "W3C secure contexts potentially trustworthy origin https certificate error isSecureContext"; Q6 "cross-site WebSocket hijacking Origin header check … cookie SameSite"; Q7 "Safari on iPhone certificate warning "Show Details" "visit this website" Apple support self-signed"; Q8 "Chrome ERR_CERT_COMMON_NAME_INVALID "Proceed to" unsafe bypass allowed name mismatch not HSTS"; Q9 "Chromium commonName fallback removal Chrome 58 subjectAltName RFC 2818 chromestatus"; Q10 "Safari iOS maximum simultaneous HTTP/1.1 connections per host … idle timeout" (nothing usable).

## Settled

- `new WebSocket(url, protocols)` takes no headers. WHATWG, https://websockets.spec.whatwg.org/ (fetched); MDN WebSocket (fetched). Q3, Q6.
- The server may send `Sec-WebSocket-Protocol` only with a value derived from the client's offer, and must not send it otherwise. If protocols were offered and none comes back, the browser fails the connection. RFC 6455 §4.2.2, https://www.rfc-editor.org/rfc/rfc6455 (fetched); WHATWG (fetched). So a token carrier needs the server to echo one offered value.
- Subprotocol values are `1#token`, U+0021–U+007E, unique; a violation throws SyntaxError. RFC 6455 §4.1 (fetched); MDN (fetched).
- The server "SHOULD verify the Origin field" and "SHOULD respond … 403" if unexpected. RFC 6455 §10.2 (fetched, offset 100000). The allowlist is the owner's.
- Cookies "do not provide isolation by port". RFC 6265 §8.5, https://www.rfc-editor.org/rfc/rfc6265 (fetched). `__Host-` does not change this.
- `Secure` limits a cookie to secure channels (RFC 6265 §4.1.2.5, fetched). `__Host-` needs `Secure`, a secure page, no `Domain`, `Path=/`. MDN Cookies guide, https://developer.mozilla.org/en-US/docs/Web/HTTP/Guides/Cookies (fetched). Vendor docs, no contradiction.
- `SameSite=Strict` cookies are sent "only in response to requests originating from the cookie's origin site." Same MDN page. WebSocket is not named (Unresolved).
- An `https` or `wss` origin is potentially trustworthy by scheme alone; certificate validity is not a condition. W3C Secure Contexts §3.1 step 3, https://www.w3.org/TR/secure-contexts/ (fetched). The search's claim that a certificate-error page "is generally not a secure context" is unsourced; do not build on it. Q5.
- `crypto.randomUUID()` is "available only in secure contexts." MDN, https://developer.mozilla.org/en-US/docs/Web/API/Crypto/randomUUID (fetched).
- `localStorage` is per origin and protocol; scripts of that origin read it. MDN, https://developer.mozilla.org/en-US/docs/Web/API/Window/localStorage (fetched).
- `replaceState` "modifies the current history entry", changing the URL without a load. MDN, https://developer.mozilla.org/en-US/docs/Web/API/History/replaceState (fetched).
- Apple's TLS server rules, certificates issued after 1 July 2019: DNS name in the SAN (CN not trusted), validity ≤825 days, EKU `serverAuth`, RSA ≥2048. HT210176, https://support.apple.com/en-us/HT210176 (fetched). It does not say whether user-trusted certificates are exempt. The repo certificate has no SAN, a year-4096 `not_after` (`src/adapters/remote.rs:95-127`), and P-256 keys; the page lists RSA sizes only, so ECDSA is unaddressed. Q4.
- Apple DTS: a CA "can issue a certificate for an IP address via the Subject Alternate Name extension"; install a custom CA's root on client devices. Apple Developer Forums 70139, https://developer.apple.com/forums/thread/70139 (fetched). No contradiction.
- Chrome's vendor intent proposes removing commonName fallback, "in effect, requiring a subjectAltName." https://groups.google.com/a/chromium.org/g/security-dev/c/IGT2fLJrAeo/m/csf_1Rh1AwAJ (fetched). It gives no version. Q9.
- Pairing codes are single-use: the window closes on `Paired` (`src/core/remote.rs:65-66`); `remote_pair` checks the code only while open (`src/app.rs:3646`).
- Tree: no `last_message` in `control/src/lib.rs`, `src/app.rs`, `client/Sources`. The card excerpt is not on the wire.
- Tree: no `Cookie`, `Origin`, `User-Agent` or `Sec-WebSocket-Protocol` handling in `src/adapters/remote.rs`.
- Tree: "never in a URL" is `docs/remote-client.md:312`, the project's spec. The issue's "the spec forbids the token in a URL" means that line, not an external standard.
- Tree: `make_identity` generates a new key on every call (`src/adapters/remote.rs:101`); the key PEM goes to the Keychain only (`:92-93`, `:126`). `KeyPair::from_pem` and `spki_sha256_of_key` exist (`:138-140`). A reissue over the stored key is the same `self_signed(&key)` call (`:109`) with a loaded key, and `spki_sha256` is unchanged.
- Tree: no test builds `RemoteRequest` literally (`tests/remote.rs`). `RemoteReply` literals in `src/app.rs` (about 20) would need touching for a new field.
- Tree: `PairingInvite.addresses` is a list (`KnownServer.swift:14`, `ConnectionManager.swift:483`, `switchboard-remote/main.swift:120`). A URL holds one host:port.
- Tree: the watch-cap loop evicts the oldest `last_read` across all watches (`src/app.rs:1789-1795`); the shown lines do not exclude subscribed ones.
- Tree: the pre-pairing rule is `docs/remote-client.md:306`; static paths amend it.

## Unresolved

- **Does iOS Safari allow a click-through on a no-SAN certificate for a bare IP?** Q1, Q7. Apple's Mac help says cancel and verify the address; a Peplink forum post says iOS allows "visit this website" (lead); a Kurento doc says iOS rejects untrusted self-signed outright (secondary, conflicting). Build line: produce a SAN-bearing certificate with an IP SAN and offer it for install; that works either way.
- **Chrome's name-mismatch bypass, and whether HSTS blocks it.** Q8: consumer guides only. Chrome's CN removal version: Q2 and Q9 give a Microsoft blog and a third-party report (Chrome 58). Build line: SAN required whatever the version.
- **Does a click-through cover `wss://` and page `fetch`, per browser?** No source. Device check.
- **Does a bypassed https page count as secure (`randomUUID`, `Secure` cookies)?** Spec says scheme only; browser behaviour unsourced. Build line: make op ids without `randomUUID`; uniqueness is enough.
- **Does Safari send `SameSite=Strict` cookies on a same-origin WebSocket handshake?** Q6 found no per-browser source. Build line: the subprotocol carrier works regardless; choose cookies only after a device check.
- **Does a click-through survive relaunch or an IP change?** No source. Device run.
- **Does a page load exhaust the pool of 4 on iOS, and how many connections does Safari open?** Q10 found nothing. Device run. Build line: the page reconnects on a dropped socket.
- **Does Apple's SAN / 825-day rule bind a user-trusted certificate?** HT210176 is silent. Build line: issue within the limits, so the answer does not matter.
- **Does `.local` resolve from iOS Safari?** Not searched. Build line: the URL uses the route-chosen IP; `.local` optional.
- **Is the code in history before `replaceState` runs?** No source. Single-use code (settled) limits the damage after pairing.
- **Does a headless Chrome run settle the Chrome-only questions?** Not run here. The plan must not depend on a Chrome-only answer.
- **Does `HttpOnly` hide the cookie from script?** Not fetched (RFC 6265 §5.2.6). Check before citing.

## For the owner

1. **Trust step (decision 1).** The click-through, said in the dialog, or a certificate download to install first. Both need a SAN-bearing certificate within the Apple limits; build that first.
2. **Reissue over the stored key.** Keeps `spki_sha256`, so native pins hold. Who reissues on an address change: if you reissue, reuse the key; if not, the address change needs a new pin. The same certificate code serves both.
3. **Token carrier (decision 2).** The subprotocol route's mechanics are settled and work for WebSocket and fetch. The cookie route rests on the unsettled WebSocket SameSite question. `localStorage` is readable by any script of the origin; with no inline script under the CSP, only the page's own script runs. Subprotocol unless a device check clears cookies.
4. **Origin check.** Required by RFC 6455 §10.2; the allowlist (IP:port, `.local`, loopback) is yours.
5. **QR (decision 3).** One URL works for browser and native text paths if `PairingInvite.parse` accepts URL and JSON for now.
6. **Static paths, cache, rate window, pool (decision 4).** Yours. Pool and rate windows are `src/adapters/remote.rs:567-587`; static paths have no window.
7. **Revoked versus never paired.** Both refusals share `unauthorized` (`src/app.rs:3125-3153`, from the notes, not re-read). A new code is additive.
8. **Scope after pairing.** No endpoint returns it; the header goes stale without one.
9. **Excerpt.** Add a field to `SessionView` (additive, fixtures both sides), or fetch `last-response` per session. Watching every listed session can evict the headset's watches (`src/app.rs:1789-1795`); the runtime effect is unmeasured.
10. **Acceptance run.** The iOS questions above need a phone.
