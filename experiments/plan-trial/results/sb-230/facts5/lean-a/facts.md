# Facts for #230 (web control surface)

No spike was run. The decisive question (Safari's `wss://` after a click-through) is settled in shape by sources, and its current behaviour needs an iPhone, so it goes to the owner. Questions about the tree are answered in the investigation and not repeated.

## Settled

1. **A browser `WebSocket` cannot set headers; its only handshake inputs are the URL and subprotocol names.** WHATWG: `new WebSocket(url, protocols)`, no options (https://websockets.spec.whatwg.org/#the-websocket-interface). MDN agrees (https://developer.mozilla.org/en-US/docs/Web/API/WebSocket/WebSocket). Query: "WebSocket constructor cannot set custom headers Authorization browser WHATWG Sec-WebSocket-Protocol token".

2. **A subprotocol-carried token needs the 101 to echo an offered protocol.** WHATWG: a non-empty `protocols` with no acknowledging `Sec-WebSocket-Protocol` fails the connection. RFC 6455 §4.2.2: the value must derive from the client's offer (https://www.rfc-editor.org/rfc/rfc6455#section-4.1). `respond_upgrade` writes none today and `read_request` drops the header (investigation §1).

3. **A script sees nothing of a refused upgrade.** WHATWG: user agents "must not convey any failure information to scripts"; `close` has code 1006, `wasClean=false`. MDN: no documented `error` properties. So "revoked" must be learned from a `fetch` 401, and `unauthorized` needs a distinct code for revoked (`src/app.rs:3129-3148`). Query: "WebSocket error event no information on failed handshake status code browser security".

4. **Browsers send `Origin` on every WebSocket handshake; checking it is the server's job.** RFC 6455 §4.1 ("MUST include ... if the request is coming from a browser client"), §4.2.2 ("If the server does not validate the origin, it will accept connections from anywhere"; answer 403). `read_request` drops `Origin` today (investigation §1).

5. **Cookies are not isolated by port.** RFC 6265bis §1: "cookies for a given host are shared across all the ports on that host"; §4.1.3.2: "Ports are the only piece of the origin model that `__Host-` cookies continue to ignore" (https://httpwg.org/http-extensions/draft-ietf-httpbis-rfc6265bis.html). The draft defers "same-site" to another spec, so `Strict` across ports is not stated there. GHSA-2vfc-3h43-vghh (https://github.com/Syslifters/sysreptor/security/advisories/GHSA-2vfc-3h43-vghh) reports `SameSite=Strict` cookies sent on a same-site WebSocket handshake with no Origin check. Consequence: a cookie token needs the `Origin` check from 4, and a custom header on `fetch` (forces a preflight the host does not answer). `SameSite` alone is not the defence. Query: "SameSite=Strict cookie sent with WebSocket handshake same-site cross-port Origin header check cross-site WebSocket hijacking".

6. **"No token in a URL" is this repo's rule, not a standard's.** `docs/remote-client.md:312` and `:318`. RFC 6455 does not forbid it; OpenReplay and an Auth0 thread advise against it for log leakage (leads, not standards). The plan keeps the repo rule.

7. **Chrome ignores the common name; a certificate with no SAN fails hostname matching for a bare IP.** Since Chrome 58 (2017), `ERR_CERT_COMMON_NAME_INVALID`: "does not contain a Subject Alternative Name extension containing a domain name or IP address"; Advanced > "Proceed (unsafe)" still offered. Evidence is user and vendor reports (Graylog, NethServer, Atlassian, Netgate, Metric Insights); no source contradicts it, and the tree confirms the empty SAN list (`src/adapters/remote.rs:102`). Query: "Chrome self-signed certificate IP address no subjectAltName ERR_CERT_COMMON_NAME_INVALID proceed unsafe". The plan adds the listen address as an IP SAN. A certificate regenerated on the same key keeps `spki_sha256`.

8. **Safari's `wss://` does not honour a certificate click-through; this is the plan's main constraint.** WebKit bug 158345, open since 2016, last update July 2021 still NEW: WebSockets run on CFStream with no challenge callback, and a fix needs a rewrite on NSURLSession (https://bugs.webkit.org/show_bug.cgi?id=158345; origin report bug 41419). Cockpit documents current behaviour: Safari's default WebSocket on iOS, iPadOS and ARM Macs does not support `wss` with self-signed certificates "even when they are manually marked as trusted"; the options are Apple's experimental "NSURLSession WebSocket" flag or a valid certificate (https://cockpit-project.org/running/safari, undated). A vendor article tested on Safari 15.6.1 says Keychain "Always trust" makes `wss` work. An iOS 26 beta 3 forum post reports the same error (seen in results, not opened). Queries: "Safari iOS self-signed certificate warning \"visit this website\" WebSocket wss fails after accepting certificate"; "WebKit Safari WebSocket self-signed certificate exception not honored wss NSURLSession WebSocket fixed enabled by default Safari 15". Consequence, whatever the owner reports: REST polling is the floor; the socket is an upgrade. Trusting the certificate on the device (the issue's download route) is what gives Safari a socket.

9. **CSP `connect-src 'self'` has not matched `wss:` in Safari.** WebKit bug 201591 (2019; a 2021 comment says unfixed in Safari 14); MDN notes `'self'` does not cover WebSocket schemes in all browsers; W3C CSP issue #7 is the spec history. Newer Safari is untested. Use `connect-src 'self' wss:` (the host name is not known at compile time). Query: "CSP connect-src 'self' websocket ws wss same origin Safari not matching 'self' bug CSP3".

10. **iPadOS Safari sends a Mac user agent by default.** Apple developer forum thread 119186 and MacStories: since iPadOS 13 only Slide Over keeps the Mobile token; `navigator.maxTouchPoints` is 5 on iPad, 0 on a Mac. Data from 2019-2020. A name from the user agent cannot say "iPad"; the page can send `maxTouchPoints` with the name, or the name reads "Safari on Mac". Query: "iPadOS Safari user agent Macintosh desktop-class browsing iPad detection".

11. **Chrome's Local Network Access prompt does not cover a page fetching its own LAN address, nor WebSockets.** The first milestone covers "public network to a local network or loopback"; local-to-local is planned later; WebSocket is a listed known gap; prompt "launching in Chrome 142" (https://developer.chrome.com/blog/local-network-access). Safari's rule was not found. Query: "Chrome Local Network Access permission prompt fetch private IP address from page served from local network same address".

## Unresolved

**U1. A `Secure; HttpOnly` cookie set from a click-through IP origin: stored and sent on `fetch`?** Query: "Chrome certificate exception \"Proceed to\" self-signed cookies Secure attribute stored IP address https secure context"; results covered the warning only. Build so both work: the host accepts the cookie and the bearer header, the page feature-tests. If favourable, the cookie is the default and the token stays out of script; if not, the page keeps the token in `localStorage` and sends the bearer, and the paired-devices dialog says so. The plan never waits on this.

**U2. Chrome's click-through covering `wss://` and persisting across restarts.** Only a 2011 WebKit list thread says Chrome treats acceptance as valid for both. Build to the polling floor from 8; if favourable, Chrome gets live events.

**U3. Any browser pinning an SPKI hash for page `fetch`/`WebSocket`.** No source found providing it, and no fetched source says none does. Plan on none: the `key` check compares against `/info`, which an impostor also serves, so word it as a guard against a stale or mistyped address, not authentication. If one exists, nothing is added.

**U4. Connections per page load; unprompted `/favicon.ico` or `apple-touch-icon` requests.** Not found. Build to: serve `/favicon.ico` as a static path or a 204, and give static GETs a pool separate from the four unauthenticated slots that `/pair` and `/info` use.

**U5. Browser retry when the host drops a connection past the pool (`src/adapters/remote.rs:701`).** Not found. Build to: the page retries with backoff and says "host busy".

**U6. Whether `history.replaceState` clears the `?code=` URL from back-forward cache and session restore, and from the Referer.** Query: "history.replaceState remove query string Referer referrer-policy fragment safari back-forward cache session restore URL"; only how-to pages. Build to: serve `Referrer-Policy: no-referrer`, call `replaceState` first, keep the code single-use with the five-minute window, and say in the dialog that the code is in the camera app's history.

**U7. `Sec-Fetch-Site` on WebSocket handshakes; secure-context status of an IP-literal https origin.** Not found. Nothing in the plan depends on either: the Origin check from 4 stands, and the page uses no secure-context-only API.

## For the owner

**O1. On an iPhone, does the Camera open the QR's https URL in Safari, and after the warning does the page load, and does the events socket open?** Favourable: the socket is used. Unfavourable (expected from 8): the page polls and the dialog describes trusting the certificate on the device for live updates. Works in both: REST polling first, socket as an upgrade.

**O2. Is the address `local_address()` picks the one the phone reaches (several interfaces, a VPN)?** Favourable: one address in the URL. Unfavourable: the window lists the other interface addresses as text beside the URL. Works in both: the list.

**O3. Is one click-through the only trust step acceptable for a `control`-scope phone?** Favourable: the dialog says so. Unfavourable: the plan adds the certificate download. Works in both: default a browser device to `read` scope, with the owner raising it in the pairing window.
