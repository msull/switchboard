# Facts for issue #230: a web control surface on the remote port

Worktree `/Users/sbgragent/.dispatch/worktrees/da9387bb` at `7fe8949`. No spike: every runnable question is settled by a source or the tree, and the rest need the owner's phone.

## Settled

**The host's certificate has no SAN, no EKU and a validity to the year 4096; Apple's TLS rules require a SAN naming the server, `id-kp-serverAuth`, and 825 days or fewer, and "connections to TLS servers violating these requirements will fail".**
Tree: `src/adapters/remote.rs:102-110` (`CertificateParams::new(Vec::<String>::new())`); rcgen 0.14.10 defaults `not_before` 1975, `not_after` 4096 (`~/.cargo/registry/src/*/rcgen-0.14.10/src/certificate.rs:83-85`). Apple: https://support.apple.com/en-us/103769. The vendor tracker that agrees: ThinLinc bug 7401 (iOS 13 Safari: HTTPS page loaded, the WebSocket failed with `-9807` until a SAN was added; closed FIXED 2019-10-23) https://bugzilla.cendio.com/show_bug.cgi?id=7401. WebKit bug 158345 (2018 comment: Chrome and Safari refuse a self-signed server certificate without a SAN) https://bugs.webkit.org/show_bug.cgi?id=158345.
Queries: "Apple TLS server certificate requirements subject alternative name 398 days self-signed Safari proceed anyway"; "iOS Safari self-signed certificate wss WebSocket fails after accepting certificate warning".

**Reissuing the certificate from the same key does not break any paired native device: both clients pin the SPKI hash and ignore names, dates and chain; rcgen 0.14 takes `SanType::IpAddress` and `SanType::DnsName`.**
`src/adapters/remote.rs:166-200` (`Pin::verify_server_cert` compares only the SPKI hash); `client/Sources/SwitchboardTransport/Pinning.swift:5-29`; `docs/remote-client.md` §5 "a certificate reissued from the same key still verifies"; `~/.cargo/registry/src/*/rcgen-0.14.10/src/lib.rs:313`.

**Chrome applies the owner's certificate click-through to `wss://` on the same host; Firefox applies a stored exception for `https://host:port` to `wss://` on that host and port, and shows no dialog for a WebSocket by itself.**
Chromium bug 364361, landed as r275066 (2014-06-05): https://codereview.chromium.org/304093003. Mozilla bug 1187666 (WONTFIX, staff: "add the exception by going to https://host:port first", reporter confirmed): https://bugzilla.mozilla.org/show_bug.cgi?id=1187666.
Queries: "Chrome WebSocket wss self-signed certificate proceed override applies to WebSocket after clicking through interstitial chromium bug"; "Firefox wss WebSocket self-signed certificate exception added for https host works bugzilla".

**A browser that sent `Sec-WebSocket-Protocol` fails the connection when the 101 does not echo one of the offered values.**
WHATWG WebSockets §2.2 step 11.2: https://websockets.spec.whatwg.org/ ("if protocols is not the empty list and extracting header list values ... results in null, failure, or the empty byte sequence, then fail the WebSocket connection"); Chromium's handshake helper enforces the same (search result, cobalt mirror of `web_socket_handshake_helper.cc`). `respond_upgrade` echoes nothing today: `src/adapters/remote.rs:519-526`.
Query: "WebSocket Sent non-empty 'Sec-WebSocket-Protocol' header but no response was received RFC 6455 server must echo subprotocol".

**The browser WebSocket handshake carries cookies (credentials mode `include`); a `Secure` cookie is sent on what the user agent deems secure, typically TLS, with no rule about the host being an IP; a cookie without `Domain` is host-only on the request host, IP or not; `SameSite=Strict` is sent on same-site requests.**
WHATWG WebSockets §2.2 step 2; RFC 6265bis §4.1.2.5, §4.1.2.7, §5.1.3, §5.7: https://datatracker.ietf.org/doc/html/draft-ietf-httpbis-rfc6265bis. An `https` origin is "potentially trustworthy" by scheme alone, host unchecked: https://w3c.github.io/webappsec-secure-contexts/ §3.1 step 3. `history.replaceState` needs no secure context, only a same-origin URL: https://developer.mozilla.org/en-US/docs/Web/API/History/replaceState.
Safari caveat, vendor tracker: Strict cookies were missing from WebSocket handshakes in tabs restored without a top-level navigation (Safari 14 and 15, reported fixed in 15.1): https://bugs.webkit.org/show_bug.cgi?id=232666, https://bugs.webkit.org/show_bug.cgi?id=200345. A cookie set on the fetch reply to `/pair`, not on a 101, avoids the Safari `HttpOnly`-on-101 report (https://developer.apple.com/forums/thread/104488).
Queries: "Secure cookie IP address origin self-signed certificate browser secure context WebSocket sends cookie"; "SameSite=Strict cookie not sent WebSocket handshake Chrome Firefox Safari bug".

**CSP `connect-src 'self'` matches a same-host `wss://` in Chrome (since 2018), Firefox, and WebKit since r292266 (2022-04-02, bug 235873, duplicate 201591); older Safari needs the scheme or origin listed.**
https://bugs.webkit.org/show_bug.cgi?id=235873; https://bugs.webkit.org/show_bug.cgi?id=201591; MDN's note: https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Content-Security-Policy/connect-src.
Query: "Content-Security-Policy connect-src 'self' websocket wss same origin CSP3 Safari Firefox".

**Chrome and Firefox open up to 6 HTTP/1.1 connections per host (Chrome: 255 for WebSockets, separate pool); the unauthenticated pool is 4 per listener and the fifth stranger is dropped at accept.**
Chromium `net/socket/client_socket_pool_manager.cc` (`g_max_sockets_per_group = {6, 255}`); Firefox `network.http.max-persistent-connections-per-server` default 6 (`modules/libpref/init/all.js`). Tree: `src/adapters/remote.rs:567`, `699-706`. Safari's figure (6) comes only from a vendor-neutral table (DiffusionData docs), a lead.
Query: "browser maximum connections per host HTTP/1.1 Safari Chrome Firefox 6 connections".

**rustls with no `alpn_protocols` ignores the client's ALPN offer and sends no alert, so a browser offering `h2` completes the handshake and speaks HTTP/1.1; RFC 7301 §3.1 makes the server's ALPN response optional and §3.2 reserves `no_application_protocol` for a server that supports ALPN with no overlap.**
`~/.cargo/registry/src/*/rustls-0.23.45/src/server/hs.rs:98-115` (the alert only when `!our_protocols.is_empty()`); `tls_config` sets none: `src/adapters/remote.rs:144-155`; https://www.rfc-editor.org/rfc/rfc7301.
Query: "rustls server no alpn_protocols configured client offers h2 ALPN handshake succeeds HTTP/1.1 fallback RFC 7301".

**The URL form of the invite is 153 bytes, the JSON form 189; `qrcode` 0.14 picks the smallest version at level M, so the URL is version 9 (53 modules, 180 byte capacity) and the JSON version 10: the QR gets smaller, not larger.**
`QrCode::new` → `EcLevel::M` and `encode_auto` (`qrcode-0.14.1/src/lib.rs:75-94`); capacity table https://www.thonky.com/qr-code-tutorial/character-capacities; lengths computed from `CODE_LENGTH` 8 (`src/adapters/remote.rs:321`), a 64-hex key, a 36-char UUID, `192.168.100.100:50000`. Readability of 4 px modules is the owner's observation (below).
Query: "QR code capacity version 7 8 byte mode error correction M 160 characters".

**`read_request` can keep more headers: it already parses every header into `headers` and drops them when the struct is built; the only test of it asserts method, path, `lines`, bearer and body (`a_request_reads_with_its_query_bearer_and_body`), so adding fields breaks nothing.** `src/adapters/remote.rs:444-514`, tests at `1045-1062`.

**`hide_workers` applies to `projects`, `sessions` and `waiting` replies without `scope=all`; a page reading those needs nothing more.** `src/app.rs:2687-2698`, called at `3119-3121`.

**With `listen.lan` off the port binds `127.0.0.1`; `/info` does not carry `listen`, so a page cannot tell, and need not: a phone cannot reach a loopback bind at all.** `src/app.rs:2962-2966`, `remote_info` `3568-3604`.

## Unresolved

**Whether Safari on iOS and iPadOS applies the owner's certificate click-through to `wss://` once the certificate has a SAN.** Tried: WebKit bug 41419 (2010, macOS Safari 5: works after "always trust"); Cockpit's documentation (undated, https://cockpit-project.org/running/safari) says Safari's default WebSocket on iOS, iPadOS and ARM Macs fails with a self-signed certificate "even when manually marked as trusted" and recommends a valid certificate or the experimental NSURLSession WebSocket; ThinLinc bug 7401 says a SAN made it work on iOS 13; Galène issue 18 and a gist (2020) say only a leaf signed by a device-trusted CA worked; an Apple forum thread (iOS 26 beta 3, `wss://192.168.1.81`, "resolved in beta 5") https://developer.apple.com/forums/thread/792842. The sources disagree and none names the certificate's SAN. Queries: the two under the first Settled claim and "Safari iOS 17 OR iOS 18 self-signed certificate WebSocket wss fails Show Details visit website accepted HTTPS works WebSocket does not" (extended). Line the plan builds to: the page must work without the socket, polling `revision` through REST on a timer when `/api/v1/events` fails to open, and the certificate gets a SAN, `serverAuth` EKU and a validity of 825 days or fewer regardless; if the owner's phone opens the socket, the poll is only the fallback.

**Whether a SAN for an IP address survives the Mac changing address.** An `iPAddress` SAN names one address; `local_address()` returns one outward route (`src/adapters/remote.rs:962-971`). No source consulted. Line: the plan either reissues the certificate from the same key when the listen address changes (safe for native pins, above) or puts a DNS name in the SAN and serves the URL under that name; either way the identity's key does not rotate.

**Whether Safari's per-host connection limit is 6.** Only a vendor-neutral table. Line: size the unauthenticated pool for 6 per browser plus the native clients, or count strangers per source address; the pool of 4 is too small for one browser either way.

## For the owner

**Does the iPhone's Camera open `https://<ip>:<port>/?code=...` in Safari from the QR?** Apple's guide says "tap the link to open the content" without naming link types (https://support.apple.com/en-us/HT208843). If yes, scanning is the whole flow; if no, the owner types the URL from the dialog's text, which the plan must show in full either way.

**After the certificate warning on the phone, does `/api/v1/events` open under the SAN'd certificate?** If yes, live updates; if no, the REST poll above carries the page, and the dialog says so.

**Does the one address in the URL route to the phone on the owner's network?** If yes, one URL; if no, the dialog must list every non-loopback interface's address, which the plan can do from `if_addrs` or a `getifaddrs` wrapper, and the QR keeps the first.

**Is a version 9 QR at 4 px per module readable from the Mac's display by the phone?** If yes, `QR_SCALE` stays; if no, raise it (`src/ui/remote.rs:41`); the dialog width 620 holds up to 8 px.
