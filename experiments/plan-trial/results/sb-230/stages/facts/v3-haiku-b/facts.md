# Issue #230 facts: browser page on the remote port

Tree at 7fe8949. No spike run: every open question left needs a source we lack or a device. Queries are listed so a not-found can be repeated. WebFetch gave summaries, not verbatim text.

## Settled

- Apple requires a TLS server certificate to carry the DNS name in SAN; CN alone is not trusted. Also EKU serverAuth (issued after 2019-07-01), validity ≤825 days, RSA ≥2048. Silent on manually trusted self-signed certificates. Evidence: https://support.apple.com/en-us/HT210176 (fetched). The current cert has no SAN (`src/adapters/remote.rs:95-127`, per investigation notes, not re-read). Query: "Apple HT210176 requirements for trusted certificates TLS server certificate 825 days subjectAltName extended key usage".
- Chrome 58 and later match identity only against SAN dNSName; a CN-only self-signed certificate fails with `NET::ERR_CERT_COMMON_NAME_INVALID`. Evidence: https://textslashplain.com/2017/03/10/chrome-deprecates-subject-cn-matching/ (2017, fetched via search snippet) and the security-dev intent to remove, https://groups.google.com/a/chromium.org/g/security-dev/c/IGT2fLJrAeo (snippet). Current Chrome not checked. Query: "Chrome 58 deprecates Common Name certificate matching subjectAltName required chromium blog self-signed".
- WebKit does not carry a user's acceptance of an https certificate error to `wss://`. Evidence: https://bugs.webkit.org/show_bug.cgi?id=41419 (NEW, unresolved, last changed 2026-01-12, fetched); the 2011 webkit-dev thread, https://lists.webkit.org/pipermail/webkit-dev/2011-June/017385.html (snippet), says the same. Current iOS Safari not tested. Query: "WebKit WebSocket self-signed certificate exception not applied wss bug".
- A `https` or `wss` origin is a secure context by scheme alone; host, port, and certificate validity do not enter the rule. So a page reached through a click-through is a secure context by the spec. Evidence: W3C Secure Contexts §3.1, https://www.w3.org/TR/secure-contexts/ (fetched). Browser behaviour not tested.
- In a 101 response, `Sec-WebSocket-Protocol` is optional, and if sent it must be one value the client offered; it must not be sent when none is agreed, and never empty. The client value is `1#token`. So the page's token route works only if the 101 echoes exactly the offered token. Evidence: RFC 6455 §4.2.2 step 5 and §4.1 item 10, https://www.rfc-editor.org/rfc/rfc6455#section-4.2.2 (fetched).
- Cookie rules: `Secure` depends on the user agent's definition of "secure" (draft-ietf-httpbis-rfc6265bis §4.1.2.5, §5.7 step 13); `SameSite=Strict` is sent only with same-site requests (§4.1.2.7). The draft does not map `ws`/`wss` to `http`/`https`. Evidence: https://datatracker.ietf.org/doc/html/draft-ietf-httpbis-rfc6265bis (fetched).
- The Stop hook's message is stored: `src/bin/switchboard-hook.rs:37` maps `last_assistant_message` to `last_message`, capped at 200 chars (`:39`); the field is on the record (`src/core/tests.rs:1717`). A grep of `src` and `control` finds no `last_message` in `SessionView` (`control/src/lib.rs`), so the excerpt is not on the wire.
- The watch cap is `WATCH_CAP = 128` (`src/content.rs:42`).
- The pairing code is single-use: the window closes at the first pairing (`src/core/remote.rs:66-68`).
- `RemoteRequest` is built by literal only at `src/adapters/remote.rs:506`; no literal in `tests/`. `MAX_UNAUTHENTICATED = 4` and `MAX_PAIRED = 12` at `src/adapters/remote.rs:567-568`.
- `PairingInvite` is built at `client/Sources/switchboard-remote/main.swift:118` and `client/Sources/SwitchboardClientCore/ConnectionManager.swift:474`; 25 references across `client/Sources` and `client/Tests` (grep).

## Unresolved

- **Does iOS Safari offer a click-through for a no-SAN certificate on a bare IP?** Query: "iOS Safari self-signed certificate "Show Details" visit this website proceed warning IP address no subject alternative name". Found only a report that Show Details did nothing for an IP-addressed NAS (https://discussions.apple.com/thread/256235137, lead) and a Cendio bug saying SAN made iOS 13 connect (https://bugzilla.cendio.com/show_bug.cgi?id=7401). The SAN requirement is settled; the click-through is not. **Build:** a SAN certificate for the LAN IP, and a CA-download route for when no click-through is offered.
- **Do Safari or Chrome keep a click-through across launches or an IP change?** Query: "Chrome "Proceed to" unsafe self-signed certificate exception session only cleared browser restart Safari remembers exception certificate". Only a vendor article (https://help.univention.com/t/11026) saying the next visit proceeds; no duration. **Build:** a page that says "accept the warning again" when its requests fail.
- **Does Chrome offer "proceed" on a missing-SAN error?** A vendor article says no (https://live.paloaltonetworks.com/t5/expedition-articles/access-expedition-gui-using-google-chrome/ta-p/318360, snippet). **Build:** the SAN certificate, which Chrome needs regardless.
- **Does a WebKit page's `wss` work after click-through, and does `fetch` work?** Not found for current versions; `fetch` on the exception was not checked. **Build:** a REST refresh the page falls back to when the socket does not open.
- **Does a page load exhaust the pool of 4 on iOS Safari?** Query: "Safari maximum simultaneous HTTP/1.1 connections per host limit 6 keep-alive idle timeout". Nothing from Apple or WebKit. **Build:** static requests kept out of the unauthenticated pool, or a pool sized by counting the page's requests, without assuming a Safari number.
- **Does Safari send a `SameSite=Strict`, `Secure` cookie on a same-origin WebSocket?** Query: "Safari secure cookie SameSite Strict WebSocket handshake cookies sent same-site". WebKit bug 232666 (Safari 14/15 dropped Strict cookies on WebSocket in restored tabs; "resolved, configuration changed", snippet) and Apple forum 104488 (Strict cookie on a WebSocket upgrade not sent on the next Safari WebSocket; no answer). Current state not settled. **Build:** the subprotocol token works either way, so it can be chosen without this answer.
- **Is a `Secure` cookie accepted for an IP-address https host?** Query: "Secure cookie set over HTTPS IP address host Chrome allowed ...". Nothing for IP hosts; the RFC leaves it to the user agent. **Build:** a cookie name carrying `server_id` if cookies are used, or the token route if not.
- **Which token characters does the subprotocol header accept?** RFC 6455 §4.1 points at RFC 2616's token, whose separator list was not fetched. **Build:** a hex or base64url token, checked by a test through the host's own handshake.
- **Is the code recorded in history before `replaceState` runs?** No source found. **Build:** nothing for the code, which is single-use (Settled); a history entry before the first pairing is a live code for up to five minutes, and the spec must say so.
- **Does `<host>.local` resolve in iOS Safari?** Apple DTS uses `.local` with a trusted test CA in NSURLSession (https://developer.apple.com/forums/thread/70139, fetched); a lead for Safari only. **Build:** a SAN with the IP; add `.local` only if the device run shows it resolves.

## For the owner

1. On your iPhone with the current certificate: does the warning offer a way on, and after you proceed, do requests and live updates work?
   - Both work: the click-through is the trust step; live updates stay primary.
   - Requests only: the page uses its REST refresh fallback. Built in both cases.
2. Does `https://<host>.local:<port>/` load on the iPhone?
   - Yes: `.local` may be a URL host. No: IP only. The SAN carries the IP either way.
3. After accepting the warning, does Safari keep the exception across a restart and an IP change?
   - Kept: no re-accept message needed. Not kept: the page shows "accept the warning again". Built in both cases.
4. With a `Secure; HttpOnly; SameSite=Strict` cookie set, does Safari send it on the page's WebSocket?
   - Yes: the cookie route is open. No: the token route. The plan can choose the token route now.
