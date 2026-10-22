# Facts for issue #230 (web control surface)

No spike was run. Every open question that a source settles is below. The rest need a real browser, a phone or a scan, and go to the owner. Nothing was built or run here, so no `[run]` question is settled.

## Settled

Tree:
- `tls_config` takes the stored DER as is and the key from PEM, so a cert reissued from the same key keeps the SPKI pin. Only `certificate_der` (`src/adapters/remote.rs:131`) reads `identity.certificate`. Evidence: `src/adapters/remote.rs:96-135, 144-155`.
- The cert has no SAN, only a CN. Evidence: `src/adapters/remote.rs:102-106`.
- The cert's `not_after` is rcgen's default, year 4096, by the code's own comment. Evidence: `src/adapters/remote.rs:91`. The notes' question on validity limits is therefore a browser question (see Unresolved), not a tree one.
- `devices.json` is a bare JSON array of `PairedDevice` with no version and no migrate. A browser device is the same record, so no store change and no `SCHEMA_VERSION` bump. Evidence: `src/adapters/remote.rs:52-90`.
- No route returns the device's own scope or name. The only scope-bearing reply is `/pair` (`src/app.rs:3662`). The GET arms are projects, spaces, sets, sessions, waiting, ops, runs, dispatch tickets and content. Evidence: `src/app.rs:3030-3113`.
- `respond` is called 5 times (`src/adapters/remote.rs:789, 819, 839, 864`, plus the definition at 530) and `respond_upgrade` once (853). Changing either signature touches only that function and these sites.
- `last_message` is not on the session record. It exists only on the hook event (`src/ports/events.rs:24`, `src/adapters/hooks.rs:71,196`, `src/bin/switchboard-hook.rs:37`). There is no `last_message` match in `src/core/` outside tests. The spec's claim that it is "a field of the session's metadata" does not hold, so the card excerpt is not on the wire.
- `agent_excerpt` takes a `&Conversation` from the UI's cache (`src/ui/cards.rs:558`), so the wire cannot compute it today. The page can fetch `last-response` per Claude Code session (`src/app.rs:1967`), or `SessionView` gains a field (additive).
- The only test pinning the QR JSON is `tests/ui.rs:7487-7492` (`code`, `spki_sha256`, `addresses`). Changing the QR or the Copy invite text breaks that test, so it is amended.
- A revoked token and a rotated-key token get the same 401. Evidence: `src/app.rs:3146`. "Revoked" vs "identity changed" needs page-side inference from `/info` (`spki_sha256` differs from the pinned one means rotated) or a host tombstone.
- Unauthenticated slots: a static-only connection never sets `authenticated`, so it holds its slot until close or idle timeout. Evidence: `src/adapters/remote.rs:707, 836-847, 573, 785`. The pool is 4 (`MAX_UNAUTHENTICATED`, 567), so a multi-connection page load plus `/pair` can be dropped at accept (701-704). Whether a real browser actually does this is a `[run]` question.
- `read_request` drops `Cookie`, `Origin`, `Sec-WebSocket-Protocol`, `Content-Type`. `respond` hard-codes `Content-Type: application/json` and sends no cache or security headers. Evidence: `src/adapters/remote.rs:444-553`.

Outside the tree:
- A script cannot set request headers on `new WebSocket()`. The constructor takes only `url` and `protocols`, and the user agent sets the handshake headers. https://websockets.spec.whatwg.org/
- With a non-empty `protocols` list, the connection fails unless the server's response selects one of the offered subprotocols. A server must send no `Sec-WebSocket-Protocol` header if it agrees to none, and a client must fail the connection if the server names one it did not offer. https://websockets.spec.whatwg.org/ and https://www.rfc-editor.org/rfc/rfc6455.html (§4.1 item 6, §4.2.2). So the subprotocol route needs `respond_upgrade` to echo exactly one offered value. The echoed value does not have to be the token: the page may offer `["switchboard.v1", "<token>"]` and the host echoes `switchboard.v1`, which also keeps the token out of the reply.
- A browser client MUST send `Origin` on the WebSocket handshake. https://www.rfc-editor.org/rfc/rfc6455.html (§4.1 item 8). So the host can require `Origin` equals its own authority on the upgrade.
- `Sec-Fetch-Site` has the values `same-origin`, `same-site`, `cross-site`, `none`. It is forbidden to script, sent only to potentially trustworthy URLs, and Baseline since March 2023. https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Sec-Fetch-Site. MDN does not say it is sent on the WebSocket handshake, so the host should rely on `Origin`, with `Sec-Fetch-Site` as an extra, not the only check.
- `Secure` cookies are sent only on `https:` requests (and on localhost). https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Set-Cookie
- `history.replaceState` modifies the current entry and adds none, so Back skips the old URL. The URL must be same-origin. https://developer.mozilla.org/en-US/docs/Web/API/History/replaceState. The caveat is that the first request, with the code in its query, is already in the server's hands and in any proxy; the host's `refuse` log omits the query (`src/app.rs:3754`), but the static handler must not log it either.
- `connect-src 'self'` "does not resolve to websocket schemes in all browsers". https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Content-Security-Policy/connect-src. The CSP must therefore name the socket explicitly, for example `connect-src 'self' wss://<authority>`. The host knows its authority from the `Host` header, which `read_request` would also have to keep, or the CSP must be built per request.
- `crypto.randomUUID()` is secure-context only and Baseline since March 2022 (Safari 15.4 by MDN's general knowledge, not confirmed on the page). https://developer.mozilla.org/en-US/docs/Web/API/Crypto/randomUUID. An `https:` page through a cert exception is normally still a secure context, but that part is unconfirmed (see Unresolved). The page should feature-detect and fall back to `getRandomValues`.
- Chrome 58 removed CN matching and requires a SAN. https://developer.chrome.com/blog/chrome-58-deprecations. This confirms that a SAN-less cert warns for name mismatch in Chrome, and the fix is a reissue with an IP SAN, or a warning-only click-through. The IP-SAN claim for a bare IP is implied, not stated, on that page.

## Unresolved

Each was tried by a fetch of the vendor documentation above, and none of those pages states it:
- Whether iOS Safari accepts a SAN-less self-signed cert for a bare IP and keeps the exception per host/port or per cert; whether a changed cert warns again.
- Whether a `Secure` cookie is stored and sent through a cert exception, including on the WebSocket handshake, and whether `window.isSecureContext` is true there.
- What "same-site" means for an IP host and a different port. MDN defers to the Glossary, which I did not fetch; general knowledge says scheme plus host, port ignored, with an IP as its own site. Not evidence.
- Whether `Origin` and `Sec-Fetch-Site` arrive on the upgrade in iOS Safari and Chrome (RFC says `Origin` must; `Sec-Fetch-Site` on the handshake is not documented).
- Whether the iOS camera opens an `https://<ip>:<port>/` link with a self-signed cert.
- Whether any browser caps the validity of a user-accepted self-signed cert (year 4096 here).
- Whether rustls 0.23 with `ring` negotiates with iOS Safari and Chrome (no source read; the visionOS client already connects).
- The user-agent strings of iPhone/iPad Safari and Chrome (iPad desktop mode reports macOS). Needs a device or a UA reference; not fetched.
- All `[run]` questions (parallel connection count per page load, slot hold time, favicon request, frame cost of statics, QR density of the URL form, Swift tests after `PairingInvite.parse` accepts a URL). None was run, since nothing was built.
- Whether `agent_excerpt` could run from the record plus transcript on the host side (`src/ui/cards.rs:558` reads the UI's conversation cache); that is design, not a fact, and is left for the plan.

## For the owner

1. **Trust step: click-through or downloadable cert.** If click-through: say so in the paired-devices dialog, add an IP SAN by reissuing from the same key (SPKI unchanged) so the warning is only about the issuer, and accept that an impostor page is not stopped. If download: serve the cert at an extra pre-pairing path (a §5 amendment), and the owner installs and trusts it on each device (a profile install on iOS needs a manual step, unverified here).
2. **Token transport: cookie or subprotocol.** If cookie: the host needs `Cookie` and `Origin` parsing, `Set-Cookie` on the `/pair` reply, an `Origin`-equals-authority check on every POST and the upgrade, and the dialog says a stolen browser equals a stolen token. If subprotocol: the token sits in script memory or `localStorage`, readable by any script on the origin (the CSP forbids inline and external scripts), and the host echoes a fixed protocol name. The cookie route depends on the unresolved cert-exception cookie behaviour above, so the plan should include a phone check as its first step.
3. **One QR.** If one URL: `PairingInvite.parse`, `main.swift:110-111`, `BrowserView.swift:883` and `tests/ui.rs:7487` change, and the URL form keeps `Copy invite`. If two: the JSON stays for native and the URL goes to the camera; the cost is two payloads in the window.
4. **Advisory `key` in the URL.** If kept: the page compares it to `/info`, and the host already checks `spki_sha256` in `/pair`. If dropped: shorter URL, and nothing is lost that the host's own check does not cover.
5. **Static paths and pools.** If statics are answered on the frame (a new arm in `serve_remote`): they cost one of the 8 drains per pass (`src/app.rs:1587`) and share the 4-slot unauthenticated pool; consider raising the pool or giving statics their own. If on the connection thread: no frame cost, but a new path in the adapter. Rate-limit statics either way.
6. **Revoked vs rotated.** If a tombstone: a `devices.json` addition and a distinct 401. If page-side inference: the page compares `/info` `spki_sha256` with the pinned key and says "revoked" otherwise.
7. **Device's scope on reload.** If a new read route (for example `GET /api/v1/me`): additive; the page hides the reply box from it. If stored in the browser: stale after a scope change on the host.
8. **Read-scope devices.** If they see the page: reply box hidden. If only control-scope: say so in the pairing window.
9. **Several LAN addresses.** `local_address` returns one (`src/adapters/remote.rs:962`). If the QR should carry more than one, the URL form carries only one authority; pick the primary or show a list.
10. **Acceptance on a phone** (scan, warning, sessions, live reorder, reply, revoke) and the headset paste of the URL form need a device and cannot be run here.
