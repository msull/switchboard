# Facts for #230 (web control surface)

No spike was run: every open question needing a browser or iPhone is not runnable here, and none of the rest needs an experiment. `git status` is clean.

## Settled

### The tree

- A leaked pairing code is short-lived: the window closes on the first successful pairing, after 3 wrong codes, or at 5 minutes. `src/core/remote.rs:46-72` (`Paired` clears `pairing`, "single-use").
- `RemoteRequest` is built only inside `src/adapters/remote.rs` (definition `:360`); no test in `tests/` constructs it. Adding `cookie`, `origin` and `user_agent` fields touches the adapter alone. Grep: `RemoteRequest\b` over `src tests`.
- `RemoteReply { status, body, authenticated }` is built literally at about 15 sites in `src/app.rs` (e.g. `:1664`, `:2365`, `:2754`, `:3570`, `:3723`, `:3736`, `:3767`) and not in `tests/`. A header field on it needs a `Default` or a constructor, or every site changes.
- `RemoteRequest` and `RemoteReply` live in `adapters/`, not `ports/` or `model.rs`, so CLAUDE.md's additivity rule for the shared contract does not name them. `src/adapters/remote.rs:355-393`.
- `last_message` exists on the session record: the hook binary writes it from `last_assistant_message` (`src/bin/switchboard-hook.rs:37`) and `src/core/tests.rs:1717` constructs records with `last_message: None`. It is not on the wire: no match in `control/` or `client/Sources`. Adding it to `SessionView` is a new field there.
- The certificate can be reissued over the same key. `rcgen::KeyPair::from_pem` is already used at `src/adapters/remote.rs:139,146`, the SPKI hash is of the key (`:107`), and `make_identity` (`:95-127`) is the only path that makes a new key. A reissue keeps `spki_sha256`.
- `PairingInvite` is used by `ConnectionManager.swift`, `BrowserView.swift`, `switchboard-remote/main.swift` and eight test files under `client/Tests`. `addresses` is read as a list at `ConnectionManager.swift:483` and built as a one-element list at `main.swift:120`. A URL parser can fill `addresses` with the one host:port.

### Outside the tree

- **Apple's TLS server certificate rules.** SHA-2 signature, RSA at least 2048 bits, the server's DNS name in subjectAltName (CommonName not trusted), and for certificates issued after 1 July 2019 an EKU of `serverAuth` and a validity of at most 825 days. Non-compliant connections fail and "websites [may] not load in Safari". Evidence: https://support.apple.com/103769 (iOS 13 / macOS 10.15; Citrix says iOS 14 / macOS 11 also). The page does not mention IP-address SANs, user-added roots, or exemptions. The current certificate has no SAN, a CN only, and a not-after of year 4096 (`src/adapters/remote.rs:95-127`), so it meets none of the SAN, EKU or validity rules.
- **Safari and `wss://` after a click-through.** The behaviour exists and is unfixed: WebKit bug 41419 ("allow wss:// if user has accepted https:// warning") is open, status NEW, last modified 2026-01-12, and the reporter got `wss` working only after marking the certificate always-trusted (https://bugs.webkit.org/show_bug.cgi?id=41419). code-server's iPad doc agrees: Safari "will prevent WebSocket connections unless the certificate is installed as a profile", needs a domain name rather than a bare IP, and needs a SAN matching the host (https://github.com/cdr/code-server/blob/v3.7.0/doc/ipad.md). Two sources agree; they cover Safari up to 2026-01. One more report is mixed: on iOS 26 beta 3, `wss` failed after acceptance and the poster said it was resolved in beta 5 (https://developer.apple.com/forums/thread/792842). That is a forum post, a lead only.
  - Queries: "Safari iOS WebSocket wss self-signed certificate accepted exception fails fetch works"; "iOS Safari self-signed certificate no subjectAltName IP address proceed certificate warning".
- **Secure contexts.** `crypto.randomUUID()` is available only in secure contexts (https://developer.mozilla.org/en-US/docs/Web/API/Crypto/randomUUID). The spec decides trustworthiness by the origin's scheme: `https` and `wss` are "Potentially Trustworthy", with no mention of certificate errors (https://w3c.github.io/webappsec-secure-contexts/ §3.1; MDN Secure_Contexts agrees). By the text, an https page on a click-through certificate is a secure context. Whether browsers also treat it so is not stated, and it is under Unresolved.
- **Cookies are not isolated by port.** RFC 6265 says so, quoted on https://developer.mozilla.org/en-US/docs/Web/HTTP/Guides/Cookies. `Secure` cookies are sent only over HTTPS. `localStorage` is per origin including port (MDN, general). This confirms the notes' cross-port claim.
- **WebSocket subprotocol (RFC 6455 §4.1, §4.2.2, https://www.rfc-editor.org/rfc/rfc6455).**
  - The client must fail if the 101 names a subprotocol it did not offer.
  - The server's reply is one `token`, and an empty value is illegal.
  - Offered values are `token`s, so base64url without padding is legal. Base64 `=` is not a token character (a separator).
  - Whether the client must fail when the server returns none is not normative: §4.2.2 says the server sends none when it agrees to none, and §1.9 (non-normative) says it must include it.
  - Browsers must send `Origin` on the handshake. A server may validate it, and rejects with 403. "If the server does not validate the origin, it will accept connections from anywhere."
  - The tree's `respond_upgrade` (`src/adapters/remote.rs:519-527`) echoes no subprotocol.
- **`history.replaceState` replaces the current entry**; Back then shows the new URL and bypasses the old one (https://developer.mozilla.org/en-US/docs/Web/API/History/replaceState). It says nothing about what the browser recorded before the script ran.
  - Query: "MDN history.replaceState".

## Unresolved

Each line ends with what the plan builds to. The plan never waits on a line here.

1. **Does a click-through exception cover `wss://` and `fetch` on current iOS Safari, with this no-SAN certificate on a bare IP?**
   - Tried: the queries above, plus "Safari remember certificate exception visit this website how long self-signed iOS per session". The only evidence is old (2010 bug, iOS 7 and 13 era), the code-server doc (v3.7, 2021), and the one iOS 26 beta post. Nothing current and primary exists.
   - Favourable: the page may use the events socket directly. Unfavourable, and the plan must do this anyway: the page works with REST polling alone (poll `changed`-equivalent reads on a timer) and treats the socket as an upgrade it may lose, saying "live updates unavailable" rather than failing.
   - Reissuing the certificate with a SAN, `serverAuth` and a bounded validity over the same key is cheap and helps in both cases (Apple's page; the code-server doc).
2. **Does a SAN with an IP address, rather than a DNS name, satisfy Safari and Chrome?** The Apple page speaks only of DNS names. Queries: as in 1, plus "iOS Safari self-signed certificate no subjectAltName IP address". No primary source found.
   - Favourable: put the LAN IP in the SAN. Unfavourable: the page's URL host must be a `.local` name. Either way the plan reissues with the `.local` hostname as a DNS SAN as well.
3. **Does Apple's 825-day rule apply to a manually trusted self-signed leaf?** The page lists no exemption and no scope. Build to a validity of 825 days or fewer and say in the dialog that the certificate is renewed (the key, and so the pin, is unchanged). If it does not apply, nothing is lost.
4. **Does a click-through certificate give a secure context in Safari, Chrome and Firefox?** The spec reads yes by scheme, but I found no browser documentation on certificate-error overrides. The plan uses `crypto.randomUUID` only when `window.isSecureContext`, with a fallback id generator built on `getRandomValues`, and does not rely on `Secure` cookies being stored without a manual test (see item 5).
5. **Are `SameSite=Strict` cookies sent on a same-origin `new WebSocket()` handshake, and do `Secure` cookies work on an IP host, in Safari, Chrome and Firefox?** Query: "SameSite=Strict cookie sent on same-origin WebSocket handshake; Secure cookie IP address host Safari Chrome Firefox". Results were blogs and a Chromium design thread (WebSocket requests carry cookies; `wss` counts as the same scheme as `https`): leads, not evidence. Plan to either transport behind one server-side reader that accepts the bearer header, the cookie and the subprotocol token, and let the first manual run pick the page's transport.
6. **Is the code-bearing URL recorded in history, tab list or iCloud tab sync before `replaceState` runs?** No source. The plan assumes yes: the code is single-use and the window is 5 minutes or 3 failures (settled above), so a recorded URL is dead once pairing succeeds. The plan says that in the dialog and keeps the query out of server logs (`refuse`, `src/app.rs:3754`, logs path only).
7. **Per-site exception lifetime across launches, and loss when the IP changes.** Query: "Safari remember certificate exception ... how long". Only a 2013 blog (iOS 7: exception not removable). Plan: an address change means another warning, and the page and dialog say so. A `.local` name or a reissue never makes it worse.
8. **Pool of 4 under a real page load (iOS Safari parallel connections, keep-alive, `/favicon.ico`).** No source for the connection count; it needs a device. Plan: answer static paths on the connection thread with `Connection: close`, serve the page as few files as possible (HTML plus one script and one stylesheet, inlined where possible by the CSP), and give static paths their own bound so a page load cannot starve `/pair`.
9. **Bonjour `.local` resolution from iOS Safari.** No query run, so no evidence. Plan: the URL carries the IP from `local_address()` and the `.local` name only if the owner answers item B below.

## For the owner

A. **On a real iPhone/iPad, after accepting the warning, does the page load, do `fetch` calls work, and does `new WebSocket("wss://…")` connect, with the current certificate and with the reissued one?** Ask with the page as built. Favourable branch: use the socket. Unfavourable: ship polling. The plan builds the polling path regardless.
B. **Does `https://<mac-name>.local:<port>/` resolve from the owner's phone?** Favourable: reissue with that DNS SAN and put it in the URL. Unfavourable: the IP stays the URL host. Reissuing with both works either way.
C. **Does the owner's phone camera open the URL straight into Safari, and does the tab appear in iCloud tab sync or history before the page script runs?** Either branch: the code is single-use and expires, so the plan holds.
D. **Does a cookie, or a subprotocol token, work in Safari with the owner's certificate (items 5, 1)?** Favourable: use the one that works, the cookie preferred for the token never being readable by script. Unfavourable: use the other; the server accepts all three transports regardless.
