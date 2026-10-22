You are scoring one implementation plan for a GitHub issue, blind: you do not know who or what wrote it, and you score this plan on its own.
The repository at the issue's base commit is checked out at /Users/sully/code_repos/plan-trial/sb-230-base (read-only; change nothing there). Every file path in the plan is relative to that tree.

Score by the rubric, column by column. Length is not thoroughness. For grounding, take the first five distinct file or line references the plan makes, in order of appearance, open each in the tree, and say what you found. For departures and decisions, use the reference as the yardstick; where you disagree with the reference, say so in the evidence rather than silently scoring your own view.

Your final message must be exactly this shape and nothing else:

```
| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| N | <0-3> | <0-3> | <0-3> | <0-3> | <yes|note|no> |
```

## Grounding
- <ref 1 as the plan wrote it>: found / wrong (<what is there>)
- ... five rows
## Departures caught
- <each departure in the reference>: caught / missed / caught with a wrong fix
- <any departure the plan found that the reference lacks>
## Decisions to overturn
- <each call you would reverse, with the reference decision it conflicts with>; or none
## Over-specification
- <what an implementer would not need, and roughly what fraction of the plan it is>
## Hand-off
- <the one line of guidance if note; why not if no>
## Notes
- best: <one line>
- worst: <one line>

---

# The issue

# A web control surface: the remote port serves a browser page, paired from a URL that carries the code

## What I want

A second control surface beside the headset app: a page the host serves itself, opened in any browser on the local network (a phone, an iPad, another Mac), that lists the sessions with the waiting ones marked, shows a session's last response or screen, and sends a reply. It runs over the remote port that already exists. Nothing to install on the device, no App Store, no Xcode.

## What exists

- The remote port (`docs/remote-client.md`; `src/adapters/remote.rs`; `serve_remote` in `src/app.rs`): TLS with a per-installation self-signed certificate, pairing by a code with a pinned key, bearer tokens, the `/api/v1/*` read and control endpoints, the `/api/v1/events` WebSocket with the revision counter and `changed` events, redaction before every cut. Any path outside `/api/v1/` is refused.
- The pairing window (`src/ui/remote.rs`): a QR carrying `{server_id, spki_sha256, code, addresses}` as JSON, the same in text beside it, open for five minutes or three wrong codes. The headset pairs from the Bonjour record and a typed code; the command-line `switchboard-remote pair` takes the QR's text as one argument.
- Three speakers of the protocol already: the visionOS app, `switchboard-remote`, and the fixtures shared by `tests/remote.rs` and the Swift tests.

## The shape

- **The host serves the page from the same port.** `GET /` and the page's few files answer with HTML, CSS and JS embedded in the binary at compile time (`include_str!`), so there is no build step, no bundler and no package manager. Same origin as the API: the page calls `/api/v1/...` and opens `/api/v1/events` directly.
- **The page speaks the protocol the headset speaks.** REST for reads and commands, the events socket for live changes, replies applied by revision the way §4.4 says. No new read model unless the page cannot do without one.
- **Pairing from a URL.** The pairing window's QR becomes a URL, `https://<address>:<port>/?code=<code>&key=<spki_sha256>&id=<server_id>`, so a phone's camera opens it in the browser. The page reads the query, removes it from the address bar with `history.replaceState` before anything else happens, compares the key in the URL with the one `/info` presents, posts `/api/v1/pair` only when they match, keeps the token, and shows the sessions. The code never stays in the address bar or the history. The native text path (`switchboard-remote pair`, the headset's paste) takes the URL too, or the JSON as before.
- **The first cut of the page.** Sessions grouped by project, waiting ones first and marked, each with its kind, state and the one-line excerpt the card shows; a session view with the last response for a Claude Code session or the screen for anything else, refreshed on `changed`; a reply box that posts `send` when the device has `control` scope and is hidden otherwise; the device's scope and host name in the header; "identity changed, pair again" and "revoked" states said plainly.

## Decisions the plan must settle

1. **Browser trust in the self-signed certificate.** The native client pins the SPKI; a browser cannot. The first visit shows a certificate warning the owner clicks through, and the browser then trusts that certificate for that address. The plan says what this buys and what it does not: the `key` in the URL lets a genuine page refuse to send the code to a host whose `/info` key differs, but a page served by an impostor is not a genuine page. Either the click-through on the local network is the trust step, said in the paired-devices dialog, or the host offers its certificate for download so the owner can trust it on the device first. Pick one, say why, and keep §5's rule that no non-loopback bind serves anything but `/info`, `/pair` and now the page itself before pairing.
2. **How the token reaches the WebSocket.** A browser cannot set a header on `new WebSocket()`, and the spec forbids the token in a URL. Candidates: the `/pair` reply sets an `HttpOnly; Secure; SameSite=Strict` cookie that `fetch` and the WebSocket both carry and the page's script never sees, or the token travels as a `Sec-WebSocket-Protocol` value. The host keeps accepting the bearer header for the native clients. The choice also decides where the token lives on the device; `localStorage` is readable by any script on the origin, a cookie is not.
3. **One QR or two.** One URL read by the browser and by the native text path is the preference; the headset has no camera and the CLI parses text, so the JSON form can be kept only if something still needs it.
4. **Static paths on a port whose allowlist is `/api/v1/*`.** The exact set of paths, their `Content-Type`, a `Content-Security-Policy` with no inline script and no external resource, no caching across versions, which pool and rate window an unpaired browser's page load counts against (§4.1 allows four unauthenticated connections), and what the page shows when opened without a code and without a token.
5. **Where the page lives and how it is tested.** `web/` embedded at compile time; a test in `tests/remote.rs` that the embedded page references only paths the host serves and that the new paths answer with the headers the plan names; the pairing flow tested end to end with the host's own HTTP client as the CLI does, since no UI test drives a browser; the fixture set extended if a view changes.
6. **The device record.** A browser is a paired device like any other: a name from the user agent (`Safari on iPhone`), scope chosen in the pairing window as today, listed and revocable in the paired-devices dialog.

## Not in this cut

- Dispatch tickets and decisions on the page (the API serves them; a later issue).
- The conversation beyond the last response, drafts, dictation, runs and pinned files (the API serves them; later).
- Anything beyond the local network: no relay, no tunnel; §9 of the spec stands.
- A framework, a build step or a dependency for the page.

## Where to look

- `docs/remote-client.md` §4.1 (bounds), §4.3 (endpoints), §5 (pairing): the plan amends the spec and the remote status section of `docs/design.md`.
- `src/adapters/remote.rs`: `connection`, `read_request`, the pools and the rate windows.
- `src/app.rs` `serve_remote`: the `/api/v1/` prefix match and the allowlist.
- `src/ui/remote.rs`: the pairing window and its QR payload; `src/core/remote.rs` the pairing state.
- `client/Sources/switchboard-remote/main.swift` and `client/Sources/SwitchboardProtocol/Info.swift`: the text pairing path that has to keep working.

## Acceptance

Open the pairing window, scan the QR with a phone, accept the certificate warning, see the sessions; a session that starts waiting moves to the top without a reload; send a reply from the phone and watch it arrive; revoke the phone in the dialog and the page says so on its next request.


---

# The rubric

# Scoring a plan

Score blind: the twelve plans of an issue as A to L, with the key closed
until every row is filled. Length reads as thoroughness, so score each
column on its own and never the whole impression.

Each column is 0 to 3 except the last.

- **Grounding.** Open five file or line references the plan makes, in a
  worktree at the issue's base commit (`results/<short>-<issue>/base.txt`).
  3: all real and relevant. 2: one wrong. 1: several wrong. 0: the plan
  cites nothing checkable, or mostly misses.
- **Departures caught.** Where the issue's claims do not match the code,
  did the plan say so and say what it does instead? 3: every departure,
  including ones you had not noticed. 0: it planned the issue as written
  where the code says otherwise.
- **Decisions to overturn.** Count the calls you would reverse before
  implementing. 3: none. 2: one. 1: two. 0: three or more.
- **Over-specification.** How much of the plan an implementer would not
  need: restated code, sections with nothing to say, steps the test
  suite would force anyway. 3: nothing to cut. 0: half or more.
- **Hand-off.** Would you give this plan to an implementer as it is?
  `yes`, `note` (with one line of guidance), `no`.
- **Notes.** One line: the plan's one best and one worst thing.

After the key is opened, put beside each arm's scores what the run
measured: plan review rounds and whether the cap was hit (from
`report.json`), wall time, tokens by model, and for a single session the
`total_cost_usd` and `duration_ms` in `run.json`. Two repetitions per
arm: note where they disagree, since that is the consistency result.

Write the result per issue as the table and a paragraph of judgment, not
an average over two issues.

---

# The reference (owner-checked)

# Reference: Switchboard #230 at 7fe8949

Written from the code at the base commit before any plan was read. It is the grader's yardstick for the five rubric columns, not a plan.

## What the code does

- `serve_remote` (`src/app.rs:3017`) strips `/api/v1/`; any other path is `404 not_found` with `authenticated: false`, decided on the frame. Only `info` and `pair` run before `authenticate` (`app.rs:3127`), which reads `request.bearer` and nothing else.
- `read_request` (`src/adapters/remote.rs` ~440-515) keeps method, path, query, body, the `Authorization: Bearer` value and the `Sec-WebSocket-Key`; every other header is dropped. `respond` (`remote.rs:531`) writes every reply as `Content-Type: application/json`; `RemoteReply` (`remote.rs:389`) is status, body and `authenticated`. Nothing can serve a page today.
- Pools: `MAX_UNAUTHENTICATED` 4, `MAX_PAIRED` 12 (`remote.rs:567`); a connection moves pools on its first authenticated reply (`connection`, `remote.rs:770`). Rate windows only on `/pair` and `/info`. Idle timeout 30 s, keep-alive, one request in flight per connection.
- `remote_subscribe` (`app.rs:2723`) needs a device first, so the socket is authenticated by the bearer header; `respond_upgrade` (`remote.rs:519`) echoes no subprotocol.
- `remote_pair` (`app.rs:3606`): inside the window, `spki_sha256` must equal the host's, the code is case-insensitive, `device_name` is the body's (64 chars, `new_device` in `src/core/remote.rs`); the reply is `{token, scope, server_id, device_id}`; the host keeps the token's SHA-256 in `devices.json`. The code is single-use, three failures close the window.
- The pairing window (`src/ui/remote.rs:155-227`) builds `{server_id, spki_sha256, code, addresses: ["ip:port"]}` from `remote::local_address()` (`remote.rs:962`), draws it with the `qrcode` crate and "Copy invite" copies the JSON. `PairingInvite.parse` (`client/Sources/SwitchboardProtocol/Info.swift:114`) decodes JSON only; the CLI takes it as one argument (`switchboard-remote/main.swift:12-13`); the headset pairs from Bonjour and a typed code (`docs/design.md:3562`). `tests/ui.rs:7474` asserts the copied invite parses as that JSON.
- The certificate (`make_identity`, `remote.rs:95`): P-256, self-signed, no SAN, no EKU, CN `Switchboard <id>`, expiry in 4096. Clients pin the SPKI (`pinned_client`, `remote.rs:168`); the identity model deliberately binds nothing to an address or a validity window (spec §5, §9).
- Embedding precedent: `include_bytes!` for the icon and fonts (`src/main.rs:167`, `src/ui/theme.rs:203`). `tests/remote.rs` has its own TLS client (`http`, line 245) that reads only the status and `Content-Length`; `pair` (406) and `subscribe_at` (1960) are the helpers. Dev aids: `SWITCHBOARD_REMOTE_PORT`, script lines `pairing <code>` and `remote-listen lan <port>` (`src/script.rs:729-746`, README ~267-280).

## Departures from the issue's framing

1. **The WebSocket will not open from Safari.** WebKit refuses `wss://` to a certificate the user only clicked through, on iPhone, iPad and Apple-silicon Macs, a limitation open since 2011 (WebKit bug 41419; Cockpit documents the same), and today's certificate also fails Apple's rules for a trusted certificate (SAN, `serverAuth` EKU, 825 days). The page therefore loads after the click-through but `/api/v1/events` does not. The issue assumes the socket. A plan must say this and make the page work without it: poll the collections and the open session's content on a short interval, use the socket when it opens (Chrome, Firefox, Android). A plan that ships the socket alone has planned a page that is static on the device it is for.
2. **"Offer the certificate for download" is not an option with this certificate.** Trusting it on the device needs a reissue with the address as a SAN, the EKU and a short validity, which ties the identity to an address and an expiry the spec avoids, and Safari still refuses `wss://` by Cockpit's report. The choice in the issue collapses: the click-through is the trust step. The key in the URL keeps its value against a relayed code.
3. **Each file of the page is a connection.** A browser fetches subresources on parallel keep-alive connections that each hold an unauthenticated slot for up to 30 s. The issue frames this as "which pool"; the answer is that the page should be at most two files (HTML and script), and the plan must say what the pool leaves for a second device.
4. **The device name is the page's to send.** The host drops every header but two, so the "name from the user agent" is the page's script filling `device_name` from `navigator.userAgent`, as every client fills it; no host-side parsing.

## Decisions, with the answer I would take

1. **Trust.** The click-through on the local network is the trust step; the paired-devices dialog and spec §5 say so. No certificate download, no reissue. The page compares the URL's key with `/info` before posting the code.
2. **Token.** `/pair` sets an `HttpOnly; Secure; SameSite=Strict; Path=/` cookie named with the server id (cookies ignore ports, and two instances on one Mac would otherwise overwrite each other's) when the pair body asks for it; `read_request` fills `bearer` from that cookie when the header is absent, so `authenticate` and every handler are untouched and the native clients keep the header. `SameSite=Strict` is the CSRF defence and the plan must name it; an `Origin` check is fine, a CSRF token is over-specification. `localStorage` for the token is the call I would reverse.
3. **Live updates.** Poll first, socket second (departure 1). Over the socket the page sends `watch` for the open session's `conversation` or `screen` as §4.4 says, else `changed` never arrives for content; polling refetches the same reads.
4. **One QR, a URL.** `https://<address>:<port>/?code=…&key=…&id=…`; `addresses` is the authority. "Copy invite" copies the URL; `PairingInvite.parse` accepts the URL and the JSON; the UI test at 7474 changes with it. The page clears the query with `history.replaceState` before any fetch.
5. **Static paths.** `/` and `/app.js` (CSS inline under a hash or in the HTML), answered on the connection thread as constants with `Content-Type`, `Cache-Control: no-store`, `X-Content-Type-Options: nosniff` and a CSP of `default-src 'none'; script-src 'self'; style-src 'self'` (or the hash) `; connect-src 'self'`. Nothing else outside `/api/v1/` changes. Raising `MAX_UNAUTHENTICATED` to 8 is acceptable; a frame-side route for a constant file is the call I would reverse.
6. **The page.** `web/` with two files embedded by `include_str!`, plain script, no dependency, no build. First cut as the issue says: projects and their sessions, waiting first, the session's last response or screen, a send box under `control`, scope kept from the pair reply. Tickets, the conversation, drafts and files stay out.
7. **Tests and docs.** In `tests/remote.rs`: the static paths and their headers (`http` learns to return headers), a pair that sets the cookie, a cookie-authenticated read and refusal after revoke, the embedded page's paths all served; in `tests/ui.rs` the invite as a URL; a Swift test for the URL form of `parse`. Spec §4.3 rows and a §5 paragraph, a dated entry in design.md's remote section, a `web/` row and the dev-aid sentence in the README, CHANGELOG under Unreleased. No new Rust or JS dependency.

## Anchors a grounded plan should cite

`src/app.rs:3017` (`serve_remote`), `3127` (`authenticate`), `3606` (`remote_pair`), `3568` (`remote_info`), `2723` (`remote_subscribe`); `src/adapters/remote.rs:389` (`RemoteReply`), `~440-515` (`read_request`), `531` (`respond`), `567-568` (pools), `770` (`connection`), `95` (`make_identity`), `962` (`local_address`); `src/ui/remote.rs:155-227` (the payload and Copy invite); `src/core/remote.rs` (`new_device`, `device_by_token`); `src/ports/remote.rs:96` (`PairedDevice`); `tests/remote.rs:245` (`http`), `406` (`pair`), `1960` (`subscribe_at`); `tests/ui.rs:7424-7495`; `client/Sources/SwitchboardProtocol/Info.swift:94-116`; `docs/remote-client.md` §4.1, §4.3, §4.4, §5; `docs/design.md:3107` (remote section), `3573` (known gaps); `README.md` ~267-280.

## Errata, after grading

Eleven of the twelve plans found a departure this reference missed: the card's one-line excerpt is not on the wire. `SessionView` (`control/src/lib.rs:637`) has `card`, `waiting` and `waiting_reason` but no `last_message`; the spec's sentence at `docs/remote-client.md:207` describes a field that was never added, and `agent_excerpt` (`src/ui/cards.rs:558`) computes the excerpt UI-side. Decision 6's "as the issue says" therefore asked for a field the page cannot read; a plan that reads `last-response` for the waiting sessions, shows `card` and `waiting_reason`, or defers the excerpt was right to depart, and the Codex grader's "restore the excerpt" reversals should be read with that in mind (no score changed: by its own count every plan kept three or more other reversals). Several plans also found that nothing tells a device its scope after a reload behind a cookie (§5's "answers with the device's scope" is true only of the pair reply), that the invite carries a LAN address while the host listens on loopback, and that the pair reply would carry the token in its body even with a cookie. All verified by the graders; a plan that handled them earned it under departures.

---

# The plan (N)

# Plan for #230: the browser page on the remote port

Worktree at 7fe8949. Line references are to this tree. `SCHEMA_VERSION`
is 16 (`src/core/model.rs:12`) and `IDENTITY_SCHEMA_VERSION` is 1
(`src/ports/remote.rs:17`). Neither moves: the one serialized change is
an additive `#[serde(default)]` field on `Identity`, the way `listen`
was added under version 1 (`src/ports/remote.rs:39-40`); see The
record. "Facts" is `facts/1/repo/facts.md`.

## Where this departs from the issue

- **The code travels in the fragment, not the query.** The QR is
  `https://<addr>:<port>/#code=<code>&key=<spki_sha256>&id=<server_id>`.
  A browser never sends a fragment to the server, so the code never
  reaches a request target, `RemoteRequest::query` or a `Referer`. It
  reaches the host only in the `/pair` body. That makes the notes'
  questions about logging the query, refusing a `GET /?code=`, and
  `Referer` leakage go away. This rests on RFC 3986 §3.5 (the user agent
  handles the fragment). That is not in the facts file, and
  `tests/remote.rs` cannot show it, because the host's client is not a
  browser. The page still calls `history.replaceState(null, "", "/")`
  before anything else, but facts list whether Safari or the camera
  records the URL first as Unresolved (facts:46). The fragment does not
  change that, and the plan accepts it: the code is single-use, and the
  window closes on the first pairing (`src/core/remote.rs:64`).
- **The card's one-line excerpt is not on the wire.** `agent_excerpt`
  derives it in the UI from the parsed conversation (facts:17,
  `src/ui/cards.rs:558-574`), and `SessionView` has no such field
  (`control/src/lib.rs:637-688`). The page's list shows
  `waiting_reason` when the session is waiting, otherwise the card word
  (`card`), plus the first line of `notes`. Facts leave the excerpt to
  the owner (facts:83). If the owner wants it, the fallback is an
  additive `#[serde(default)] excerpt: Option<String>` on `SessionView`,
  computed on the host by the `agent_excerpt` rule. That means a
  fixture rewrite and a `revision.rs` fingerprint change, so it is a
  separate commit.
- **The current certificate is not one a phone is sure to accept.** It
  has no SAN, validity to 4096, and no serverAuth EKU (facts:13, :33).
  The host issues a second, served leaf over the same Keychain key,
  keeps it in `identity.json`, and reissues it only when its address no
  longer matches or its expiry is within 30 days (details under The
  record and The signals). Pins survive (facts:12, spec §5 "a
  certificate reissued from the same key"). A browser's acceptance of
  the served leaf survives an app restart, because the leaf is the same
  bytes. Each issue has its own random serial, so no two certificates
  share issuer and serial (RFC 5280 §4.1.2.2; rcgen would otherwise
  derive the serial from the key, `rcgen-0.14.10/src/certificate.rs:458-468`).
- **"Revoked" is told apart by the page, not the host.** A revoked
  token and an unknown token get the same 401 (`src/app.rs:3142-3148`).
  The page keeps `{server_id, spki_sha256}` per origin in
  `localStorage`. On a 401 it says "revoked" when `/info` still shows
  that key, "identity changed, pair again" when the key differs, and
  "not paired" when it kept nothing. Facts leave this to the owner
  (facts:91). If the owner says otherwise, the fallback is a distinct
  401 code, which needs a record of revoked token hashes. Safari wipes
  script storage after 7 days without a visit (facts:32), so after that
  a revoked browser hears "not paired, scan the QR". That is still the
  right instruction.
- **"The screen for anything else" has no screen when the session is
  stopped.** `screen` refuses a session that is not running (spec
  §4.3). The page says "Not running; nothing to show". Facts leave this
  to the owner (facts:94). If the owner wants more, the fallback is
  `runs` then `runs/{n}/output` for a command or service.
- **The cookie is not scoped by port** (facts:28). A cookie set by
  `https://<ip>:43187` is sent to every port on that address. So the
  cookie name carries the port. Every cookie-authenticated request also
  passes an origin check, because `SameSite=Strict` does not stop
  another HTTPS service on the same IP: that service is same-site.

## Decisions

1. **Trust: the click-through is the trust step, and a browser pairs
   `read` only.** This is the owner's decision (facts:56-58), not a
   settled line, and it weakens two guarantees of §5:
   - "Pairing verifies the key before the code is sent"
     (`docs/remote-client.md:309`). A browser cannot. An impostor that
     the owner clicks through serves its own page, reads the fragment
     and relays the code with the real key. The URL's `key` then only
     catches a QR for another host or a stale QR: a genuine page
     compares it with `/info`'s self-reported `spki_sha256` and posts
     nothing on a mismatch.
   - "after pairing, every connection verifies the pin before the token
     is presented" (same line), and the token "is sent only as a bearer
     header over the pinned connection" (`:312`). A browser sends its
     cookie to whatever certificate it has accepted for the origin.

   What the plan builds against that: a stable served leaf (The
   record), so a second warning on a known origin is
   a sign of trouble rather than routine; and **a browser pairing is
   granted `read` whatever the window's scope.** The owner raises it to
   `control` by hand in the devices dialog, which §5 already allows
   (`docs/remote-client.md:314`). A relayed code therefore yields a
   reader, never a device that types into agent sessions, unless the
   owner raises it. The host tells a browser pairing by the `/pair`
   body's `"cookie": true` (decision 2). The dialog says what the
   click-through trusts in one sentence, and the hover on a browser's
   scope combo says what raising it trusts.

   §5's lines 309, 312 and 317 are rewritten to say the native
   guarantee and the browser's weaker one side by side, not added to
   (Docs).

   **If the owner rejects the click-through as the trust step**, the
   fallback the implementer builds is facts:58: the host serves its
   certificate before pairing at `GET /certificate`
   (`application/x-x509-ca-cert`), issued with basicConstraints CA:TRUE
   plus the served leaf's SAN, EKU and ≤825-day validity (facts:33,
   :34). The dialog gives the Settings > General > About > Certificate
   Trust Settings steps (facts:34), the pre-pairing rule names a fourth
   path, and browsers keep the window's scope. Without that answer the
   build is the read cap above.

   Separately, whether iOS Safari opens `wss://` after the click-through
   is Unresolved (facts:38, owner item facts:53). **The build does not
   depend on that answer.** The page falls back on its own to polling
   REST every 5 s when the socket fails before `hello`. Spike 23
   records it (Tests).
2. **Token: an `HttpOnly; Secure; SameSite=Strict; Path=/` cookie, no
   `Domain`, `Max-Age=34560000`.** The page's script never sees it. It
   survives Safari's 7-day storage cap where `localStorage` does not
   (facts:32). The same-origin WebSocket handshake sends it (facts:25).
   Name: `switchboard-<port>`, so two instances on one Mac never
   overwrite each other's cookie (facts:28). Native clients keep the
   bearer header, which is checked first. The page calls `fetch` in its
   default mode (`cors`) and never sets `mode` or `referrerPolicy` on a
   request: under `Referrer-Policy: no-referrer`, a non-GET request in
   any other mode carries `Origin: null` (Fetch, "append a request
   `Origin` header" step 4.1), which the origin rule refuses. Whether a page reached through
   a certificate override is a secure context, and so keeps a `Secure`
   cookie, is Unresolved (facts:45). If Safari drops the cookie, the
   fallback is the subprotocol route: `/pair` returns the token in the
   body, the page keeps it in `localStorage`, sends it as
   `Authorization` on `fetch` and as the offered `Sec-WebSocket-Protocol`,
   and `respond_upgrade` echoes it (facts:18, :26, :27). Re-pairing
   after 7 days unused is the known cost.
3. **One QR: the URL.** The JSON form leaves the QR and "Copy invite".
   `PairingInvite.parse` (`client/Sources/SwitchboardProtocol/Info.swift:114`)
   accepts the URL and still accepts the JSON, because
   `SWITCHBOARD_INVITE` lines and older pasted invites are JSON. That
   one function covers every caller (facts:14). The URL carries one
   address: the routed address the served leaf names
   (`ServedCertificate::address`, The record), bracketed when it is
   IPv6, so the QR's host and the certificate's SAN cannot disagree.
   When the pairing window opens and `local_address()` differs from
   that address (the Mac changed networks since the bind), the app
   reissues the leaf and rebinds before the QR is drawn (The actions
   and effects). The dialog already shows only one address. Facts leave which
   address to the owner (facts:71). If the owner says otherwise, the
   fallback is a `.local` name, which is Unresolved (facts:49) and not
   built.
4. **Static paths.** `GET /` (`text/html; charset=utf-8`),
   `GET /app.js` (`text/javascript; charset=utf-8`), `GET /app.css`
   (`text/css; charset=utf-8`). Nothing else: `/index.html`,
   `/favicon.ico` and every other non-API path answer 404 `not_found`
   as today. Each of the three carries `Content-Security-Policy:
   default-src 'none'; script-src 'self'; style-src 'self'; connect-src
   'self'; img-src 'self'; base-uri 'none'; form-action 'none';
   frame-ancestors 'none'` as a header. `connect-src 'self'` covers the
   same-origin `wss:` (facts:30), and `frame-ancestors` must be a
   header (facts:31). They also carry `Cache-Control: no-store`,
   `X-Content-Type-Options: nosniff` and `Referrer-Policy: no-referrer`.
   Every JSON reply also gains `Cache-Control: no-store` and `nosniff`.
   `serve_remote` answers all three on the frame, before the `/api/v1/`
   strip. When the request carries a `switchboard-<port>` cookie that
   `device_by_token` knows, the answer is `authenticated: true` (no
   `DeviceSeen`, no origin rule: a GET of public bytes has no effect),
   so the connection moves to the paired pool as an API request's does
   (`src/adapters/remote.rs:835-848`). Otherwise a paired phone's
   reload would leave connections that fetched only `/app.css` in the
   unauthenticated pool for the 30 s idle timeout (`:573`, set at
   `:785`), and the accept loop drops a fifth (`:701-705`): the phone's
   own `/device` and anyone's `/info` would be locked out for 30 s. An
   unknown or absent cookie still gets the page, unauthenticated. Facts
   leave the thread to the owner (facts:74); the frame is chosen
   because it has to serve the cookie case anyway, and one path is
   simpler than two. Static loads count in a new `page` window of 60 a
   minute per listener, shared by the three paths and checked on the
   connection thread with the others, which bounds the frame's load:
   three files per unpaired visit against 8 drains per pass. How many connections iOS Safari opens at
   once is Unresolved (facts:48). An unpaired load is three files and
   `/info`, so at most four in the stranger pool, each held up to 30 s
   idle. Spike 23 counts them; if an unpaired load or a pairing right
   after it is dropped, the fallback is `MAX_UNAUTHENTICATED` 4 → 8 and
   the §4.1 line with it. §5's rule becomes: before pairing,
   nothing but `/info`, `/pair` and the page's three files, which carry
   nothing of the host. With neither a fragment nor a cookie, the page
   shows `/info`'s `server_name`, whether pairing is open, and "On the
   Mac, open Remote devices, allow pairing and scan the QR" (the owner
   question at facts:77; the fallback is static text alone).
5. **Where the page lives and how it is tested.** `web/index.html`,
   `web/app.js` and `web/app.css` are embedded with `include_str!`
   (the precedent for embedding is `include_bytes!` at
   `src/main.rs:167`; the tree has no `include_str!` yet). The test checks a fixed list
   of referenced paths. There is no HTML parser (facts:21) and no JS
   test harness, since the page has no build step and no dependency.
6. **Device record.** The page sends `device_name` from
   `navigator.userAgent`: "Safari on iPhone", "Safari on Mac", "Chrome
   on Android", and so on, else "Browser". Safari on iPadOS sends the
   Mac user agent by default
   (https://developer.apple.com/forums/thread/119186), so a Mac user
   agent with `navigator.maxTouchPoints > 1` is named "Safari on iPad".
   A heuristic: a Mac with a touch screen would read as an iPad, and
   the owner sees the name only in the devices list.
   `/pair` already takes it (`src/app.rs:3650`) and caps it at 64
   characters (`src/core/remote.rs:166`). Scope is `read` for a browser
   whatever the window says (decision 1). The device is listed and revocable as any other. There is no
   `kind` field; the name tells devices apart. Facts leave this to the
   owner (facts:80). If the owner wants one, the fallback is an additive
   `#[serde(default)] kind` on `PairedDevice`, with no version
   (facts:16).
7. **The page's scope after a reload: `GET /api/v1/device`.** It
   answers `{device_id, name, scope}` under any scope. The page cannot
   do without it: the cookie is unreadable, and storage is wiped after
   7 days. It is also the request the page uses to learn it was
   revoked. Facts leave this to the owner (facts:87). If the owner says
   no, the fallback is to keep the scope from the `/pair` reply in
   `localStorage`, show the reply box when the scope is unknown, and
   hide it on a `403 scope`.
8. **The cookie reaches every endpoint the bearer does.** There is no
   second allowlist, because scope is the authority and a browser is a
   paired device like any other. Facts leave this to the owner
   (facts:65). If the owner says otherwise, the fallback is a match on
   how the request authenticated, limited to the page's paths.

## The record

`devices.json`: no change. A browser is a `PairedDevice`; the token's
hash is the same whether the token travels as a bearer header or as a
cookie.

`identity.json` gains the served leaf, so a browser's acceptance
survives a restart. `Identity` (`src/ports/remote.rs:26`) gets an
additive `#[serde(default)] pub served: Option<ServedCertificate>`,
under `IDENTITY_SCHEMA_VERSION` unchanged, as `listen` was
(`:39-40`). It is a field of the existing file rather than a new one
because it is written through the same `Effect::SaveIdentity` and
`RemoteStore::save_identity` path, so no port method is added.

```rust
/// The certificate the port serves: over the identity's key, naming
/// where a browser reaches the host. Reissued only when `address` no
/// longer matches or `not_after` is near.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServedCertificate {
    /// DER, base64.
    pub certificate: String,
    /// The routed address among its SANs, which the QR uses.
    pub address: Option<IpAddr>,
    pub not_after: SystemTime,
}
```

`certificate` in `Identity` stays as it is (the identity's own record
and what `make_identity` checks against the key); `tls_config` serves
`served` when present. A key rotation makes a new identity with
`served: None`. An older build reads the file and ignores the field
(`Identity` has no `deny_unknown_fields`); if it saves the identity, the
field is dropped and the next newer launch issues a fresh leaf, which a
browser meets as one more warning.

A change of the Mac's address is a new origin, so the browser pairs
again from a fresh QR. Write that under Known gaps.

## The signals

These change in `src/adapters/remote.rs`.

- `read_request` (:444) keeps three more headers on `RemoteRequest`
  (:360). `cookie: Option<String>` is only the value of the cookie
  named `switchboard-<port>`; other cookies are dropped. `origin:
  Option<String>` and `host: Option<String>` are the raw values.
  `page: bool` is true when `X-Switchboard-Page: 1` is present. The
  adapter learns its port from `Shared`, which gains `port: u16`,
  filled from `listener.local_addr()` in `bind` (:673).
- `Limits::limited` (:629) gains the `page` window for `/`, `/app.js`
  and `/app.css`; `connection` (:770) otherwise hands them to the frame
  like any request (decision 4).
- `respond` (:530) takes a content type and extra header lines.
  `RemoteReply` (:389) gains `cookie: Option<String>`, the token to set
  as `Set-Cookie`, and `content_type: &'static str`, so the frame's
  static answers carry their own type and headers.
- **The served leaf.** A new `pub fn serving_certificate(identity,
  key_pem, address: Option<IpAddr>, now: SystemTime) ->
  Result<ServedCertificate, String>` makes the leaf over the Keychain
  key with `rcgen::KeyPair::from_pem` (facts:12). It keeps the CN of
  `make_identity`. The SANs are `localhost`, `127.0.0.1`, `::1` and the
  routed address; rcgen turns an IP string into `SanType::IpAddress`
  (facts:13). It sets `extended_key_usages = [ServerAuth]`,
  `not_before = now − 1 day`, `not_after = now + 800 days` (under
  Apple's 825, facts:33), and `serial_number =
  Some(SerialNumber::from_slice(&r))` with `r` 16 bytes from
  `fill_random` and the top bit cleared so the integer is positive
  (`rcgen-0.14.10/src/lib.rs:878`). `time` is already in the tree at
  0.3.55 behind rcgen. Add it as a direct dependency (`time = "0.3"`)
  for `OffsetDateTime::from(SystemTime)`, and commit the updated
  `Cargo.lock`.
- `tls_config` (:144) serves `identity.served` when it is present and
  `identity.certificate` otherwise. Its signature does not change.
- The rule for when to reissue is a pure function in
  `src/core/remote.rs`: `pub fn served_is_current(served:
  Option<&ServedCertificate>, address: Option<IpAddr>, now: SystemTime)
  -> bool`, true when `served` exists, names `address`, and expires more
  than 30 days after `now`. If the owner answers "no" on naming the IP
  (facts:59), the SANs drop the routed address and `address` is not
  compared; the QR then takes `local_address()` itself.

## The actions and effects

One new action, `AppAction::ServedCertificateMade(ServedCertificate)`:
the core stores it on the identity through `update_identity`
(`src/core/remote.rs:105`), which emits the existing
`Effect::SaveIdentity`. No new `Effect`. `Paired` and `RevokeDevice`
carry a browser unchanged. The frame changes in `src/app.rs`:

- `bind_remote` (:2936) reads `local_address()` once. When
  `served_is_current` is false it calls `remote::serving_certificate`
  with the Keychain key it already holds (:2951-2956), dispatches
  `ServedCertificateMade`, and builds `tls_config` from the updated
  identity. A failure is logged and the port serves
  `identity.certificate` as today.
- The core routes `ServedCertificateMade` with the other remote
  actions (`src/core/action.rs:1257`).
- In `SwitchboardApp::dispatch`, after an `AppAction::OpenPairing`, the
  app compares
  `local_address()` with `identity.served.address`. When they differ it
  calls `bind_remote` again, which reissues and rebinds; the QR is
  drawn from the new `served.address` on the next frame. Existing
  connections drop with the old listener, as a `SetRemoteListen`
  rebind does today. The address lookup goes through a field
  `address_source: fn() -> Option<IpAddr>` on `SwitchboardApp`,
  `remote::local_address` by default and set by a test-only
  `set_address_source`, beside `set_remote_port_override`
  (`src/app.rs:2920`).

- `serve_remote` (:3017) routes `("GET", ["device"])` after
  `authenticate` to a new `remote_device`, which returns `{device_id,
  name, scope}` in the envelope.
- `authenticate` (:3127) checks `request.bearer` first, unchanged. Then
  it checks `request.cookie`. A cookie-authenticated request passes only
  when `origin` is absent or equals `https://{host}`, and either it is
  an upgrade whose `origin` equals `https://{host}`, or it carries
  `page`. Otherwise the reply is 403 `cross_origin`, "this request did
  not come from the host's page". The reasoning: a browser always sends
  `Origin` on a WebSocket handshake (RFC 6455 §4.1). A custom request
  header forces a CORS preflight, which the host never answers (Fetch
  standard). A same-origin GET in `cors` mode carries no `Origin`, so
  "absent" is accepted; `Origin: null` is not `https://{host}` and is
  refused. The tests prove the host's side of the rule only.
- `serve_remote` answers `GET /`, `/app.js` and `/app.css` before the
  `/api/v1/` strip: the bytes with their headers, and `authenticated`
  set when `device_by_token` knows the cookie.
- `remote_pair` (:3606): when the body has `"cookie": true`, the
  request must carry `page`, else 403 `cross_origin`. The device is
  made with `Scope::Read` whatever the window's scope (decision 1):
  `new_device` (`src/core/remote.rs:166`) gains no parameter; the app
  sets `device.scope = Scope::Read` before dispatching `Paired`. The
  reply's `scope` says `read`, omits `token`, and sets
  `cookie: Some(token)`. Without `cookie` it is unchanged.
- The invite moves out of `src/ui/remote.rs:160-166` into a pure
  function in `src/core/remote.rs`: `pub fn invite_url(identity:
  &Identity, code: &str, port: u16) -> Option<String>`. The host is
  `identity.served`'s `address`, so the QR names exactly the address
  the served leaf carries; it returns `None` when there is none.
  `SocketAddr::new(address, port)`'s `Display` brackets IPv6.

## The UI

- **The pairing window** (`src/ui/remote.rs:141`) draws its QR from
  `invite_url`, so `remote::local_address()` leaves the dialog (:157)
  and the frame does no lookup. The QR cache is keyed by the URL, not
  the code (:168-172). "Copy invite" copies
  the URL. A new line under the code says: "Scan with a phone's camera
  to open the page in its browser. The browser warns that the
  certificate is not trusted. Accepting it on a network you trust is
  the trust step. The key in the code stops the page pairing with
  another host by mistake, not an impostor. A browser pairs read-only;
  raise it below once it is yours." The stale hover on Control (:218)
  becomes "Reads, and sending replies into a session. A browser still
  pairs read-only."
- **The devices list** (`src/ui/remote.rs:230-276`): the scope combo's
  hover says "Control lets this device type into sessions. For a
  browser, raise it only if you paired it yourself, just now, and saw
  no second certificate warning."
- **The page** (`web/`, plain JS, no framework):
  - **Load.** Read `location.hash`, call
    `history.replaceState(null, "", "/")`, then `GET /api/v1/info`. If
    the fragment's `key` and `id` match `/info`, post `/pair` with
    `{code, spki_sha256: key, device_name, cookie: true}`; on a
    mismatch, say "This code is for another host" and post nothing.
    Keep `{server_id, spki_sha256}` in storage. Every `fetch` carries
    `X-Switchboard-Page: 1`.
  - **Header.** `server_name`, plus the device's name and scope from
    `/device`.
  - **Live data.** Open `wss://${location.host}/api/v1/events`. After
    `hello`, snapshot `projects`, each project's `sessions` and
    `waiting`. Apply buffered and later `changed` events by per-key
    revision as §4.4 says: refetch the resource the event names when
    its revision is past what the page holds. On `resync_required` or a
    close, reconnect and resync. Do the same on `visibilitychange` to
    visible (facts:47). If the socket fails before `hello`, poll the
    same reads every 5 s.
  - **The list.** Projects with a waiting session come first. Within a
    project, waiting sessions come first and are marked. Each row shows
    kind, liveness and the line from the departure above.
  - **The session view.** A Claude Code session shows `last-response`.
    A running session of any other kind shows `screen`, after a `watch`
    frame and its `watching` answer, and sends `unwatch` on leave.
    Refetch on the key's `changed` and every 30 s while visible, as
    §4.4 says. A stopped non-Claude session shows the note.
  - **Reply.** Shown only under `control`. It posts
    `sessions/{id}/send` with `{op, text}`, where `op` is 16 random
    bytes from `crypto.getRandomValues` as hex. `randomUUID` needs a
    secure context, which facts list as Unresolved (facts:45). A `403
    scope` hides the box. Under `read`, where every browser starts, the
    box's place says "Read-only. Raise this device to Control in Remote
    devices on the Mac to reply." `/device` is reread on every socket
    `hello` and on a `403 scope`, so a raise shows
    without a re-pair.
  - **States.** "Not paired", "Revoked", "Identity changed, pair again"
    and "Pairing is closed" are each said in a sentence. Resume, Trust,
    drafts, tickets and the conversation beyond the last response stay
    out (the issue's Not in this cut).

## The tests

- **Core** (`src/core/tests.rs`):
  - `invite_url` for a served leaf with an IPv4 and an IPv6 address,
    and `None` with no served leaf; the URL's host is the leaf's
    `address`, so the two cannot disagree.
  - `served_is_current`: true for a match; false for no leaf, another
    address, and an expiry within 30 days.
  - `ServedCertificateMade` sets `identity.served` and emits one
    `SaveIdentity`.
- **Adapter unit tests** (`src/adapters/remote.rs` tests):
  - `read_request` keeps only `switchboard-<port>` out of a `Cookie`
    with several cookies, and keeps `Origin`, `Host` and the page
    header.
  - `serving_certificate` over a key has the key's SPKI hash, a SAN for
    each name, the serverAuth EKU, and `not_after` at most 825 days
    out. Two issues over one key have different serials, and both
    differ from the serial of `make_identity`'s certificate.
  - An identity saved without `served` loads with `served: None`.
- **`tests/remote.rs`**:
  - Change `http` (:245) into `http_with(.., headers: &[(&str, &str)],
    ..) -> (u16, Vec<(String, String)>, String)` and keep `http` as a
    wrapper.
  - New: `the_page_is_served_before_pairing_with_its_headers`. The
    three paths return 200 with their content types, the exact CSP,
    `no-store`, `nosniff` and `no-referrer`. `/index.html`,
    `/favicon.ico`, `POST /` and `/other` return 404 `not_found`.
    `index.html` references exactly `/app.js` and `/app.css`, with no
    inline `<script>`, no `style=`, and no `http:` or `https:` URL.
    Every `"/api/v1/…"` literal in `app.js` begins with one of `info`,
    `pair`, `device`, `projects`, `waiting`, `sessions/` or `events`.
  - New: `a_browser_pairs_by_cookie_and_reads_and_subscribes_with_it`:
    - `/pair` with `cookie: true` and the page header, inside a
      `control` window, returns a `Set-Cookie` with each attribute, a
      body without `token`, and `scope: "read"`; the device in the
      core is `read`. Without the header it returns 403 `cross_origin`.
    - `/device` with the cookie returns `read`. A `send` with the
      cookie is 403 `scope`; after `SetDeviceScope` to control it
      types. Without the page header, with a foreign `Origin`, or with
      `Origin: null`, `/device` and `send` return 403 `cross_origin`.
    - `GET /app.css` with a known cookie moves its connection to the
      paired pool: `MAX_UNAUTHENTICATED` connections each fetch it with
      the cookie and stay open, and an unpaired `/info` on one more
      connection is answered. Without the cookie, that connection is
      closed at accept.
    - An events upgrade with the cookie and a matching `Origin` gets
      `hello`. A foreign `Origin` is refused. `subscribe_at` (:1960)
      gains a headers argument.
    - A cookie named for another port is not a token.
    - After `RevokeDevice`, the cookie gets 401 and the socket closes.
    - A bearer `/pair` inside a `control` window still grants
      `control`.
  - New: `the_served_certificate_is_kept_across_a_restart_and_names_the_qrs_address`.
    The port's leaf (read from the client handshake) is the same DER
    after the app is rebuilt over the same store, and it carries the
    address source's IP. With the address source changed, opening the
    pairing window rebinds, the served leaf names the new address, and
    `invite_url` names it too. The native pin verifies throughout.
  - No fixture changes: no wire view changes, and the Swift client does
    not read `/device`.
- **`tests/ui.rs`**: `remote_devices_dialog_opens_pairing_and_revokes_a_device`
  (:7424) seeds `served` on the identity and also finds the trust sentence while the window is open.
- **Swift**: `FixturesTests.swift` gains `testInviteParsesTheURL` next
  to the JSON test at :157. Cover IPv4, `[v6]:port`, and refusal of a
  URL missing `code`, `key` or `id`.
- **Spike 23** (`spikes/23-browser-page/README.md`) records the owner's
  device check, run before the commit:
  - scan, accept the warning; does `wss://` open (facts:53)?
  - is the cookie kept across a Safari relaunch (facts:45)?
  - quit and relaunch Switchboard, reload the page: does Safari warn
    again? It must not; a second warning means the served leaf changed.
  - how many connections an unpaired load opens, and whether `/pair`
    right after it is dropped while they sit in the stranger pool for
    their 30 s idle (facts:48); a drop selects the 4 → 8 fallback of
    decision 4.
  - a paired reload: does `/device` answer at once?

  The page works either way on the socket, so the commit does not wait
  on a "No". The README says which path iOS took.
- **Acceptance** is on a real phone with the signed bundle. It is not
  in CI. Between "see the sessions" and "send a reply", the owner
  raises the phone to Control in the devices dialog (decision 1).

## Docs

- **`docs/remote-client.md`**:
  - §4.1 bounds: the page window, and the three static files answered
    on the frame.
  - §4.3: add rows for `/`, `/app.js`, `/app.css` ("none, before
    pairing") and `GET /api/v1/device` (read).
  - §5, the pre-pairing rule (:306): `/info`, `/pair` and the page's
    three files.
  - §5, rewritten rather than added to:
    - :309 "Pairing verifies the key before the code is sent" states
      the native guarantee, then the browser's: the click-through is
      the trust step, the URL key catches a wrong or stale QR and not
      an impostor, and so a browser pairs `read` and is raised by hand.
    - :312, the token: "sent only as a bearer header over the pinned
      connection" becomes the native rule, beside the browser's
      `HttpOnly` cookie named by port, sent to whatever certificate the
      browser accepted for the origin, with the origin rule. "never in
      a URL" stays true: the code travels in the fragment.
    - :317 "The client verifies the pin on every connection" names the
      native clients only, and says a browser relies on the stable
      served leaf, so a second warning is the sign of a changed host.
  - §5, :307: the served leaf beside the identity's certificate, its
    SANs, EKU, validity and random serial, and when it is reissued.
- **`docs/design.md`**, Remote client section (from :3107): a new
  bullet, "The browser page (2026-10-xx)", before "Known gaps". Known
  gaps gain:
  - a changed address means re-pairing a browser;
  - iOS socket behaviour per Spike 23;
  - the excerpt is not on the page.
- **`README.md`**:
  - Layout lines for `web/` and `spikes/23-browser-page`.
  - The `tests/remote.rs` line (:437) gains the page and the cookie.
  - The adapter line (:383) gains the page window and the served leaf;
    the `src/app.rs` line (:401) gains the static files.
  - :277 says "Copy invite" copies the page's URL, which
    `switchboard-remote pair` also takes.
- **`client/README.md:180-190`**: `SWITCHBOARD_INVITE` takes a URL or
  JSON per line.
- **`CHANGELOG.md`** Unreleased: "The pairing QR code opens a page in a
  phone's or tablet's browser that lists the sessions, shows a
  session's last response or screen, and sends a reply once the
  browser is raised to Control in Remote devices."

## Commit

`The remote port serves a browser page, paired from a URL in the QR code`

Contract additions:

- **`src/ports/remote.rs`**: `ServedCertificate {certificate, address,
  not_after}`; `Identity.served: Option<ServedCertificate>`,
  `#[serde(default)]`, `IDENTITY_SCHEMA_VERSION` unchanged.
- **`src/core/action.rs`**: `AppAction::ServedCertificateMade(ServedCertificate)`.
  No new `Effect`.
- **`src/core/model.rs`**: none.
- **Protocol** (`docs/remote-client.md`):
  - static `GET /`, `/app.js`, `/app.css`;
  - `GET /api/v1/device` → `{device_id, name, scope}`;
  - `/pair` body field `cookie: bool`; with it, the device is `read`
    whatever the window grants, and the reply omits `token` and sets
    `Set-Cookie: switchboard-<port>`;
  - request header `X-Switchboard-Page: 1`;
  - refusal code `cross_origin`;
  - `Cache-Control: no-store` and `X-Content-Type-Options: nosniff` on
    every reply;
  - the invite as `https://<addr>:<port>/#code=&key=&id=`, with the JSON
    still accepted by `PairingInvite.parse`.
- **Adapter** (not contract):
  - `RemoteRequest.{cookie, origin, host, page}`;
  - `RemoteReply.{cookie, content_type}`;
  - `remote::serving_certificate`;
  - `tls_config` serves `identity.served` when present.
- **App** (not contract): `SwitchboardApp::set_address_source`.
- **Core helpers** (not contract): `core::remote::invite_url`,
  `core::remote::served_is_current`.
