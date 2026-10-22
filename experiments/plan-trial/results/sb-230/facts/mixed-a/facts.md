# Facts for issue #230: a web control surface on the remote port

Settled from the tree (worktree at "Release 0.4.1"), the standards and vendors' own pages. No spike was run. Every question still open needs a phone, a browser click-through (a person's tap) or the owner's decision. Spec text was fetched on 2026-10-22.

## Settled

### Tree
- `SessionView` has no excerpt field and `last_message` exists nowhere in the wire crate. Evidence: `grep -n "last_message\|excerpt" control/src/lib.rs` returns nothing. The card's line is drawn on the Mac only (`caption_of`, `src/app.rs:4105`).
- No endpoint tells a caller its own device or scope, and there is no all-sessions read. The route table in `serve_remote` (`src/app.rs:3027-3110`) has info, pair, events, waiting, projects, `projects/{id}/sessions`, sessions/..., ops, spaces, sets and dispatch, and nothing like `me` or `whoami`.
- A revoked token and an unknown token get the same reply, and `devices.json` keeps no tombstone. Evidence: `authenticate` `src/app.rs:3127`; `RevokeDevice` deletes the row (`src/core/remote.rs:86`).
- The pairing code is single-use. The window closes on the first pairing (`src/core/remote.rs:65-70`) and after three failures (`:56-63`). So a code left in browser history is dead once the device has paired.
- The code is 8 characters from a 32-letter alphabet (`src/adapters/remote.rs:319-321`). The token is 32 random bytes in base64url with no padding, 43 characters (`:334-340`).
- `read_request` keeps only the `content-length`, `authorization`, `upgrade` and `sec-websocket-key` headers (`src/adapters/remote.rs:444-514`). It drops Cookie, Origin, Host, User-Agent and `Sec-WebSocket-Protocol`.
- The page does not need a User-Agent header to name the device. `/pair` already takes `device_name` from the body (`src/app.rs:3650`), so the page can build the name from `navigator.userAgent` and send it there.
- `PairedDevice` (`src/ports/remote.rs:96-107`) already uses `#[serde(default)]` for a field added later (`last_seen`). `devices.json` is read with plain serde (`src/adapters/remote.rs:80-82`) and has no migrate step, so another defaulted field is additive.
- The certificate has no SAN, no EKU and a `not_after` in the year 4096 (`src/adapters/remote.rs:95-125`: `CertificateParams::new(Vec::<String>::new())`, CN only). It is a P-256 key.
- `tls_config` sets no ALPN (`src/adapters/remote.rs:144-157`), so a browser speaks HTTP/1.1 on this port: one request at a time per connection.
- A `web/` directory at the repo root reaches `include_str!` in every build. `[package]` has no `include` or `exclude` (`Cargo.toml:1-8`; the only `exclude` is the workspace's, `:91`). `scripts/bundle.sh:34` and CI (`.github/workflows/ci.yml:22-25`) build the workspace with cargo. There is a precedent for embedding: `include_bytes!` in `src/ui/theme.rs:203` and `src/main.rs:167`.
- The test client in `tests/remote.rs:245-296` reads only the status line and `Content-Length`. It has to be extended to assert on `Content-Type`, CSP, `Cache-Control` or `Set-Cookie`.
- The QR is made by `qrcode` 0.14.1 at error-correction level M (`QrCode::new` → `EcLevel::M`, `~/.cargo/registry/src/*/qrcode-0.14.1/src/lib.rs:75-76`), drawn at `QR_SCALE = 4` (`src/ui/remote.rs:42`).
- The URL QR is smaller than today's JSON QR. With a 21-character `ip:port`, the JSON payload is 189 bytes and the URL `https://<ip:port>/?code=..&key=<64 hex>&id=<uuid>` is 153 bytes (computed in a Python one-liner). M-level capacity is 1232 bits at v7, 1456 at v8 and 1728 at v9 (`qrcode-0.14.1/src/bits.rs:661-672`). In pure byte mode the JSON needs v9 (1524 bits) and the URL v8 (1236 bits).

### Cookies (RFC 6265 and Fetch)
- Cookies are not isolated by port. https://www.rfc-editor.org/rfc/rfc6265.html §8.5 says "Cookies do not provide isolation by port." Two Switchboard instances on one host share a cookie unless its name differs, for example by carrying the `server_id`.
- `SameSite` ignores the port. Same site is decided per scheme and host, "the port and domain components are ignored" (https://html.spec.whatwg.org/multipage/browsers.html#same-site). A page on another port of the same IP is same-site, so `SameSite=Strict` does not stop it.
- An `HttpOnly` cookie is kept out of script. https://www.rfc-editor.org/rfc/rfc6265.html §4.1.2.6 says it "instructs the user agent to omit the cookie when providing access to cookies via 'non-HTTP' APIs".
- Per the standard, a `Secure` cookie is stored and sent for any `https` URL, whether or not the certificate is valid. Fetch's Cookie and Set-Cookie steps set "isSecure be true if request's current URL's scheme is 'https'" (https://fetch.spec.whatwg.org/#cookie-header). What browsers do on a click-through origin is under Unresolved.

### WebSocket (RFC 6455, WHATWG WebSockets and Fetch)
- The browser sends cookies on the WebSocket handshake. It is created with "credentials mode is 'include'" (https://websockets.spec.whatwg.org/ §2.2 step 2).
- The browser always sends `Origin` on the handshake. Fetch's "append a request `Origin` header" includes it when "request's mode is either 'websocket' or 'webtransport'" (https://fetch.spec.whatwg.org/#origin-header).
- `Origin` is also sent on any POST. The same algorithm appends it "if request's method is neither `GET` nor `HEAD`", and it is not nulled for a same-origin request under the default policy. A same-origin GET without CORS carries no `Origin`. So an Origin check can guard POSTs and the socket, but not GETs.
- RFC 6455 lets the server check `Origin` but does not require it: "If the server does not validate the origin, it will accept connections from anywhere" (https://www.rfc-editor.org/rfc/rfc6455.html §4.2.2).
- A subprotocol that the browser requests and the server does not echo fails the connection. The WHATWG steps cover "a subprotocol requested by the client, but not acknowledged by the server" (https://websockets.spec.whatwg.org/ §2.2 step 11.2). So the subprotocol candidate needs `respond_upgrade` (`src/adapters/remote.rs:519`) to echo it.
- The token can be used as a subprotocol value. Values "MUST be non-empty strings with characters in the range U+0021 to U+007E" and must be tokens (RFC 6455 §4.1 item 10), and base64url (`A-Z a-z 0-9 - _`) contains only token characters.
- A relative URL works for the socket: `new WebSocket` resolves the URL against the page's base and turns `https` into `wss` (https://websockets.spec.whatwg.org/ constructor steps).

### CSP (CSP Level 3)
- `connect-src 'self'` covers `wss://` to the page's own host and port. `'self'` matches when the host is the same, the ports are the same, and "url's scheme is 'https' or 'wss'" (https://w3c.github.io/webappsec-csp/ §6.7.2.8). Older Safari builds are under Unresolved.
- `connect-src` governs WebSocket: "This directive also controls WebSocket connections" (CSP3 §6.1.2).
- `default-src` does not cover `frame-ancestors`, `form-action` or `base-uri`. The fallback list in CSP3 §6.8.3 names only fetch directives (script, style, worker, connect, manifest, object, frame, media, font and img). The other three must be set explicitly.
- `frame-ancestors` works only as a response header, not in `<meta>` (CSP3 §3.3).

### History and Referer (HTML and Referrer Policy)
- `history.replaceState` may change the path, query and fragment of an `https` URL: "Differences in path, query, and fragment are allowed for http: and https: URLs" (https://html.spec.whatwg.org/multipage/nav-history-apis.html, "can have its URL rewritten").
- `replaceState` replaces the active session-history entry. It does not add one: "entryToReplace be activeEntry if historyHandling is 'replace'" … "Set the URL given document to newURL" (https://html.spec.whatwg.org/multipage/browsing-the-web.html#url-and-history-update-steps). The spec does not cover the browser's global history, which is under Unresolved.
- A `Referer` never carries a fragment. "Strip url for use as a referrer" sets the fragment to null (https://w3c.github.io/webappsec-referrer-policy/ §8.4).
- By default a same-origin `Referer` is the full URL, query included (`strict-origin-when-cross-origin`). The referrer comes from the document's URL when the request is made (same spec, §8.3). `Referrer-Policy: no-referrer` sends none.
- On a code in the query versus the fragment: a query reaches the host's request line and, without `no-referrer`, a Referer sent before `replaceState`. A fragment reaches neither.

### Trust and secure contexts
- Every `https` origin is a secure context; the spec never mentions certificate validity. "If origin's scheme is either 'https' or 'wss', return 'Potentially Trustworthy'" (https://w3c.github.io/webappsec-secure-contexts/ §3.1 step 3).
- Chromium will not register a service worker over an invalid certificate (https://codereview.chromium.org/643773004, "Don't register ServiceWorker with an invalid HTTPS certificate"). An offline or home-screen worker is out under option A.
- To trust a certificate on iOS, the owner installs a profile and then turns on full trust in Settings > General > About > Certificate Trust Settings > "Enable full trust for root certificates" (https://support.apple.com/en-us/102390).
- On iOS 13 and macOS 10.15 and later, a trusted TLS server certificate must name the server in the SAN; a name in the CN is not trusted (https://support.apple.com/en-us/103769). Certificates issued after 2019-07-01 must also carry the `serverAuth` EKU and be valid for at most 825 days (same page). The host's certificate meets none of these. So option B (download the certificate and trust it on the device) needs a new certificate with the LAN IP in the SAN, the EKU and a validity of 825 days or less. Changing the certificate does not change the SPKI pin as long as the key stays the same, because the pin hashes the key (`src/adapters/remote.rs:110`).

### Storage and device names
- Safari's tracking prevention (ITP) deletes LocalStorage, IndexedDB, SessionStorage and service workers "after seven days of Safari use without user interaction on the site". Home-screen web apps keep their own count of days (https://webkit.org/blog/10218/full-third-party-cookie-blocking-and-more/). A token in `localStorage` can therefore disappear. The cap is on storage that script can write, which a server-set `HttpOnly` cookie is not.
- A user agent cannot tell an iPad from a Mac: "With the exception of iPad mini, Safari on iPad will now send a user-agent string that is identical to Safari on macOS" (https://webkit.org/blog/9674/new-webkit-features-in-safari-13/). A name like "Safari on iPad" needs another signal or the owner's own words.
- The iOS Camera recognises a QR code, shows a notification, and opens the link when tapped (Apple support "Scan a QR code with your iPhone", https://apple.co/2NAZ0Br, reached through a search result and not fetched directly).

## Unresolved

- **Does Safari let the WebSocket through after a click-through?** WebKit bug 145819 ("Websocket Invalid Certificate Error after accepting self signed certificate", NEW since 2015, last comment 2019; https://bugs.webkit.org/show_bug.cgi?id=145819) says that on Safari 8 `wss` failed after the page's certificate was accepted, and that adding the certificate to the keychain fixed it. Bug 41419 (https://bugs.webkit.org/show_bug.cgi?id=41419) is the same report from 2010. Both are old: one is cross-origin, and WebSocket has since moved to an NSURLSession path (comments in https://bugs.webkit.org/show_bug.cgi?id=158345). Nothing from the vendor says current Safari behaves either way. This is the fact decision 1 turns on, and only a device test settles it.
- **Will Safari and Chrome on iOS/Android show a click-through for a no-SAN certificate on an IP address, and keep and send `Secure; HttpOnly` cookies there?** The standards say yes for cookies (see Settled). No vendor page covers the warning page for no-SAN certificates, and no vendor document was found on cookies over an accepted invalid certificate.
- **Does Chrome extend a page's certificate exception to `wss` on the same host?** The only lead is a 2011 webkit-dev post (https://lists.webkit.org/pipermail/webkit-dev/2011-June/017335.html) saying it does. That is not primary evidence.
- **Can page JS read the TLS certificate or its SPKI?** Nothing in Fetch, HTML or Secure Contexts exposes either, but no source states the negative. The plan should assume the page can compare the URL's `key` only with the `spki_sha256` field of `/info`'s JSON, which an impostor writes itself.
- **Does Safari on older devices honour `connect-src 'self'` for `wss`?** CSP3 settles current spec behaviour, but no WebKit release note was found for when Safari adopted it. Listing `wss://<host>:<port>` explicitly cannot be done in a static header without knowing the address. Test on the target devices.
- **Does the browser's global history (the History menu, iCloud or Chrome sync) record the pairing URL before `replaceState` runs?** No spec covers global history, so neither the query nor the fragment form is known to stay out of it. Because the code is single-use (see Settled), the code it would leak is dead after pairing.
- **Does the Camera, Messages or a link scanner prefetch or upload the URL before the page runs?** No vendor source found.
- **How many connections does one page load hold in the 4-slot unauthenticated pool?** That needs a browser against the port. HTTP/1.1 without ALPN means parallel asset fetches open parallel connections (see Settled), but the count per browser was not measured.
- **Exact QR version and pixel size of the URL QR.** The arithmetic says v8 or smaller against the JSON's v9 in pure byte mode. The crate may mix segment modes and shrink both, so the exact version was not run.

## For the owner

1. **Is click-through enough on your iPhone and iPad?** On each device, with the current certificate (no SAN, P-256, IP address, non-standard port), open the page, accept the warning, and check three things: the page loads, `fetch` to `/api/v1/info` works, and `new WebSocket('/api/v1/events')` opens.
   - If yes on every device: option A. The click-through is the trust step, the paired-devices dialog says so, and the certificate stays as it is.
   - If the socket fails on Safari (bug 145819): either option B, which reissues the certificate with the LAN IP in the SAN, the `serverAuth` EKU and a validity of 825 days or less while keeping the key so the native pins hold, and serves it for install plus Certificate Trust Settings; or the page falls back to polling `/api/v1/*` by revision with no socket.
2. **Cookie or subprotocol for the token?** A cookie is ambient: any page on another port of this IP is same-site and can use it, so every `/api/v1` POST and the socket need an `Origin` check, and a GET cannot get one (Fetch sends no `Origin` on a same-origin GET).
   - If cookie: name it with the `server_id`, mark it `HttpOnly; Secure; SameSite=Strict`, accept it only from a request whose `Origin` matches the host's own, and keep the bearer header for the native clients.
   - If subprotocol, or a header token for `fetch`: the page holds the token in script. In `localStorage` it lasts until ITP's seven days without use; in memory only, the device pairs again on every load.
3. **One QR or two?**
   - If the headset app may be updated with this release: one URL QR, and `PairingInvite.parse` and `switchboard-remote pair` accept the URL as well as the JSON.
   - If an installed headset build that parses only JSON must keep working: "Copy invite" keeps the JSON (the headset pastes it), and only the QR becomes the URL.
4. **Code in the query (`?code=`), as the issue says, or in the fragment (`#code=`)?**
   - If fragment: the code never reaches the host's request line or a Referer, and the page reads `location.hash`.
   - If query: send `Referrer-Policy: no-referrer` and make sure no code path logs the query.
5. **Should the page say "revoked" when it means revoked?**
   - If yes: a durable tombstone of revoked token hashes, which is a new record.
   - If no: the page says "not paired, or revoked; pair again" for every 401.
6. **Should the first cut show the card's one-line excerpt?**
   - If yes: an additive field on `SessionView`. The wire crate, its fixtures and the Swift fixtures change.
   - If no: kind, state and waiting reason only.
7. **Does the page need a "who am I" read for its scope and device name?**
   - If yes: a small authenticated endpoint, which is additive.
   - If no: the page stores the scope from the `/pair` reply. It goes stale when the dialog changes it, and the page learns of the change only from a 403.
8. **What does a stranger with no code and no token see?**
   - If a page: a static "Pair from the Mac" page, with no host name unless `/info` already shows it.
   - If nothing: a 404 for `/` when no pairing window is open.
9. **Should the loopback bind serve the page, and does the web page get its own switch, separate from "Allow devices on the local network"?** Yes or no on each changes `bind_remote`'s path table and the dialog.
10. **Which devices are supported, and can you run question 1 on them?** The name a device pairs under cannot tell an iPad from a Mac by user agent. If the owner wants the name to say which, the page asks the person to type a name.
