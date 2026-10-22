# Facts for issue #230 (web control surface)

No spike was run. Every open question is either answered by a source or needs a device, so no experiment here could settle it. The worktree has no changes.

## Settled

- The certificate has no Subject Alternative Name and its CN is `Switchboard <server_id>`. Evidence: `src/adapters/remote.rs:102` (`CertificateParams::new(Vec::<String>::new())`), `:91` (not_after "year 4096").
- Apple platforms do not trust a DNS name that appears only in the CommonName. They want the name in the SAN, `serverAuth` EKU, and validity of 825 days or fewer for certificates issued after 2019-07-01. The current certificate fails all three, so Safari may refuse it outright instead of showing a clickable warning. Evidence: https://support.apple.com/en-us/103769. Whether Safari offers "Visit this website" for it is Unresolved.
- A bare IP in the URL needs an IP-type SAN to match, and the certificate has none. The Apple page speaks only of DNS names, so this half is inference. Evidence: none beyond the rows above (see Unresolved).
- `new WebSocket(url, protocols)` takes no headers. A token can travel only in the URL, in a cookie, or in the subprotocol list. Evidence: https://developer.mozilla.org/en-US/docs/Web/API/WebSocket/WebSocket.
- Subprotocol carrier, RFC 6455:
  - Values must be tokens (U+0021–U+007E minus separators). Base64url fits.
  - If the client sends the header and the server omits it from the 101 reply, the browser fails the connection. The server must echo exactly one of the values.
  - Browsers must send `Origin` on the upgrade. Checking it is optional for the server, and without a check the server accepts connections from anywhere.
  - Evidence: https://www.rfc-editor.org/rfc/rfc6455 §1.9, §4.1, §4.2.2.
- `respond_upgrade` (`src/adapters/remote.rs:519`) echoes no subprotocol and checks no `Origin`. `read_request` (`:444`) keeps only `content-length`, bearer, and the upgrade key. Both would need additive changes for either carrier.
- Cookie attributes:
  - `HttpOnly` hides the cookie from script, but the browser still sends it on `fetch` calls.
  - `Secure` cookies are sent only over `https:`.
  - `SameSite=Strict` is relative to the *site*, which is scheme plus eTLD+1 with the port ignored. Another service on the same IP at another port is same-site, so a cookie token needs a server-side `Origin` check on the upgrade and on `POST`.
  - Evidence: https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Set-Cookie; https://web.dev/articles/same-site-same-origin.
- CSP `connect-src 'self'` does not cover `ws:`/`wss:` in all browsers. The plan should name `wss://<origin>` or `wss:` explicitly. Evidence: https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Content-Security-Policy/connect-src, which cites w3c/webappsec-csp#7.
- `respond` (`src/adapters/remote.rs:530`) always sends `Content-Type: application/json` and no other header, and the status reasons are a fixed list. Serving HTML needs content type, CSP, cache and cookie headers added to `RemoteReply`. Only `src/adapters/remote.rs:360/506` and `src/app.rs` (about 20 sites) construct `RemoteReply`. `RemoteRequest` is built only at `src/adapters/remote.rs:506`, so adding a field there breaks no test or fake literal. Evidence: grep `RemoteRequest {` and `RemoteReply {` over `src` and `tests`.
- `Limits::limited` rate-limits only the exact `/pair` and `/info` paths. A page load counts against no rate window, only the 4-slot unauthenticated pool. Evidence: `src/adapters/remote.rs:631`; `docs/remote-client.md` §4.1.
- `serve_remote` (`src/app.rs:3017–3023`) refuses any path outside `/api/v1/` with a JSON 404. The tests that assert this are `tests/remote.rs:426/462` (`/other`) and `:682`. They stay valid if `/other` is still refused.
- The card's one-line excerpt is not on the wire. `SessionView` (`control/src/lib.rs:637`) has no excerpt. `agent_excerpt` is in `src/ui/cards.rs:558`, and shell captions are `UiState` only (`cards.rs:177–182`). `last-response` (`GET /api/v1/sessions/{id}/last-response`, `docs/remote-client.md:173`) goes through the single content worker, `ContentKey::Conversation` (`src/app.rs:1967`). Each viewed key joins the watch set (cap 128, from the investigation's `WATCH_CAP` note). The plan must choose: N content reads, or an additive `SessionView` field computed in the core.
- No tree document says anything about a browser surface, a cookie, or certificate download. `grep` for browser/cookie/origin over `docs/remote-client.md`, `docs/design.md` and `spikes/22-remote-port/README.md` finds only "browser" meaning the native app's session browser.
- Swift `PairingInvite.parse` decodes JSON only (`client/Sources/SwitchboardProtocol/Info.swift`). The call sites are `client/Sources/switchboard-remote/main.swift:107` and `client/Sources/SwitchboardUI/BrowserView.swift:106/883`. A URL form needs a parser branch and a new fixture line in `tests/remote.rs::write_fixtures` (`:1587`).
- `cx.remote_port` comes from `app.remote_addr()` (`src/ui/mod.rs:420`) and falls back to `identity.listen.port`. The QR address is `local_address()` even when `listen.lan` is false, so the phone would be handed an address that does not answer. This is an existing gap that the plan should close (warn, or require LAN on).
- Build: root `Cargo.toml:91` excludes only `vendor/egui_term`, and `include_str!("../web/...")` from a root-level directory needs no packaging change. CI is a single `.github/workflows/ci.yml`. No script lists a file set (`grep` of `scripts/bundle.sh`, `ci.yml`, `Cargo.toml` found none), though `scripts/release.sh` was not read.

## Unresolved

- Whether Safari (iOS/iPadOS/macOS), Chrome, Firefox and Android Chrome allow a click-through for this certificate (no SAN, CN-only, year-4096 expiry, opened by bare IPv4). Tried: Apple's requirements page (above), which says the certificate fails and "can break ... Safari page loads". Chrome's CN-fallback removal could not be confirmed, because chromestatus returned no text. Needs a device. If the click-through is a hard error, decision 1 is forced: the host must put the IP in a SAN, or serve a downloadable certificate.
- Whether `fetch`, `wss://`, and a `Secure` cookie keep working after a click-through, per browser and for how long. No primary source found. Needs a device.
- iOS certificate-profile install for a self-signed leaf with no CA bit: whether it is accepted and then trusted under Certificate Trust Settings. Tried only the Apple server-requirements page, which does not cover profiles. Not settled.
- Whether the iOS camera opens `https://<ipv4>:<port>/?code=…&key=<64 hex>&id=<uuid>` (about 140 characters) as a link, and whether the camera or a link previewer fetches the URL before the owner taps. No source found. Needs a phone.
- Whether `history.replaceState` keeps the code out of Safari's history and recents. No source found. Needs a phone.
- `User-Agent` naming: iPadOS Safari reports a desktop Mac by default. No source fetched. Needs a device.
- Whether the Safari/iOS WebSocket reconnects after sleep, and how it behaves after a certificate exception. Not found.
- Parallel-connection behaviour of a phone against the 4-slot unauthenticated pool. Needs a running host and a phone (not run here).
- Browser URL rules for IPv6 literals, link-local zones and `.local`. Not fetched. `local_address()` currently returns one IPv4.
- Whether `Cache-Control`/`Referrer-Policy: no-referrer`/`X-Content-Type-Options: nosniff` suffice for a URL that carried a code. Standard practice and no primary source fetched. The plan can adopt them, but they are unverified.
- How a later visit learns its scope. The `/pair` reply carries `scope`, but the cookie holds only the token, and there is no `/me`. The plan can read the scope from the `/pair` reply and remember it, or add an additive field. This is a design choice, not a world fact.

## For the owner

- **Certificate trust.** Does the owner accept a click-through warning on the local network as the trust step (said in the paired-devices dialog), or want a downloadable certificate?
  - If click-through and the target browsers refuse a no-SAN certificate: the plan must reissue the certificate with the IP and `serverAuth` in a SAN and validity of 825 days or fewer. This changes the SPKI only if the key changes. A re-pin with a new certificate on the same key keeps the pin, but the plan must check `identity.json` migration.
  - If download: the plan adds a certificate route and dialog text. A profile-install flow has to be tried on a device first.
- **Token carrier.** The plan should pick based on the answer.
  - Cookie (`HttpOnly; Secure; SameSite=Strict`): the script never sees the token, but the host must check `Origin` on the upgrade and on `POST` and needs a `Set-Cookie` header on the `/pair` reply.
  - Subprotocol: needs the 101 echo, and the token sits in script-readable storage.
- **One URL or JSON too.** May the QR's JSON be dropped? If yes, the CLI and headset parsers take the URL only, and the fixtures change. If no, parsers accept both and the QR is the URL.
- **Excerpt on the wire.** May `SessionView` gain an additive excerpt field? If yes, compute it in the core (moving `agent_excerpt`). If no, the page fetches `last-response` per visible session, bounded by the worker and the watch cap.
- **Devices for acceptance.** Does the owner have a phone, an iPad and a signed bundle on the LAN? If not, the acceptance steps (scan, warning, waiting rises, reply, revoke) are not verifiable and tests must stand in with the host's own HTTP client.
- **Browser device name.** What should an iPad (which reports as a Mac) be called? If the page may edit the name before pairing, `/pair` already takes `device_name`, and no schema change is needed. If the host derives it from `User-Agent`, `RemoteRequest` gains an additive field.
