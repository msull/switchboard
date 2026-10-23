# Facts for #230: a web control surface on the remote port

Worktree `/Users/sbgragent/.dispatch/worktrees/f96f6569` at 0.4.1; line
numbers are from this tree. No spike was run: every question no source
settles needs a phone, a browser on a device, or the owner's eyes.

## Settled

**A browser cannot set a header on `new WebSocket()`; the handshake
carries cookies.** Constructor takes `(url, protocols)`; the handshake is
a fetch with credentials mode "include"
(https://websockets.spec.whatwg.org/ §2.2 step 2). RFC 6455 §4.1 item 12:
"The request MAY include any other header fields, for example, cookies".
Query: `WHATWG websockets establish a WebSocket connection
Sec-WebSocket-Protocol response missing "fail the WebSocket connection"`.

**A token in `Sec-WebSocket-Protocol` is legal and the host must echo
it.** RFC 6455 §4.1 item 10: elements are U+0021 to U+007E "not including
separator characters"; base64url has none and the token is unpadded
(`src/adapters/remote.rs:334-340`), so no `=`. WHATWG §2.2 step 11.2:
protocols non-empty and the response header "null, failure, or the empty
byte sequence" fails the connection, so `respond_upgrade`
(`src/adapters/remote.rs:519-527`) must echo it; RFC 6455 §4.2.2 lets the
server echo one offered value. Same query.

**`connect-src 'self'` covers same-host `wss://` on an https page, in
the spec and in WebKit since 2022-04.** CSP3 §6.7.2.8 'self': hosts
equal, ports equal or default, "url's scheme is https or wss"
(https://w3c.github.io/webappsec-csp/). WebKit bug 235873 RESOLVED FIXED
2022-04-02 (r292266); bug 201591 (same-host wss refused under 'self') is
its duplicate (https://bugs.webkit.org/show_bug.cgi?id=235873). The
commit predates Safari 16. Query: `Content Security Policy connect-src
'self' WebSocket wss same origin CSP3 Safari WebKit bug`.

**A `Secure; SameSite=Strict` host-only cookie on an IP origin reaches
same-origin `fetch` and the WebSocket.** RFC 6265bis §5.7 step 10: no
`Domain` means host-only; §5.1.3 domain matching is for host names, so
an IP cookie is exact
(https://httpwg.org/http-extensions/draft-ietf-httpbis-rfc6265bis.html).
HTML "obtain a site": no registrable domain gives (scheme, host), same
site when hosts are equal
(https://html.spec.whatwg.org/multipage/browsers.html#same-site). The
page document needs no cookie, so how the camera-app navigation is
classed does not matter. "Secure" is user-agent defined (§4.1.2.5); see
Unresolved for the bypassed-certificate case.

**rustls with an empty `alpn_protocols` does no ALPN; HTTP/2 needs
ALPN.** "If empty we don't do ALPN at all"
(https://docs.rs/rustls/latest/rustls/server/struct.ServerConfig.html);
`tls_config` sets none (`src/adapters/remote.rs:144-158`). RFC 7301 §3.1
makes the server's selection optional; the fatal alert is only for a
server that supports ALPN and none of the offered protocols (§3.2). So a
browser speaks HTTP/1.1 to the hand-rolled reader. Query: `rustls server
no ALPN configured browser offers h2 http/1.1 falls back RFC 7301`.

**iOS and macOS trust no TLS server certificate without a SAN, with
validity over 825 days, or without EKU serverAuth.** "All TLS server
certificates must comply"; CommonName "no longer trusted"; 825 days and
`id-kp-serverAuth` for certificates issued after 2019-07-01
(https://support.apple.com/103769). The host's certificate has no SAN
and rcgen's default `not_after` (`src/adapters/remote.rs:95-128`, line
102), so it fails all three; a certificate reissued from the same key
keeps every pin (`docs/remote-client.md:308`). Query: `Apple
requirements for trusted certificates iOS 13 subject alternative name
825 days TLS server`.

**iOS lists a certificate under Certificate Trust Settings only if it is
CA-shaped.** Apple DTS, Nov 2022: it "needs [Basic Constraints] in order
to be considered a CA cert, which is what that UI is all about"; "Don't
create a self-signed certificate for the server itself", issue the leaf
from a CA (https://developer.apple.com/forums/thread/720477). So
"offer the certificate for download" means a CA certificate plus a leaf
with an IP SAN, not today's leaf. Query: `iOS install self-signed
certificate profile "Certificate Trust Settings" requires root CA
basicConstraints`.

**Chrome remembers a certificate-error bypass for one week, on disk,
keyed by host and chain.** `kCertErrorBypassExpirationInSeconds =
604800`, stored in content settings `SSL_CERT_DECISIONS` under
`https://<host>` with the error code and chain SHA-256
(https://chromium.googlesource.com/chromium/src/+/refs/heads/main/components/security_interstitials/content/stateful_ssl_host_state_delegate.cc).
A per-host bypass covers Chrome's `wss://` to the same host. Query:
`Chrome Android proceed to unsafe site certificate decision remembered
one week ChromeSSLHostStateDelegate`.

**iPad Safari presents a Mac user agent by default** (`Macintosh`, no
`iPad`, since iPadOS 13), so a user-agent name says "Safari on Mac" for
an iPad (https://developer.apple.com/forums/thread/119186). Query:
`iPadOS Safari user agent string Macintosh desktop by default`.

**The URL is a smaller QR than today's JSON.** `QrCode::new` is
`EcLevel::M` at the smallest fitting version
(`~/.cargo/registry/src/*/qrcode-0.14.1/src/lib.rs:75-76`; capacities
`bits.rs:661`). Measured in a scratch crate on 0.14.1 with a 64-hex key
and a 36-char id: JSON 189 bytes = version 10 (57 modules); URL with hex
key 153 bytes = version 9 (53); URL with a 43-char base64url key 132
bytes = version 8 (49). `QR_SCALE = 4` (`src/ui/remote.rs:42`) either way.

**A full unauthenticated pool drops the TCP stream; 503 is for a full
frame channel.** `src/adapters/remote.rs:699-705` and 579, 821. Rate
windows cover `/pair` and `/info` only (586-587). The test at
`tests/remote.rs:3226` asserts the drop for a fifth stranger and is
unchanged by page files on the pool; a browser's parallel page-load
connections meet the drop (see Unresolved).

**"Revoked" and "never paired" are the same on the wire.** `authenticate`
answers 401 for both (`src/app.rs:3133`, `3146`). The page infers
"revoked" from a 401 on a token that once answered 200.

**The invite's shape is asserted in two places,** `tests/ui.rs:7474-7489`
(clipboard JSON: `code`, `spki_sha256`) and
`client/Tests/SwitchboardProtocolTests/FixturesTests.swift:158-166`
(inline JSON through `PairingInvite.parse`). No shared invite fixture.

**The token is forbidden in a URL** (`docs/remote-client.md:312`). A URL
fragment is never sent to the server (RFC 3986 §3.5); `refuse` logs path
without query (`src/app.rs:3754-3766`).

**`local_address` is the default route's source address**
(`src/adapters/remote.rs:962-973`, a UDP connect to `192.0.2.1`). A VPN
that takes the default route changes it; one routing only its prefix
does not.

## Unresolved

**Does Safari on iOS carry an https certificate bypass to `wss://` on the
same origin?** No vendor statement. Against: Apple Developer Forums, iOS
26 beta 3, certificate accepted, WebSocket failed with "The certificate
for this server is invalid" (https://developer.apple.com/forums/thread/792842;
reporter says beta 5 resolved it); WebKit bug 158345 (NEW, last 2021):
"you need to do a normal page load first", WebSockets ran on CFStream
outside the Network process (https://bugs.webkit.org/show_bug.cgi?id=158345);
two blogs: "accepted the cert for HTTPS, but not WSS"
(https://www.hotelexistence.ca/ios-safaris-websockets-implementation-doesnt-work-with-self-signed-certs/,
https://gist.github.com/apankrat/612dde3d7f01c4713c9579b0a94d9547, which
works only with a trusted CA and SANs). Queries: `iOS Safari self-signed
certificate "visit this website" WebSocket wss fails after accepting
certificate warning`; `bugs.webkit.org WebSocket wss self-signed
certificate iOS Safari fails "certificate for this server is invalid"`.
Line: the page must work without the socket, polling `/api/v1/waiting`
and the collections by revision over `fetch` when the socket fails or
closes; the socket is an upgrade when it connects. If the owner's phone
connects, the poll becomes a slow fallback.

**Does an iOS Safari bypass persist across tabs and relaunches?** No
source states a duration; the gist calls acceptance "temporary". Query:
`iOS Safari self-signed certificate warning exception persist across
tabs relaunch "This Connection Is Not Private"`. Line: the page must
lose nothing when the warning returns, and the dialog says it may
return. If it persists, nothing changes.

**Are `Secure` cookies stored and sent on a bypassed-certificate
origin?** "Secure" is user-agent defined; no vendor statement found.
Query: `Chrome cookies Secure attribute set over https page with
certificate error bypassed interstitial`. Line: the end-to-end test uses
the host's own client and proves the host side only; the page must show
"pair again" when its first authenticated call after `/pair` is 401, so
a dropped cookie is a visible state. If cookies work, nothing is added.

**Does a browser retry a page-load `GET` whose connection the host
dropped under the four-connection cap?** Chromium reviews show retries
only for a closed reused socket (https://codereview.chromium.org/403393003,
https://codereview.chromium.org/174287); nothing current for Safari.
Query: `Chrome retry GET request connection closed before response
reused socket ERR_CONNECTION_CLOSED`. Line: the unauthenticated page
load needs fewer parallel connections than `MAX_UNAUTHENTICATED` (4)
less one for `/info` and `/pair`: one HTML plus at most one CSS and one
JS, and the plan states the count. If browsers retry, the count holds.

**Does `history.replaceState` keep the code out of history and sync?**
MDN-derived text says it does not prevent a global history entry; no
vendor statement. Query: `history.replaceState does Chrome browser
history record original URL before script runs`. Line: the code is
single-use and the window closes on first success
(`docs/remote-client.md:310`), so a code in history after pairing is
spent; the plan says so, and closes the window on the page's failure
report or the five minutes. A fragment keeps it out of server logs, not
history.

## For the owner

**Does the iPhone Camera open `https://<ip>:<port>/?code=…` in Safari
with the query intact, and Chrome on Android?** No Apple document covers
IP-literal URLs (query: `iPhone Camera app scan QR code opens Safari URL
with query string`). If yes, the URL is the only invite; if not, the
text beside the QR is the URL to paste. Both work with the URL as the
single payload.

**After accepting the warning on the owner's phone, does
`new WebSocket("wss://<ip>:<port>/api/v1/events")` connect?** Decides
whether the poll is the fallback or the path; both work with the poll
built.

**Does the bypass survive closing the tab and relaunching Safari?** Both
branches work with the warning sentence in the dialog.

**Is a 49-module QR at 4 px per module readable at arm's length?**
Today's is 57 modules at the same scale, so no harder; if not,
`QR_SCALE` rises.

**Does the owner's Mac have a VPN or Tailscale taking the default
route?** If so the QR's address is the tunnel's and unreachable from the
phone; the plan may list every non-loopback interface and let the owner
pick. If not, the one address is right.
