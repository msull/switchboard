# Facts for #230: a web control surface on the remote port

Worktree at 7fe8949. No spike was run. The one question that the plan
turns on and that no source settles (does iOS Safari open `wss://` to a
clicked-through self-signed host?) needs an iPhone, so it goes to the
owner. Every other question was settled by the tree or a primary source.

## Settled

### Tree

- The certificate can be reissued over the same key without rotating pairings. `rcgen::KeyPair::from_pem` already reads the Keychain PEM (`src/adapters/remote.rs:139,146`). `CertificateParams` exposes `subject_alt_names`, `not_after` and `extended_key_usages` (`~/.cargo/registry/src/*/rcgen-0.14.10/src/certificate.rs:58,60,64`). The pin is `sha256(subject_public_key_info)` of the key (`src/adapters/remote.rs:117`), so a new certificate over the same key keeps `spki_sha256`. Today only `make_identity` (`src/adapters/remote.rs:95`) makes certificates, and it always generates a new key (:101), so a reissue path is new code.
- The certificate today has no SAN, and its validity runs to 4096-01-01. `CertificateParams::new(Vec::<String>::new())` is at `src/adapters/remote.rs:102`, and rcgen's default is `not_after = date_time_ymd(4096,1,1)` with `subject_alt_names: Vec::new()` (`rcgen-0.14.10/src/certificate.rs:85,92`). rcgen turns a string that parses as an IP into `SanType::IpAddress` (`certificate.rs:117`).
- The JSON invite is parsed only by the CLI (`client/Sources/switchboard-remote/main.swift:109`), the headset's paste field (`client/Sources/SwitchboardUI/BrowserView.swift:883`), `SWITCHBOARD_INVITE` (`BrowserView.swift:106`, `client/README.md:180-190`) and the Swift test literal (`client/Tests/SwitchboardProtocolTests/FixturesTests.swift:161`). Other Swift tests build the `PairingInvite` struct directly, not JSON (for example `client/Tests/SwitchboardClientCoreTests/NewSessionTests.swift:50`), so a URL parser leaves them alone.
- Nothing in the remote path logs `request.query`. The only reader is the accessor at `src/adapters/remote.rs:378`, the adapter's one log line is `log::debug!("remote connection: {e}")` (:714), and the `remote:` log lines in `src/app.rs` (1663, 1678, 1716, 2712) name a kind, an id or a code, never a target.
- `devices.json` is not under the `model.rs` `SCHEMA_VERSION` rule. `PairedDevice` lives in `src/ports/remote.rs:96`, not `src/core/model.rs` (whose `SCHEMA_VERSION = 16` is at :12). Only `identity.json` has its own version, `IDENTITY_SCHEMA_VERSION` (`src/ports/remote.rs:17`). An additive `#[serde(default)]` field follows `last_seen`.
- The card's one-line excerpt for an agent is derived in the UI, not stored. `agent_excerpt` (`src/ui/cards.rs:558-574`) gives the first non-empty line of the last turn's `final_text`, else the last activity line, else `You: <prompt>`, capped at 160 characters. For other kinds it is the pane caption or the host title (`src/ui/cards.rs:177-184`). So no record field holds it, and a wire field would have to compute it on the host from the parsed conversation.
- The bearer token is base64url without padding (`src/adapters/remote.rs:340` `URL_SAFE_NO_PAD`). Its alphabet `A-Z a-z 0-9 - _` is entirely RFC 7230 `tchar`, so it is a legal `Sec-WebSocket-Protocol` token as it stands.
- `respond_upgrade` writes the 101 by hand with no `Sec-WebSocket-Protocol` (`src/adapters/remote.rs:519-522`). Echoing one is a change to that format string; tungstenite is not involved in the handshake.
- A UI test of the dialog exists: `remote_devices_dialog_opens_pairing_and_revokes_a_device` (`tests/ui.rs:7424`). The script line `pairing <code>|close` exists (`src/script.rs:729-730`).
- The embedded page's referenced paths can be a fixed list in the test, because the page has no build step. The precedent for `include_str!`/`include_bytes!` is at `src/main.rs:167` and `src/ui/theme.rs:203-211`. Nothing in the tree parses HTML, so parsing `src=`/`href=` would be new code. This is a plan choice, not a fact gap.

### Protocols and standards

- A browser's WebSocket handshake carries cookies. It is made with credentials mode `include` (https://websockets.spec.whatwg.org/, "establish a WebSocket connection"). The page opens it to its own origin, so the request is same-origin and therefore same-site, and a `SameSite=Strict` cookie is not withheld by SameSite (https://httpwg.org/http-extensions/draft-ietf-httpbis-rfc6265bis.html).
- A browser fails the handshake if it offered subprotocols and the 101 does not select one. WHATWG step: if protocols is non-empty and the response `Sec-WebSocket-Protocol` is null or empty, "fail the WebSocket connection" (https://websockets.spec.whatwg.org/). RFC 6455 §4.2.2 says the selected value must be one the client offered (https://www.rfc-editor.org/rfc/rfc6455#section-4.2.2). So the subprotocol route needs the host to echo a value from the offer.
- Under the subprotocol route, `new WebSocket(url, [token])` throws `SyntaxError` for a value that is not an RFC 6455 token: printable ASCII U+0021 to U+007E, no separators (https://www.rfc-editor.org/rfc/rfc6455#section-4.1, https://websockets.spec.whatwg.org/). The token passes (see the tree line above).
- Cookies are not isolated by port. A cookie set by `https://<ip>:<port>` is sent to every port on that host (https://httpwg.org/http-extensions/draft-ietf-httpbis-rfc6265bis.html, §1 and §4.1.3.2). With no `Domain` the cookie is host-only (§4.1.2.3, §5.7 step 10). So any other HTTPS service on the Mac's address receives the cookie, and the plan should say so.
- A `Secure` cookie is stored and sent only over a channel the browser calls secure (rfc6265bis §4.1.2.5, §5.7 step 13). An `https:` origin is "potentially trustworthy", and the algorithm has no certificate-validity condition (https://www.w3.org/TR/secure-contexts/, §3.1 step 3). Whether a given browser downgrades a page reached through a certificate override is not stated there (see Unresolved).
- `connect-src 'self'` covers a same-origin `wss://`. CSP3 §6.7.2.8 step 4 matches when the host matches, the ports are the same, and the URL scheme is `https` or `wss` (https://www.w3.org/TR/CSP3/). No `wss://host:port` needs naming.
- `frame-ancestors` is ignored in a `<meta>` CSP, so it must be a response header (https://www.w3.org/TR/CSP3/ §3.3).
- Safari's tracking prevention deletes all script-writeable storage (localStorage, IndexedDB, `document.cookie` cookies) after 7 days without user interaction with the site. It does not cap first-party `Set-Cookie` cookies. A home-screen web app's domain is exempt (https://webkit.org/tracking-prevention/, "7-Day Cap on All Script-Writeable Storage" and "CNAME and Third-Party IP Address Cloaking Defense"). So a token in `localStorage` is lost after a week unused on iOS Safari; an `HttpOnly` `Set-Cookie` token is not.
- Apple's TLS server certificate rules require a SAN carrying the server's name ("all TLS server certificates"). Certificates issued after 2019-07-01 also need validity of 825 days or less and an EKU with `id-kp-serverAuth` (https://support.apple.com/en-us/103769). The current certificate meets none of the three. The page does not say whether the rules apply to a certificate the user clicked through or installed.
- A manually installed root is trusted on iOS through Settings > General > About > Certificate Trust Settings, "Enable full trust for root certificates" (https://support.apple.com/en-us/102390). That implies the downloaded certificate must be a CA (basicConstraints CA:TRUE). The article does not state that a leaf cannot be listed.

## Unresolved

- **Does iOS Safari open `wss://` to a host whose self-signed certificate the user clicked through?** The acceptance criterion depends on this. Evidence conflicts and none of it is from Apple:
  - WebKit bug 41419 (open, last modified 2026-01-12): wss to a self-signed host failed on desktop Safari until the certificate was set to always trust (https://bugs.webkit.org/show_bug.cgi?id=41419).
  - Cockpit's documentation: Safari's default WebSocket framework on iOS, iPadOS and ARM Macs refuses wss to self-signed hosts "even when they are manually marked as trusted". This is a project's documentation, so a lead (https://cockpit-project.org/running/safari).
  - An Apple forum report that iOS 26 beta 3 failed wss after the page's certificate was accepted, with "The certificate for this server is invalid", and the poster's note that beta 5 resolved it. No Apple reply (https://developer.apple.com/forums/thread/792842).

  It is not settled because it needs an iPhone and a person's tap through the warning; no source states current behaviour. It goes to the owner.
- Whether iOS Safari offers a click-through at all for a certificate with no SAN, or with validity past 825 days. Apple's page states the requirements but not whether the override applies (https://support.apple.com/en-us/103769). Needs a device.
- Whether a browser treats a page reached through a certificate override as a secure context. The spec says yes by scheme (https://www.w3.org/TR/secure-contexts/ §3.1), but no browser vendor source was found. Needs a device. It matters for `Secure` cookies and `crypto.randomUUID`.
- Whether a QR-opened URL reaches Safari history, the camera's history or iCloud Tabs before `history.replaceState` runs. No source found; Apple does not document it.
- Whether a backgrounded or locked iOS Safari tab keeps its WebSocket open. No primary source was found; forum reports exist only for the 2021 "NSURLSession WebSocket" experiment. Plan for a reconnect and resync on `visibilitychange` regardless.
- How many parallel connections iOS Safari opens to one origin on page load, and whether it reuses a keep-alive connection within the 30 s idle window. This needs a device and the running host; no source was searched beyond that.
- Whether iOS Safari resolves `<name>.local` reliably, and accepts a click-through for a certificate naming it. Not searched to a primary source; needs a device.

## For the owner

- **Does iOS Safari open the events `wss://` after you accept the certificate warning?** Test on the target iPhone, on the iOS version in use, with a throwaway host.
  - Yes: the click-through can be the trust step (decision 1).
  - No: either the host offers a CA certificate to install and trust first (reissue with SAN, serverAuth EKU, ≤825 days, CA:TRUE, see Settled), or the page polls REST on a timer in place of the socket.
- **Decision 1: click-through or a downloaded certificate?**
  - Click-through: reissue the leaf with a SAN for the address and a serverAuth EKU anyway, so the override is offered. Say in the dialog that the browser trusts what you clicked.
  - Download: serve a CA certificate before pairing (one more pre-pair path under §5), plus the Certificate Trust Settings steps in the dialog.
- **Should the certificate name the IP, and be reissued when the address changes?**
  - Yes: reissue on bind over the same key; pins survive.
  - No: every address change means a new warning and a new origin, so the browser pairs again.
- **Decision 2: cookie or subprotocol?**
  - Cookie: `HttpOnly; Secure; SameSite=Strict`, no `Domain`. It survives ITP and is sent on the socket. Add `Origin` and `Host` checks on POST and on the upgrade, because the cookie is ambient and shared across ports on that IP.
  - Subprotocol: echo the offered value in the 101. The token sits in `localStorage`, which ITP deletes after 7 days unused, so the browser re-pairs.
- **Should a cookie-authenticated browser reach every endpoint, or only the page's subset?**
  - Every endpoint: no new allowlist.
  - Subset: a second allowlist keyed on how the request authenticated.
- **Decision 3: keep the JSON invite?**
  - Drop it: change the four JSON parsers listed under Settled to read the URL; the URL carries one address.
  - Keep it: those parsers accept both forms.
- **Which address goes in the URL when the Mac has several, and should an IPv6 address be bracketed?**
  - One address: the routed one from `local_address()`, written `[addr]:port` for IPv6.
  - A `.local` name: depends on the unresolved `.local` question.
- **Decision 4: static paths and headers.** The exact set; whether unknown non-API paths 404; `Cache-Control: no-store` or versioned paths; `frame-ancestors` and the rest as headers (`<meta>` cannot carry `frame-ancestors`). Serve on the connection thread or through the frame? Which pool and rate window does a page load use?
  - Connection thread: the frame never sees static loads.
  - Frame: as today.
- **What does the page show with neither a code nor a token?**
  - `/info` data: the host name and `pairing_open`, with "scan the QR".
  - Static text only.
- **Decision 6: device name from `User-Agent` on the host, or `navigator.userAgent` on the page? Add a `kind` field to `PairedDevice`?**
  - Add `kind`: an additive `#[serde(default)]` field, no version needed (see Settled).
  - No `kind`: the name alone tells devices apart.
- **The card excerpt on the page.**
  - A `SessionView` field: compute `agent_excerpt`'s rule on the host, update the fixtures and the fingerprint.
  - A `last-response` read per session: N reads per list.
  - Drop it.
- **The device's scope after a reload.**
  - A tiny who-am-I endpoint: a new read model.
  - Page storage: subject to ITP's 7-day deletion.
  - A `403 scope`: the reply box shows until a send fails.
- **Telling revoked from identity-changed.**
  - Distinct 401 codes: a wire change.
  - The page keeps the server id and key per origin: storage subject to ITP.
- **What the session view shows for a stopped session that is not Claude Code (screen is refused).**
  - Last run output.
  - A note saying there is nothing to show.
- **In this cut: plain `send` or drafts, Resume and Trust on the page, full §4.4 subscribe-then-snapshot, `watch`/`unwatch`, the 30 s refetch.**
  - Yes: the page mirrors the headset.
  - No: the page reads only `changed`.
- **Advertise a `web` capability in `/info` or TXT?**
  - Yes: an additive field.
  - No: nothing changes.
- **Acceptance on a real phone:** scan, click through, see sessions, watch a session move to waiting, reply, revoke. Needs a device on the same network and the signed bundle.
