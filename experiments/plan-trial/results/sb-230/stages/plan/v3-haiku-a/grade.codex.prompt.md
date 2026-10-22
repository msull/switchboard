You are scoring one implementation plan for a GitHub issue, blind: you do not know who or what wrote it, and you score this plan on its own.
The repository at the issue's base commit is checked out at /Users/sully/code_repos/plan-trial/sb-230-base (read-only; change nothing there). Every file path in the plan is relative to that tree.

Score by the rubric, column by column. Length is not thoroughness. For grounding, take the first five distinct file or line references the plan makes, in order of appearance, open each in the tree, and say what you found. For departures and decisions, use the reference as the yardstick; where you disagree with the reference, say so in the evidence rather than silently scoring your own view.

Your final message must be exactly this shape and nothing else:

```
| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| v3-haiku-a | <0-3> | <0-3> | <0-3> | <0-3> | <yes|note|no> |
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

# The plan (v3-haiku-a)

# Plan: #230, a browser page on the remote port, paired from a link

Checked against the worktree at `7fe8949` (branch `stage/sb-230-plan-v3-haiku-a`, clean). Facts cited as "facts: Settled …", "facts: Unresolved n", or "facts: For the owner X" are lines of `inputs/facts.md`.

## Where this departs from the issue

Each claim below is one the tree or the facts file does not bear out, and what the plan does instead.

1. **"The one-line excerpt the card shows" on the session list.** Not on the wire. The spec says the excerpt is `last_message` from the `Stop` hook, "a field of the session's metadata" (`docs/remote-client.md:207`). The tree has no such field. `last_message` lives on the hook event (`src/ports/events.rs:23-24`), and the core binds only `pending` from it (`src/core/events.rs:132`), so it is dropped. The card's excerpt is computed in the UI from the parsed conversation (`src/ui/cards.rs:177-181`). **This contradicts facts: Settled, "The tree", the line that puts `last_message` on the session record.** The plan's list shows the card word, the waiting reason and the name. The session page shows the last response. A host-computed excerpt would need a transcript parse per listed session, so it is an owner question, not built here.
2. **"The device's scope and host name in the header."** No endpoint returns the scope after pairing. `/pair` reports it once, and `info` is unauthenticated (`src/app.rs:3568-3599`). The plan adds an authenticated `GET /api/v1/device`.
3. **"'Revoked' said plainly."** `authenticate` gives one code, `unauthorized`, for both "never paired" and "revoked" (`src/app.rs:3127-3151`). The host keeps no record of revoked tokens, so it cannot tell them apart. The page says "no longer paired: the Mac revoked this device, or forgot it", and it says "identity changed" only when `/info` presents a different key, which it can tell apart. A distinct code would need a record change, so it is an owner question.
4. **The URL `key` "lets a genuine page refuse to send the code to a host whose `/info` key differs."** That holds, but only as far as `/info` is believed. The page cannot read the certificate it talks over, and the host's own key check takes the key from the request body (`src/app.rs:3636`). Spec §5 says a relayed pairing "fails even if the code reached it" (`docs/remote-client.md:309`). For a browser, a page served by an impostor can post the right key, so that sentence does not hold. The spec amendment says so.
5. **"The code never stays in the address bar or the history."** `history.replaceState` replaces the current entry (facts: Settled, "history.replaceState"). Whether the URL was recorded before the script ran is unknown (facts: Unresolved 6). The plan claims only what the tree supports: the code is single-use, and the window closes at five minutes or three wrong codes (`src/core/action.rs:972-973`, `src/core/remote.rs:65-71`). The dialog warns that a link opened on a phone may stay in its history.
6. **Click-through alone as the trust step, with the current certificate.** The certificate has no SAN, a CN only, and rcgen's default year-4096 expiry (`src/adapters/remote.rs:101-110`). It meets none of Apple's TLS rules (facts: Settled, "Apple's TLS server certificate rules"). The plan reissues it over the same key. The spec's "the certificate is verified out of band" assumption does not hold for browsers either.
7. **The events socket as the page's live channel.** Not the default. Safari's `wss` after a click-through is unverified (facts: Settled, "Safari and `wss://`"; Unresolved 1). The page polls REST. The socket is an upgrade: when it is open the page refetches on `changed`.
8. **The cookie as the token store.** The shape makes the cookie the token store. The plan's default is the page's own storage and a header. The cookie's `Secure` on an IP host and its `SameSite` on a WebSocket handshake are unresolved (facts: Unresolved 5, For the owner D). The host reads and writes the cookie in this commit. The page does not use it until D is favourable.

Facts corrections found in the tree (these are not issue claims, but the facts file is the yardstick, so they are noted):
- `src/adapters/remote.rs:112` holds the SPKI hash, not `:107` as facts: Settled, "The tree" says.
- `RemoteReply {` appears in 10 places in `src/app.rs` by grep, not "about 15" as facts says. The implementer should count at edit time.

## The record

- **No `SCHEMA_VERSION` bump and no `model.rs` change.** `SCHEMA_VERSION` is 16 (`src/core/model.rs:12`), `VIEWS_SCHEMA_VERSION` is 7 (`src/core/model.rs:612`), `IDENTITY_SCHEMA_VERSION` is 1 (`src/ports/remote.rs:17`). None changes. The identity version is written but never checked on read, so the new identity fields are additive with serde defaults. If the implementer decides to bump `IDENTITY_SCHEMA_VERSION` anyway, the bump is its value at implementation time plus one.
- **Device record: unchanged.** A browser is a `PairedDevice` in `devices.json` (a bare `Vec`, `src/adapters/remote.rs:80-86`). Its name is the page's `device_name`, built from `navigator.userAgent` (for example `Safari on iPhone`). `new_device` cuts it to 64 characters (`src/core/remote.rs:166-175`). No `kind` field (decision 6; facts: For the owner, the `kind` question).
- **Identity: two additive fields** in `Identity` (`src/ports/remote.rs:26-42`):
  - `certificate_names: Vec<String>`, the SANs the certificate carries, `#[serde(default)]`.
  - `certificate_issued: Option<SystemTime>`, `#[serde(default)]`.
  An empty `certificate_names` or a `None` issue time marks the old certificate. The key is not touched, so `spki_sha256` is unchanged (facts: Settled, "The certificate can be reissued over the same key"; `KeyPair::from_pem` at `src/adapters/remote.rs:139,146`).
- **The reissued certificate** (decision 1). `reissue_certificate(key_pem, identity, names)` in `src/adapters/remote.rs` (next to `make_identity`, `:95`):
  - the same key from the Keychain PEM the listener was built with;
  - SANs from `names`, which are the LAN address from `local_address()` (`:962`) and `<LocalHostName>.local` (facts: Unresolved 2 and For the owner B: reissue with both, whatever the answers);
  - `extended_key_usages` = `serverAuth` (facts: Settled, Apple's rules);
  - validity under 825 days (`not_before` an hour back, `not_after` 824 days out), from facts: Unresolved 3;
  - the CN `Switchboard <server_id>` as now.
  The builder that sets these fields is a separate pure function (`certificate_params`), so tests inspect its fields without parsing DER and without a new dependency. Verify in rcgen 0.14.10 at implementation: that `CertificateParams::new` turns an IP-looking name into an IP SAN, and the names of the public fields. The registry is outside the sandbox, so this was not checked here.
- **Decision 1: the click-through is the trust step.** The first visit is already past the warning, so a downloaded certificate would be a second, manual step (on iOS, a profile and a trust toggle). The download would itself need the warning to fetch. It removes no warning and adds a step, so it is not built. What the click-through buys: a first visit whose TLS identity was accepted by the owner, on the owner's network. What it does not buy: a verified identity. An impostor on the network gets the same warning, and the owner has no key to compare. The dialog says so.
- **`identity.json` writes** go through the existing atomic path (`Effect::SaveIdentity`, `src/core/action.rs:734`; the arm in `src/app.rs:2893`).

## The signals

- **No new event and no change to the socket's frames.** The `changed`/`removed` stream is as the spec describes it (`docs/remote-client.md` §4.4).
- **The page polls REST** (decision 7): the list every 5 s, the open session every 3 s, only while the page is visible. Each reply's envelope `revision` is applied only if it is at or past the held revision for its key (spec §4.2 at `docs/remote-client.md:146`). A waiting session moves to the top on the next poll.
- **Origin check on the WebSocket upgrade** (owner question on Origin; default here). If an `Origin` header is present and is not `https://<Host>`, the connection thread answers 403 `forbidden_origin`. Native clients send no `Origin` and are unaffected. The page's origin is the host's own name, so it passes whatever the owner answers about IP or `.local`. A cross-site page cannot set the subprotocol token, so the default has no CSRF surface.
- **Token transports** (decision 2). The host reads three: `Authorization: Bearer` (native and REST), a `bearer.<token>` value in `Sec-WebSocket-Protocol` (the socket), and, behind the enhancement, the cookie. The 101 echoes the `bearer.<token>` value the browser offered. RFC 6455 §4.2.2 requires an echo of one offered token, and `bearer.` plus base64url is a legal token (facts: Settled, "WebSocket subprotocol"). The cookie is set on `/pair` as `Set-Cookie: switchboard-<server_id>=…; HttpOnly; Secure; SameSite=Strict; Path=/` and read on requests. The page ignores it unless the owner's D answer is favourable, in which case the constant `COOKIE_ONLY` in `web/app.js` flips to `true` and the page stops storing the token.
- **The page's storage** is `localStorage`, scoped to the origin (scheme, host and port; facts: Settled, "Cookies are not isolated by port"). The CSP has no inline script, no external source and no `eval`. The page renders server text with `textContent` only. What the trade-off costs: a script on the origin could read the token. The CSP keeps the origin's scripts to the page's own. The owner accepts that, or chooses the cookie.
- **No `/api/v1/` path is refused any differently.** `serve_remote` (`src/app.rs:3017-3025`) keeps its prefix match. The static paths are answered before it, on the connection thread (below).

## The actions and effects

- **Static paths: exactly `GET /` and `GET /app.js`** (decision 4). Both are answered on the connection thread in `connection` (`src/adapters/remote.rs:770`), after `read_request` and before the rate check and the queue. The page holds no core state, and the spec puts content work on the connection thread (`docs/remote-client.md` §4.1). Responses:
  - `/`: `Content-Type: text/html; charset=utf-8`; `/app.js`: `text/javascript; charset=utf-8`.
  - `Cache-Control: no-store` on both (no caching across versions).
  - `Content-Security-Policy: default-src 'none'; script-src 'self'; style-src 'sha256-…'; img-src 'self' data:; connect-src 'self' wss:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'`. The `style-src` hash is of the inline CSS, computed once with the existing `sha2` crate at first use. The CSS is inlined into `/` at serve time, so the page is two requests. `connect-src 'self' wss:` is a choice: `'self'` alone may not match `wss` in every browser, and the device run checks it.
  - `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`, `Connection: close`.
  - The favicon is `<link rel="icon" href="data:,">`, so no `/favicon.ico` request is made.
  - Any other GET is unchanged: refused on the frame as today.
- **Pools and windows** (decision 4). A page load is an unpaired stranger, so it uses the unauthenticated pool (`MAX_UNAUTHENTICATED`, `src/adapters/remote.rs:567`). `Connection: close` frees the slot as soon as the response is written, so two requests per load do not hold it for 30 s. No rate window: the bytes are constant, and a window would fail a reload. A test checks that a page load is answered while strangers hold their slots (`tests/remote.rs`, the tests section).
- **Pairing link.** `pairing_link(spki, code, server_id, host, port)` in `src/adapters/remote.rs`: `https://<host>:<port>/?code=<code>&key=<spki>&id=<server_id>`. IPv6 is bracketed. The host is the LAN address from `local_address()`, which is the route's source address, not necessarily the one a phone reaches. If there is no address, the window shows no QR and says so. The QR at `src/ui/remote.rs:160-166` encodes this text. The plan's host is the IP (decision in For the owner B: IP by default, `.local` only if the owner's answer makes it better).
- **Pairing in the page** (`web/app.js`):
  1. Read `location.search` (`code`, `key`, `id`), then call `history.replaceState(null, "", "/")` before any network call.
  2. Fetch `/api/v1/info`. If its `spki_sha256` differs from `key`, show "This is not the Mac whose code this is" and send nothing (the wrong-key path counts against the window only when the page posts, so it never posts).
  3. Post `/api/v1/pair` with `{code, spki_sha256: key, device_name}`. On success, store `{server_id, spki, token}` in `localStorage`, fetch `/api/v1/device`, and render.
  4. Map refusals: `wrong_code` says the code does not match and codes close after three tries; `pairing_closed` says to open the window again; `wrong_key` as in step 2.
- **Resumed pages**: with a stored token, `/info` is fetched first. A different `spki` gives "identity changed, pair again" and no request carries the token (`docs/remote-client.md:315`). Otherwise `/device` is fetched. A 401 gives "no longer paired", and the stored token is cleared.
- **Reply** (`POST sessions/{id}/send`, body `{op, text}`). The op is built from `crypto.getRandomValues`, not `crypto.randomUUID`, so it needs no secure context (facts: Unresolved 4 is avoided). On a network error, the page queries `/api/v1/ops/{op}` and follows spec §4.5 (`docs/remote-client.md:274-278`). It never resends blind.
- **`GET /api/v1/device`** (new, read scope, any paired device). A new `remote_device(&device)` in `src/app.rs` beside `remote_info` (`:3568`), returning `{device_id, name, scope, server_name}`. It is answered directly, with `authenticated: true`, not through the wire `Body` enum. The `DeviceView` type is in `control/src/lib.rs`, next to `SessionView` (`:637`).
- **Core changes** (`src/core/remote.rs`):
  - `CertificateNamesSeen(Vec<String>)` (the app sends the desired names on listen and on a rebind, as an action the core decides on): if the identity's names differ, its issue time is `None`, or it is more than 365 days old, emit `Effect::ReissueCertificate { names }`.
  - `IdentityReissued(Identity)`: replace the identity, keeping `server_id`, `server_name`, `listen` and `spki_sha256` (assert in the core test), emit `Effect::SaveIdentity`, and rebuild the listener with the new TLS config through the existing rebind path (`a_change_of_address_rebinds_the_port`, `tests/remote.rs:1494`).
- **Effects** (`src/core/action.rs`): `ReissueCertificate { names: Vec<String> }` runs in `run_remote_effect` (`src/app.rs:680`) and dispatches `IdentityReissued`. The fake remote store in `adapters/fakes.rs` needs no change, because `reissue_certificate` is a pure adapter function.
- **Cookie** (enhancement, behind the default, decision 2). `RemoteReply` gains `cookie: Option<String>`. `/pair` sets it. The 12 literal sites (10 in `app.rs` by grep, 2 in the adapter) take `cookie: None`. Mechanical.
- **`RemoteRequest` fields** (adapter-only; facts: Settled, "RemoteRequest is built only inside `src/adapters/remote.rs`"): `host`, `origin`, `protocols: Vec<String>`, `cookie: Option<String>`. `read_request` (`src/adapters/remote.rs:444-514`) fills them. If there is no `Authorization` header, the bearer is taken from a `bearer.` protocol value. No header is logged. `refuse` logs method and path only (`src/app.rs:3754-3772`), and the query is never logged.
- **Text pairing paths** (decision 3). The QR, the "Copy invite" text and the README's `switchboard-remote pair "$(pbpaste)"` (`README.md:265-278`) carry the link. Swift `PairingInvite.parse` (`client/Sources/SwitchboardProtocol/Info.swift:114-116`) tries the URL first (`URLComponents`, `code`, `key`, `id`), with `addresses` set to the one `host:port`, and falls back to JSON. The JSON stays only in the parser, for copies made before this change. Nothing emits it. `switchboard-remote pair` (`client/Sources/switchboard-remote/main.swift:12-13`) takes the link or the JSON.
- **Page states** the page shows in plain words: unpaired ("Open the pairing window on the Mac and scan its code, or open the link it shows"); identity changed; no longer paired; could not reach the Mac (this is also the text when the certificate warning was not accepted); pairing refusals.

## The UI

- **Pairing window** (`src/ui/remote.rs:141-210`): the QR encodes the link. Under it, the same link as text with "Copy invite". The text says: "Opening the link on a phone trusts this Mac's certificate for this address once you accept the browser's warning. The code works once, for five minutes or three wrong codes. The link may stay in the phone's history." The current "Scan, or type the key and the code" line keeps the key and code.
- **Paired devices dialog**: each row keeps scope, last seen, and Revoke. The dialog's sentence about pairing trust is the one above.
- **The page** (`web/index.html`, `web/app.js`, `web/app.css`): a header with the Mac's name, the device name and scope. Sessions grouped by project, waiting first, each waiting one marked "Waiting on you" with its reason; the list shows the card word and the name, with no excerpt (departure 1). A session view: the last response for a Claude Code session, the screen otherwise, polled every 3 s. A reply box that is shown only with `control` scope. The layout is one column, with 44-point touch targets, and it follows the system's light and dark setting.
- **Owner-facing wording** for the dialog and the page is in `docs/remote-client.md` §5 as amended.

## The tests

No browser UI test exists in the tree, and the issue rules out a JS dependency, so the page's logic is checked by hand. The host's behaviour is tested over the real TLS listener.

- **`src/adapters/remote.rs` unit tests**:
  - `certificate_params` carries the names, `serverAuth` and the bounded validity, and the reissued certificate keeps `spki_sha256` (the existing `debug_assert` at `:113`, and `spki_sha256_of_key` at `:138`).
  - `pairing_link` carries code, key and id, and brackets an IPv6 host.
  - `read_request` takes the bearer from the protocol value when there is no header, and fills `origin`, `host` and `protocols` (extend `a_request_reads_with_its_query_bearer_and_body`, `:1045`).
  - `asset` matches exactly `/` and `/app.js`, and nothing else (`/index.html`, `/app.js/`, `/api/v2` are refused).
- **Core** (`src/core/tests.rs`, dispatch with `Clock::at`):
  - a fresh identity whose names differ emits `ReissueCertificate` once, and not again when they match;
  - `IdentityReissued` keeps id, name, listen and SPKI, and emits `SaveIdentity`.
- **`tests/remote.rs`** (raw TLS, the existing `http` helper at `:245` extended to return headers, and `subscribe_at` at `:1960` extended to send headers):
  - `the_page_is_served_unpaired_with_its_headers`: `/` and `/app.js` return 200 with the content types, `Cache-Control: no-store`, `Connection: close` and the CSP named in the actions section, with no `unsafe-inline`;
  - `the_page_asks_only_for_paths_the_host_serves`: scan `web/app.js` and `web/index.html` for `/api/v1/` literals, replace `${…}` and `{…}` with a sample id, and send each with a paired token. None may answer with the message "not available on this port" (a 404 for an unknown id is fine);
  - `a_page_load_is_answered_while_strangers_hold_their_slots`: three idle unauthenticated connections, then `/` still answers;
  - `a_browser_pairs_from_the_link_and_reads_its_scope`: build the link with `pairing_link`, parse its query in the test, post `/pair` with `device_name` `Safari on iPhone`, then `GET /api/v1/device` returns the name and the scope;
  - `a_socket_takes_its_token_from_the_protocol_and_echoes_it`: an upgrade with `Sec-WebSocket-Protocol: bearer.<token>` and no `Authorization` gets 101 with the same value echoed;
  - `a_socket_from_another_origin_is_refused`: `Origin: https://evil.example` gets 403 `forbidden_origin`; `Origin` equal to `https://<Host>` gets 101; no `Origin` gets 101;
  - `a_revoked_device_is_refused_on_the_next_request`: after `RevokeDevice`, `/device` answers 401 `unauthorized` with the message the page maps to "no longer paired" (the existing revocation test at `:658` pins the same behaviour);
  - the existing `info_is_served_to_anyone_and_every_other_read_is_refused_unpaired` (`:426`), `the_allowlist_refuses_everything_else_with_no_side_effect` (`:682`, which uses `/api/v2`) and `pair_is_rate_limited_per_listener` (`:1406`) stay as they are.
  - A reissued certificate keeps the pin: a client with the old SPKI pin still verifies after `IdentityReissued` (`tls` pinned client at `src/adapters/remote.rs:168`).
- **Fixtures.** `DeviceView` is new and the Swift client does not decode it in this change, so no `client/Fixtures` file is added. If it does later, the rule applies.
- **Swift** (`client/Tests/SwitchboardProtocolTests/`): one new test file, `PairingInviteTests.swift`, parses the link, the JSON, and a link with an IPv6 host. This is the client's own test target; the implementer runs it with the client's usual command.
- **Manual, not gating the commit**:
  - Safari on the Mac opens `https://<lan address>:<port>/` (decision 1's click-through), reaches the page, pairs from the link, and sees the sessions. This is the one browser check the tree allows.
  - The phone run is the owner's (facts: For the owner A, B, C, D). It records, with the certificate as built, whether the page loads, `fetch` works, the socket connects, and the cookie works. Its answer chooses the cookie and the `.local` URL, and changes no default.
- **Pre-push**: `cargo test --locked --workspace`, `cargo clippy --locked --workspace --all-targets -- -D warnings`, `cargo fmt --all`, and the Linux clippy check for `dispatch` (CLAUDE.md). `include_str!` paths are relative to `src/adapters/`, so `../../web/…` is portable to CI.

## Docs and status

- **`docs/remote-client.md`**: §4.1 bounds gain the two static paths, the `Connection: close` rule and the Origin check (`:124-131`). §4.3 table gains `GET /`, `GET /app.js` and `GET /api/v1/device` (`:159-192`). Line 207 is amended: the excerpt is not on the session metadata today; the list shows the card word. §5: line 306 becomes "before anything but `/info`, `/pair`, and the page's two paths"; line 309 says the QR carries the link, and that a relayed pairing through a browser is not stopped by the key check; line 312 says the token travels as a bearer header, or in the `bearer.` subprotocol for a socket, and never in a URL; line 316 gains the trust sentence.
- **`docs/design.md`**: a new dated section, "Remote web page status (2026-10-22)", at the end of "Remote client (2026-10-12)", before "Versioning and updates (2026-10-15)" (`:3585`). It records what is built, the decisions, and the open facts (For the owner A–D, Unresolved 1–9) as "to run".
- **`README.md`**: the Layout table (`:369-446`) gains `web/` and `src/adapters/remote_page.rs`; the rows for `src/adapters/remote.rs` (`:383`), `src/ui/remote.rs` (`:423`), `src/core/remote.rs` (`:369`) and `tests/remote.rs` (`:437`) are updated. The Development section's remote text (`:265-278`) is updated to the link.
- **`CHANGELOG.md`**: one line under `[Unreleased]` (`:9`), for the user-visible change.

## Commit message

`Serve a browser page on the remote port and pair from a link`

## Contract additions

- `control::DeviceView` (new wire view, `control/src/lib.rs`): `{device_id, name, scope, server_name}`.
- `GET /api/v1/device` (read scope) and the static paths `GET /`, `GET /app.js` (`docs/remote-client.md` §4.3).
- `AppAction::CertificateNamesSeen(Vec<String>)` and `AppAction::IdentityReissued(Identity)` (`src/core/action.rs`).
- `Effect::ReissueCertificate { names: Vec<String> }` (`src/core/action.rs`).
- `ports::remote::Identity` gains `certificate_names: Vec<String>` and `certificate_issued: Option<SystemTime>`, both `#[serde(default)]` (`src/ports/remote.rs`).

Not contract, but changed: `RemoteRequest` gains `host`, `origin`, `protocols` and `cookie`; `RemoteReply` gains `cookie: Option<String>`. Both live in `adapters/`, so CLAUDE.md's additivity rule does not name them (facts: Settled, "RemoteRequest and RemoteReply live in adapters/"). No schema version changes.

---

# The facts file the planner was given

The plan may cite this file as facts.md or facts:<line>; such a citation is checked against this text, not the tree. It was written by an earlier stage of the same process, not by the owner, so it is not the reference and may itself be wrong.

     1	# Facts for #230 (web control surface)
     2	
     3	No spike was run: every open question needing a browser or iPhone is not runnable here, and none of the rest needs an experiment. `git status` is clean.
     4	
     5	## Settled
     6	
     7	### The tree
     8	
     9	- A leaked pairing code is short-lived: the window closes on the first successful pairing, after 3 wrong codes, or at 5 minutes. `src/core/remote.rs:46-72` (`Paired` clears `pairing`, "single-use").
    10	- `RemoteRequest` is built only inside `src/adapters/remote.rs` (definition `:360`); no test in `tests/` constructs it. Adding `cookie`, `origin` and `user_agent` fields touches the adapter alone. Grep: `RemoteRequest\b` over `src tests`.
    11	- `RemoteReply { status, body, authenticated }` is built literally at about 15 sites in `src/app.rs` (e.g. `:1664`, `:2365`, `:2754`, `:3570`, `:3723`, `:3736`, `:3767`) and not in `tests/`. A header field on it needs a `Default` or a constructor, or every site changes.
    12	- `RemoteRequest` and `RemoteReply` live in `adapters/`, not `ports/` or `model.rs`, so CLAUDE.md's additivity rule for the shared contract does not name them. `src/adapters/remote.rs:355-393`.
    13	- `last_message` exists on the session record: the hook binary writes it from `last_assistant_message` (`src/bin/switchboard-hook.rs:37`) and `src/core/tests.rs:1717` constructs records with `last_message: None`. It is not on the wire: no match in `control/` or `client/Sources`. Adding it to `SessionView` is a new field there.
    14	- The certificate can be reissued over the same key. `rcgen::KeyPair::from_pem` is already used at `src/adapters/remote.rs:139,146`, the SPKI hash is of the key (`:107`), and `make_identity` (`:95-127`) is the only path that makes a new key. A reissue keeps `spki_sha256`.
    15	- `PairingInvite` is used by `ConnectionManager.swift`, `BrowserView.swift`, `switchboard-remote/main.swift` and eight test files under `client/Tests`. `addresses` is read as a list at `ConnectionManager.swift:483` and built as a one-element list at `main.swift:120`. A URL parser can fill `addresses` with the one host:port.
    16	
    17	### Outside the tree
    18	
    19	- **Apple's TLS server certificate rules.** SHA-2 signature, RSA at least 2048 bits, the server's DNS name in subjectAltName (CommonName not trusted), and for certificates issued after 1 July 2019 an EKU of `serverAuth` and a validity of at most 825 days. Non-compliant connections fail and "websites [may] not load in Safari". Evidence: https://support.apple.com/103769 (iOS 13 / macOS 10.15; Citrix says iOS 14 / macOS 11 also). The page does not mention IP-address SANs, user-added roots, or exemptions. The current certificate has no SAN, a CN only, and a not-after of year 4096 (`src/adapters/remote.rs:95-127`), so it meets none of the SAN, EKU or validity rules.
    20	- **Safari and `wss://` after a click-through.** The behaviour exists and is unfixed: WebKit bug 41419 ("allow wss:// if user has accepted https:// warning") is open, status NEW, last modified 2026-01-12, and the reporter got `wss` working only after marking the certificate always-trusted (https://bugs.webkit.org/show_bug.cgi?id=41419). code-server's iPad doc agrees: Safari "will prevent WebSocket connections unless the certificate is installed as a profile", needs a domain name rather than a bare IP, and needs a SAN matching the host (https://github.com/cdr/code-server/blob/v3.7.0/doc/ipad.md). Two sources agree; they cover Safari up to 2026-01. One more report is mixed: on iOS 26 beta 3, `wss` failed after acceptance and the poster said it was resolved in beta 5 (https://developer.apple.com/forums/thread/792842). That is a forum post, a lead only.
    21	  - Queries: "Safari iOS WebSocket wss self-signed certificate accepted exception fails fetch works"; "iOS Safari self-signed certificate no subjectAltName IP address proceed certificate warning".
    22	- **Secure contexts.** `crypto.randomUUID()` is available only in secure contexts (https://developer.mozilla.org/en-US/docs/Web/API/Crypto/randomUUID). The spec decides trustworthiness by the origin's scheme: `https` and `wss` are "Potentially Trustworthy", with no mention of certificate errors (https://w3c.github.io/webappsec-secure-contexts/ §3.1; MDN Secure_Contexts agrees). By the text, an https page on a click-through certificate is a secure context. Whether browsers also treat it so is not stated, and it is under Unresolved.
    23	- **Cookies are not isolated by port.** RFC 6265 says so, quoted on https://developer.mozilla.org/en-US/docs/Web/HTTP/Guides/Cookies. `Secure` cookies are sent only over HTTPS. `localStorage` is per origin including port (MDN, general). This confirms the notes' cross-port claim.
    24	- **WebSocket subprotocol (RFC 6455 §4.1, §4.2.2, https://www.rfc-editor.org/rfc/rfc6455).**
    25	  - The client must fail if the 101 names a subprotocol it did not offer.
    26	  - The server's reply is one `token`, and an empty value is illegal.
    27	  - Offered values are `token`s, so base64url without padding is legal. Base64 `=` is not a token character (a separator).
    28	  - Whether the client must fail when the server returns none is not normative: §4.2.2 says the server sends none when it agrees to none, and §1.9 (non-normative) says it must include it.
    29	  - Browsers must send `Origin` on the handshake. A server may validate it, and rejects with 403. "If the server does not validate the origin, it will accept connections from anywhere."
    30	  - The tree's `respond_upgrade` (`src/adapters/remote.rs:519-527`) echoes no subprotocol.
    31	- **`history.replaceState` replaces the current entry**; Back then shows the new URL and bypasses the old one (https://developer.mozilla.org/en-US/docs/Web/API/History/replaceState). It says nothing about what the browser recorded before the script ran.
    32	  - Query: "MDN history.replaceState".
    33	
    34	## Unresolved
    35	
    36	Each line ends with what the plan builds to. The plan never waits on a line here.
    37	
    38	1. **Does a click-through exception cover `wss://` and `fetch` on current iOS Safari, with this no-SAN certificate on a bare IP?**
    39	   - Tried: the queries above, plus "Safari remember certificate exception visit this website how long self-signed iOS per session". The only evidence is old (2010 bug, iOS 7 and 13 era), the code-server doc (v3.7, 2021), and the one iOS 26 beta post. Nothing current and primary exists.
    40	   - Favourable: the page may use the events socket directly. Unfavourable, and the plan must do this anyway: the page works with REST polling alone (poll `changed`-equivalent reads on a timer) and treats the socket as an upgrade it may lose, saying "live updates unavailable" rather than failing.
    41	   - Reissuing the certificate with a SAN, `serverAuth` and a bounded validity over the same key is cheap and helps in both cases (Apple's page; the code-server doc).
    42	2. **Does a SAN with an IP address, rather than a DNS name, satisfy Safari and Chrome?** The Apple page speaks only of DNS names. Queries: as in 1, plus "iOS Safari self-signed certificate no subjectAltName IP address". No primary source found.
    43	   - Favourable: put the LAN IP in the SAN. Unfavourable: the page's URL host must be a `.local` name. Either way the plan reissues with the `.local` hostname as a DNS SAN as well.
    44	3. **Does Apple's 825-day rule apply to a manually trusted self-signed leaf?** The page lists no exemption and no scope. Build to a validity of 825 days or fewer and say in the dialog that the certificate is renewed (the key, and so the pin, is unchanged). If it does not apply, nothing is lost.
    45	4. **Does a click-through certificate give a secure context in Safari, Chrome and Firefox?** The spec reads yes by scheme, but I found no browser documentation on certificate-error overrides. The plan uses `crypto.randomUUID` only when `window.isSecureContext`, with a fallback id generator built on `getRandomValues`, and does not rely on `Secure` cookies being stored without a manual test (see item 5).
    46	5. **Are `SameSite=Strict` cookies sent on a same-origin `new WebSocket()` handshake, and do `Secure` cookies work on an IP host, in Safari, Chrome and Firefox?** Query: "SameSite=Strict cookie sent on same-origin WebSocket handshake; Secure cookie IP address host Safari Chrome Firefox". Results were blogs and a Chromium design thread (WebSocket requests carry cookies; `wss` counts as the same scheme as `https`): leads, not evidence. Plan to either transport behind one server-side reader that accepts the bearer header, the cookie and the subprotocol token, and let the first manual run pick the page's transport.
    47	6. **Is the code-bearing URL recorded in history, tab list or iCloud tab sync before `replaceState` runs?** No source. The plan assumes yes: the code is single-use and the window is 5 minutes or 3 failures (settled above), so a recorded URL is dead once pairing succeeds. The plan says that in the dialog and keeps the query out of server logs (`refuse`, `src/app.rs:3754`, logs path only).
    48	7. **Per-site exception lifetime across launches, and loss when the IP changes.** Query: "Safari remember certificate exception ... how long". Only a 2013 blog (iOS 7: exception not removable). Plan: an address change means another warning, and the page and dialog say so. A `.local` name or a reissue never makes it worse.
    49	8. **Pool of 4 under a real page load (iOS Safari parallel connections, keep-alive, `/favicon.ico`).** No source for the connection count; it needs a device. Plan: answer static paths on the connection thread with `Connection: close`, serve the page as few files as possible (HTML plus one script and one stylesheet, inlined where possible by the CSP), and give static paths their own bound so a page load cannot starve `/pair`.
    50	9. **Bonjour `.local` resolution from iOS Safari.** No query run, so no evidence. Plan: the URL carries the IP from `local_address()` and the `.local` name only if the owner answers item B below.
    51	
    52	## For the owner
    53	
    54	A. **On a real iPhone/iPad, after accepting the warning, does the page load, do `fetch` calls work, and does `new WebSocket("wss://…")` connect, with the current certificate and with the reissued one?** Ask with the page as built. Favourable branch: use the socket. Unfavourable: ship polling. The plan builds the polling path regardless.
    55	B. **Does `https://<mac-name>.local:<port>/` resolve from the owner's phone?** Favourable: reissue with that DNS SAN and put it in the URL. Unfavourable: the IP stays the URL host. Reissuing with both works either way.
    56	C. **Does the owner's phone camera open the URL straight into Safari, and does the tab appear in iCloud tab sync or history before the page script runs?** Either branch: the code is single-use and expires, so the plan holds.
    57	D. **Does a cookie, or a subprotocol token, work in Safari with the owner's certificate (items 5, 1)?** Favourable: use the one that works, the cookie preferred for the token never being readable by script. Unfavourable: use the other; the server accepts all three transports regardless.
