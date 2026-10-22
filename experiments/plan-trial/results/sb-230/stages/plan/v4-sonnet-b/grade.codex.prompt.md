You are scoring one implementation plan for a GitHub issue, blind: you do not know who or what wrote it, and you score this plan on its own.
The repository at the issue's base commit is checked out at /Users/sully/code_repos/plan-trial/sb-230-base (read-only; change nothing there). Every file path in the plan is relative to that tree.

Score by the rubric, column by column. Length is not thoroughness. For grounding, take the first five distinct file or line references the plan makes, in order of appearance, open each in the tree, and say what you found. For departures and decisions, use the reference as the yardstick; where you disagree with the reference, say so in the evidence rather than silently scoring your own view.

Your final message must be exactly this shape and nothing else:

```
| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| v4-sonnet-b | <0-3> | <0-3> | <0-3> | <0-3> | <yes|note|no> |
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

# The plan (v4-sonnet-b)

# Plan: a web control surface on the remote port (#230)

Line numbers are as they are in this worktree at 7fe8949. Schema versions
as they stand today: `SCHEMA_VERSION = 16` (`src/core/model.rs:12`),
`VIEWS_SCHEMA_VERSION = 7` (`:612`), `IDENTITY_SCHEMA_VERSION = 1`
(`src/ports/remote.rs:17`). Only the last moves, and it is written below
as "the value in `ports/remote.rs` at implementation time, plus one".
"Facts" means the facts file for this issue; a line cited as Settled N or
Unresolved N is in it.

## Where this departs from the issue

- **"The one-line excerpt the card shows" is not on the wire.**
  `SessionView` (`control/src/lib.rs:636-688`) has no excerpt. The record
  has `last_message` (written by `src/bin/switchboard-hook.rs:37`;
  Settled, tree), but `remote-client.md` §4.3 says the excerpt is
  "masked from the turn's full text, then cut", and masking resolves the
  session's secret set, which §4.1 keeps off the frame. Putting
  `last_message` on `SessionView` would send unmasked text from a frame
  answer. So no `SessionView` change: the page fetches
  `sessions/{id}/last-response` for each Claude session, as the headset
  does (`client/Sources/SwitchboardUI/BrowserView.swift:149-163`), and
  only when that session's `last_stop_at_ms` differs from what it holds.
  No `watch` is registered for these, so the 128-key watch set is not
  touched by the list. Codex, shell, command and service rows show kind
  and state without an excerpt.
- **"The device's scope in the header": no endpoint returns it after
  pairing** (`remote_info`, `src/app.rs:3568`, is unauthenticated; the
  `/pair` reply is the only place, and it goes stale when the owner
  changes the scope in the dialog). The plan adds one authenticated read,
  `GET /api/v1/device`. This is the single new read model; the page
  cannot show a current scope without it.
- **"Revoked said plainly" cannot be told from "never paired" by the
  host's answer.** `authenticate` (`src/app.rs:3127-3153`) refuses both
  with `401 unauthorized`, and with an HttpOnly cookie the page cannot
  see whether it holds a token. The page keeps a non-secret marker
  (`server_id`, `device_id`, key hash, name) in `localStorage`. A 401
  with a marker says "This browser is no longer paired (it was revoked on
  the host, or the browser dropped its cookie). Pair again." A 401 with
  no marker says how to pair. "Identity changed, pair again" is the
  marker's `server_id` matching `/info` with a different key hash.
  No new refusal code.
- **"Any path outside `/api/v1/` is refused" is true but late:**
  `serve_remote` refuses it at `src/app.rs:3023`, after the request has
  taken a slot in the 64-deep channel and a frame pass. With a page, a
  browser sends strays (`/favicon.ico`, a prefetch). The connection
  thread now owns this decision: the three page paths are served there, any
  other path outside `/api/v1/` is answered 404 there, and `serve_remote`
  keeps its own refusal as the backstop.
- **"The browser then trusts that certificate for that address" is not
  safe to build on.** Settled (WebKit 41419, code-server's iPad doc): on
  Safari a click-through can cover the page and `fetch` but not `wss://`.
  The page therefore works on REST polling alone and treats the socket as
  an upgrade (decision 1 below).
- **"`history.replaceState` … the code never stays in the history" is
  stronger than the sources.** MDN (Settled) says `replaceState`
  replaces the current entry; it says nothing of what the browser
  recorded before the script ran (Unresolved 6). The plan makes the
  claim the host can keep: the code is single-use and the window closes
  at the first pairing, three failures or five minutes (Settled,
  `src/core/remote.rs:46-72`), so a recorded URL is dead. The dialog says
  that instead of promising the history is clean.

## The record

No new record kind, no `model.rs` change, no `SCHEMA_VERSION` or
`VIEWS_SCHEMA_VERSION` bump. A browser is one more `PairedDevice`
(`src/ports/remote.rs:96-107`) in `devices.json`, which is a bare
`Vec<PairedDevice>` with no version. Its name is the page's `device_name`
("Safari on iPhone", built by the page from `navigator.userAgent`; the
host already cuts it to 64 characters, `new_device`,
`src/core/remote.rs:166-175`). No `kind` field: the name is enough for
the dialog (decision 6) and the host reads no `User-Agent`. Revoke and
scope change already apply on the next request and revoke closes the
socket.

One record changes: `identity.json`. The certificate is reissued over the
same key (Settled: `KeyPair::from_pem` is already used,
`src/adapters/remote.rs:139,146`; the SPKI is of the key, so the pin
and `spki_sha256` do not move, §5). The reason is Settled (Apple's
rules): the current certificate has no subjectAltName, a CN only, no EKU,
and a `not_after` in the year 4096, so it meets none of the SAN, EKU or
825-day rules Safari applies. Reissuing cannot hurt the click-through
path and may help it (Unresolved 1, 2 and 3 are the open questions about
how much), so it does not wait on them. Two additive fields on `Identity`
(`src/ports/remote.rs:26-42`), both `#[serde(default)]`:

- `certificate_names: Vec<String>`: the DNS and IP names the certificate
  carries, sorted.
- `certificate_expires: Option<SystemTime>`.

Bump `IDENTITY_SCHEMA_VERSION` to the value in `ports/remote.rs` at
implementation time, plus one. `identity.json` has no migration step:
`load_identity` is a plain deserialize, so an older file reads with the
defaults (an empty list, `None`), which `bind_remote` treats as "reissue
now". Written through the existing atomic `RemoteFiles::save`.

Rehydration is unchanged: `IdentityLoaded` at launch, devices by token
hash on every request. The pairing window stays non-durable. On the
device, what persists is the HttpOnly cookie (script cannot read it) and
the marker in `localStorage` (not secret: ids and a public key hash).
The cookie is the only place the token lives in the browser (decision 2).

## The signals

### Decision 1: browser trust in the certificate

**Picked: the click-through on the local network is the trust step, said
in the dialog. The host does not offer the certificate for download in
this cut.** Why: a download or profile install is a second trust
mechanism with its own surface (a route that serves a certificate before
pairing, a profile format, what iOS asks of a manually trusted
certificate), and the only evidence that it fixes `wss://` on Safari is
two sources of 2021 and 2026 (Settled), against Unresolved 1-3, which no
experiment here can settle. It is the follow-up if owner question A
comes back unfavourable (named at the end).

What the click-through buys, and what it does not (the dialog says this
in two sentences, and §5 of the spec is amended to say it):

- It lets the owner open the page. It proves nothing about who answered.
- The `key` in the URL lets the genuine page refuse to send the code to a
  host whose `/info` presents a different `spki_sha256`. That catches the
  right page on the wrong genuine host (a second installation on that
  address or port, a rotated key). A page served by an impostor is not a
  genuine page and checks nothing; a page cannot read the key of the TLS
  connection it is on, so even the genuine check proves only that the host
  claims that key. §5's sentence "a relayed request through a substituted
  certificate fails" stands for native clients and is amended: it does not
  hold for a browser.
- What bounds the damage: the code is single-use and short-lived
  (Settled), the pairing window shows scope, and the owner sees the new
  device in the dialog and can revoke it.

The reissue (above) gives the certificate `serverAuth`, a validity of 800
days (under 825, Unresolved 3: if the rule does not bind a manually
trusted leaf, nothing is lost), and SANs for `localhost`, `127.0.0.1`,
the outward address from `local_address()` as an IP SAN, and, on macOS,
`<LocalHostName>.local` as a DNS SAN (Unresolved 2 and 9: both forms are
present, so whichever Safari accepts is there). The dialog says the
certificate is renewed (key, and so pin, unchanged) and that a changed
address means a new browser warning (Unresolved 7).

The page's default is REST polling every 5 seconds plus a refetch of the
open session. When `new WebSocket` opens, the page switches to events
and stops polling; when it fails or closes, the page says "Live updates
unavailable, refreshing every 5 s" and polls. This is the same page
whether Unresolved 1 comes back favourable or not; the socket is the
enhancement, built behind the polling and not in front of it.
`crypto.randomUUID` is used only when `window.isSecureContext`, with a
`getRandomValues` fallback otherwise (Unresolved 4).

### Decision 2: how the token reaches the host

**Picked: an `HttpOnly; Secure; SameSite=Strict` cookie, set by the
`/pair` reply, carried by `fetch` and by the socket handshake. The page's
script never sees the token. Nothing in `localStorage`, no subprotocol
token.** That is the safer store (a script on the origin can read
`localStorage`; it cannot read an HttpOnly cookie), so it is the one this
plan builds. The facts file's Owner D says the server accepts all three
transports; this plan builds one (plus the bearer header the native
clients use) and does not build the other two.

Mechanics that are unresolved are the implementer's first test, before
any other page code (Unresolved 5): on Safari, Chrome and Firefox with the
reissued certificate, does a `Secure` cookie set by `/pair` over an IP
host come back on a same-origin `fetch`? (This is the basic behaviour; the
`__Host-` prefix needs `Secure`, `Path=/`, no `Domain`, all met.) Does it
come back on a same-origin `new WebSocket` handshake? If the `fetch` half
fails, the finding goes to the owner as a follow-up and no
`localStorage` or subprotocol transport is added in this commit. If only
the socket half fails, nothing changes: the page polls (decision 1).

Host side:

- `read_request` (`src/adapters/remote.rs:444-513`) keeps three more
  headers, as the bearer is kept, so none are logged with the rest:
  `cookie`, `origin`, `host`. `RemoteRequest` (`:360`) gains
  `cookie: Option<String>`, `origin: Option<String>`,
  `host: Option<String>`. Settled: only `adapters/remote.rs` builds it,
  and no test constructs it.
- The token is the bearer if present, else the cookie named
  `__Host-sb-<server_id>` (per-installation name: Settled says cookies are
  not isolated by port, so two instances on one Mac would otherwise
  collide). `authenticate` (`src/app.rs:3127`) takes it from a new
  `RemoteRequest::token(&self, server_id)`; the cookie value is checked
  for length and base64url characters before it is hashed.
- **Origin check.** A cookie is ambient: `SameSite=Strict` is by site, and
  a site includes every port on the host, so another page on another port
  of this Mac is same-site. For a request authenticated by the cookie (no
  bearer), any method but GET and any WebSocket upgrade must carry an
  `Origin` equal to `https://` + the request's `Host` header, compared
  case-insensitively; otherwise `403 forbidden_origin`, no side effect.
  Comparing with the `Host` header, not a list of addresses, is what makes
  this correct for an IP, a `.local` name or loopback. Bearer-authenticated
  requests are exempt: a header is not ambient. This answers the owner
  question on `Origin`, and it is a security decision, so it is decided
  here.
- **`/pair`** (`remote_pair`, `src/app.rs:3606`) takes an additive body
  field `"transport": "cookie"`. With it the reply omits `token`, and
  `RemoteReply` carries `Set-Cookie: __Host-sb-<server_id>=<token>;
  Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=34560000` (400 days,
  the longest browsers keep). Without it the reply is unchanged, so the
  CLI and the headset are untouched. `RemoteReply` (`:389`) gains
  `set_cookie: Option<String>` and `#[derive(Default)]`; Settled says it
  is built literally at about 15 sites in `src/app.rs` and none in `tests/`,
  so each literal gets `..RemoteReply::default()`, a mechanical edit.
- The cookie goes to every HTTPS service on that host, on any port
  (Settled). It is `HttpOnly`, so it cannot be read by script; it can be
  sent to a service the owner runs on the same Mac. The dialog says so, and
  it is named for the owner at the end.

### Decision 3: one QR

**One URL.** The pairing window's QR and "Copy invite" carry
`https://<ip>:<port>/?code=<code>&key=<spki_sha256>&id=<server_id>`,
built by one pure function in `src/core/remote.rs`,
`invite_url(address: SocketAddr, identity, code)`. It formats the address
through `SocketAddr`, so an IPv6 result gets its brackets (the JSON form
did not). Code, key and id are Crockford, hex and a UUID: nothing needs
percent-encoding. The JSON form is no longer emitted. Settled: the
visionOS paste field, `ConnectionManager.swift`, `BrowserView.swift:106,
883` and the CLI all go through `PairingInvite.parse`
(`client/Sources/SwitchboardProtocol/Info.swift:114`), so only that
changes: it tries `URLComponents` first (`code`, `key`, `id`, and
`addresses = ["host:port"]`) and the existing JSON decode second, so an
invite copied before the update still pairs. `addresses` stays a list
(`ConnectionManager.swift:483`, `main.swift:120`). The `.local` name is not
put in the URL: owner question B decides that, and the IP works whichever
way it comes back.

### Decision 4: the static paths

- **Paths, exactly:** `GET /` (also with a query), `GET /app.js`,
  `GET /app.css`. Content types `text/html; charset=utf-8`,
  `text/javascript; charset=utf-8`, `text/css; charset=utf-8`. The HTML
  carries `<link rel="icon" href="data:,">` so no `/favicon.ico` request is
  made. Any other method on these is `405` with `Allow: GET`; any other
  path outside `/api/v1/` is `404` with the usual JSON error.
- **Headers on all three (and the 404/405):** `Content-Security-Policy:
  default-src 'none'; script-src 'self'; style-src 'self'; connect-src
  'self'; img-src data:; base-uri 'none'; form-action 'none';
  frame-ancestors 'none'`, `Cache-Control: no-store`,
  `Referrer-Policy: no-referrer` (the URL with the code must not leave in
  a `Referer`), `X-Content-Type-Options: nosniff`, `Connection: close`.
  The page has no inline script, no inline style and no `style=` attribute,
  and no external URL. Whether WebKit treats `connect-src 'self'` as
  covering `wss://` on the same host is not in the facts; the first
  manual run on Safari records it, and the polling path is why a "no" does
  not break the page.
- **Who answers:** the connection thread (`connection`,
  `src/adapters/remote.rs:770`), before the request is handed to the
  frame, because the bytes need no core state and §4.1 says the frame does
  core state only. The bytes are `include_str!("../../web/index.html")`,
  `app.js`, `app.css` in `adapters/remote.rs`. No build step, no
  dependency.
- **Pool and rate window.** A page load counts against the 4
  unauthenticated connections like any stranger (§4.1; unchanged), and
  `Connection: close` frees the slot as soon as the file is sent, so the
  three files do not hold three slots for 30 s of idle (Unresolved 8: how
  many parallel connections iOS Safari opens needs a device). Static paths get
  their own `RateWindow` in `Limits` (`:625-640`), 60 per minute per
  listener, `429` past it, so a page-load loop cannot be charged to the
  `/pair` window (6/min) or the `/info` window (60/min). The page's own
  `/info` and `/pair` calls count against theirs as today.
- **Before pairing the listener serves `/info`, `/pair` and the page.**
  §5's "Require pairing before anything but `/info` and `/pair`" is
  amended to add the page. The page's bytes are identical on every
  installation and carry no host data (no id, no key, no name): the
  host-specific values arrive only in the URL the owner scanned and in
  `/info`.
- **Opened with neither code nor marker:** a plain message: "Open
  Remote devices in Switchboard on the Mac, choose Allow pairing, and scan
  the code with this device's camera." No code-entry form in this cut: a
  typed code without the scanned key would skip the one check the page
  can make. A paste field is the natural follow-up.
- **`refuse` and the code.** The page request never reaches the frame, so
  `refuse` (`src/app.rs:3754`, logs method and path only) is not on the
  path; the static arm logs nothing and never logs the query.

### The page's protocol (`web/app.js`)

Order on load: (1) if `location.search` has `code`, hold `code`, `key`,
`id` in variables and call `history.replaceState(null, "",
location.pathname)` as the first statement; (2) `GET /api/v1/info`;
(3) with a code: refuse to continue unless `info.spki_sha256 === key` and
`info.server_id === id`, else say "this is not the host whose code was
scanned" and send nothing; (4) `POST /api/v1/pair {code, device_name,
spki_sha256, transport: "cookie"}`; on success store the marker and
continue, on `403 pairing_closed|wrong_code|wrong_key` say which, plainly;
(5) with no code and a marker: if `info.server_id` equals the marker's and
the key differs, "identity changed, pair again" and send nothing; (6)
`GET /api/v1/device` for the name and current scope; a 401 is the revoked
or never-paired state above.

After that it speaks §4.2 and §4.4: every reply is the envelope
`{epoch, revision, body}`; one held revision per resource key (a project's
session list, one session, one session's `conversation` or `screen`); a
reply is applied only at or past the held revision; a changed `epoch`
refetches everything. Subscribe first, then snapshot: open
`wss://<host>/api/v1/events`, wait for `hello`, fetch `projects` and each
`projects/{id}/sessions` (at most 4 in flight), then apply buffered events
by revision. `changed {session}` refetches that session and re-sorts;
`changed {sessions, project}` refetches that project; `changed {projects}`
refetches the list; `removed` drops it; `resync_required` reconnects and
refetches. Opening a session sends `watch {kind: "conversation", id}` for a
Claude session or `watch {kind: "screen", id}` otherwise, waits for
`watching`, then fetches `last-response` or `screen`; leaving sends
`unwatch`. Refetch the open view every 30 s regardless (§4.4, covers
eviction). Without the socket, every poll refetches the list and the open
view; the held-revision rule is the same.

A reply is `POST sessions/{id}/send {op, text}` under `control`, with the
op made per command. On a timeout the page asks `GET ops/{op}` and follows
§4.5: `done` uses the reply, `interrupted` shows the session's state and
never resubmits, `unknown` resubmits with the same op. A session with
`trust_question` shows "Waiting at Claude Code's folder trust question;
answer it on the Mac" and no box; one with `prompt_refusal` shows the box
disabled with that reason. Trust, resume, new session, the draft,
dictation, Dispatch tickets and the conversation beyond the last response
are not on the page (owner question; the API serves them).

### `GET /api/v1/device`

Authenticated, any scope. Reply, in the envelope: `{device_id, name,
scope, server_id}`, read from the core's current device, so a scope change
in the dialog shows on the next call. Handled in `serve_remote`
(`src/app.rs:3017`) after `authenticate` and before the allowlist match,
beside `events`, with no `wire::Body`. No fixture: Swift does not decode
it (the fixtures exist for Swift and Rust to agree, and nothing on the
Swift side reads this).

## The actions and effects

One new action, `AppAction::CertificateReissued { certificate: String,
names: Vec<String>, expires: SystemTime }`, handled in
`src/core/remote.rs` through the existing `update_identity`
(`:105-117`), which already emits `Effect::SaveIdentity` and does nothing
when read-only. No new effect, no new port method: `RemoteStore` already
saves the identity.

Adapter: `remote::reissue_certificate(identity, key_pem, names, now) ->
Result<(String, SystemTime), String>` beside `make_identity`
(`src/adapters/remote.rs:95`), over `KeyPair::from_pem`, with the SANs,
`serverAuth`, `not_before` a day back, `not_after` 800 days out; and
`remote::local_names()`, the sorted names above (the `.local` name from
`scutil --get LocalHostName`, absent when the command fails or off macOS,
so Linux CI compiles and passes). In `bind_remote` (`src/app.rs:2935`),
before `tls_config` at `:2957`: if `identity.certificate_names` differs
from `local_names()`, or `certificate_expires` is `None` or within 30 days,
call `reissue_certificate` with the Keychain key already fetched there,
dispatch `CertificateReissued`, and build the TLS config from the core's
identity afterwards. `make_identity` also fills the two new fields. An
address that changes while the port is running is not noticed until the
next bind; that is a known gap in the design status, not machinery here.

Serving the page is not an effect: it never leaves the connection thread.
The request-side changes (three headers, `set_cookie`, the Origin check)
are in the adapter and `authenticate`, with no action.

## The UI

**The pairing window** (`src/ui/remote.rs:155-215`). The `json!` payload
and `qr_texture` input become `core::remote::invite_url(...)`; "Copy
invite" copies the URL and its hover says "The QR code's link, to open in
a browser or paste into a client". The caption keeps the key and code and
gains the trust text from decision 1 (two sentences: the browser will warn
about this host's certificate; accepting it is the step that trusts this
network and does not verify the host; the key in the link only stops a
page from sending the code to a different Switchboard; the link works
once and expires in five minutes or three wrong codes). The paired-devices
section (`devices`, `:230`) gains one line under the table: a browser's
token is an HttpOnly cookie sent to any HTTPS service on this Mac's
address. It uses `GAP` and `PAD`; no logic beyond the format call.

**The page** (`web/index.html`, `app.js`, `app.css`), one file each, no
framework. A header with the host's name (from `/info`), the device's
name and scope word (from `/device`), and the live/polling indicator.
Below: a "Waiting on you" group first (every session with `waiting`,
marked, with `waiting_reason`), then one group per project; each row has
the session name, kind, `card` word and the excerpt. A session view with
the last response (Claude) or screen text (anything else), refreshed on
`changed`, and the reply box under `control`, absent for `read`. States,
each one sentence: not paired, pairing closed, wrong host, identity
changed, no longer paired, host unreachable, live updates unavailable,
`read` scope. Text only, `textContent` never `innerHTML`, since session
text is attacker-influenced.

## The tests

CI cannot drive a browser and the tree has no JS runner (the issue rules
out a dependency), so the page's reconcile logic is checked by hand. The
host's side is tested in full.

- **`src/core/tests.rs`:** `invite_url` for an IPv4 and an IPv6 address;
  `CertificateReissued` updates the identity and returns one
  `SaveIdentity`; it is a no-op read-only.
- **`src/adapters/remote.rs` unit tests:** `reissue_certificate` keeps
  `spki_sha256` (`spki_sha256_of_cert` equals the key's) and `tls_config`
  accepts it; `RemoteRequest::token` prefers the bearer, reads only the
  cookie with the right name, rejects an oversized value; the same-origin
  rule (equal, different port, different scheme, missing, case).
- **`tests/remote.rs`** (extend `http`, `:245`, to return the headers; keep
  the old signature as a wrapper so existing callers do not change):
  - the three paths answer unpaired with the exact `Content-Type`, the
    CSP above, `Cache-Control: no-store`, `Referrer-Policy`, `nosniff` and
    `Connection: close`; `/favicon.ico`, `/web/app.js` and `/%2e%2e/` give
    404; `POST /` gives 405; `GET /?code=ABCDEFGH` serves the page and the
    frame sees nothing (the port's `poll` returns no request).
  - **The embedded page references only paths the host serves:** every
    `src`/`href` in the HTML is one of the three paths or the `data:` icon;
    no inline `<script>` body, `on…=` or `style=` attribute, no `http(s)://`
    URL at all; every `/api/v1/…` literal in `app.js` (a `${…}`
    replaced by a UUID) answers anything but `404 not_found` for a
    control-scope device.
  - **Rate window and pool:** 61 static GETs in a minute give a 429 on the
    61st, and `/pair` still answers; ten sequential page loads leave no
    unauthenticated slot held, so a pairing right after them works. The
    existing `stalled_strangers_fill_their_own_pool_and_never_a_paired_clients`
    (`:3226`) stays as is.
  - **Cookie transport:** `/pair` with `transport: "cookie"` returns no
    `token` and a `Set-Cookie` with each attribute named above and the
    per-`server_id` name; the cookie alone then reads `waiting` and
    `device`; a `send` with the cookie and no `Origin`, or with another
    origin, is `403 forbidden_origin` and sends nothing; with the matching
    `Origin` it goes; an upgrade with the cookie and a wrong `Origin` is
    403; a bearer request with no `Origin` is unchanged. Pairing without
    `transport` returns the token as today.
  - **`/device`:** returns the name and scope; after `SetDeviceScope` the
    next call shows the new one; after revoke the cookie gets `401
    unauthorized` and the socket closes (beside
    `revoke_closes_the_devices_socket`, `:2892`).
  - **The pairing flow as the CLI does it:** parse `invite_url`'s output,
    GET `/info`, compare the key, POST `/pair`, read `/device` and
    `/projects`; this is the acceptance flow minus the browser.
  - **The reissued certificate:** a rustls client that trusts the
    certificate as its own anchor completes a handshake for `ServerName`
    `localhost` and for the IP SAN; the existing pinned config connects
    unchanged (the pin did not move). If webpki refuses a self-signed leaf as
    its own anchor, the implementer asserts the SANs another way (a dev
    dependency that decodes the DER, named in the commit) rather than
    dropping the test.
- **`tests/ui.rs`** (beside `remote_devices_dialog_opens_pairing_and_revokes_a_device`,
  `:7424`): with a pairing window seeded, the dialog shows the trust text,
  the key and code, and "Copy invite" with `record_actions` copies a text
  that starts `https://` and carries `code=`.
- **Swift** (`client/Tests`): `PairingInvite.parse` of a URL (IPv4, IPv6
  with brackets, missing `code`), of the old JSON, and of garbage; the
  CLI usage text. Run `swift test` in `client/` where the toolchain is
  present.
- **By hand, once, recorded in the design status:** the issue's
  acceptance on a phone: scan, accept the warning, see the sessions, a
  session starting to wait moves to the top without a reload, a reply
  arrives, revoke in the dialog and the next request says so. Order:
  (1) the cookie test of decision 2 first; (2) whether `new WebSocket`
  connects (Owner A); (3) the pool during the page load (Unresolved 8);
  (4) `connect-src 'self'` and `wss`. With
  `SWITCHBOARD_DATA_DIR` short, `SWITCHBOARD_REMOTE_PORT` set, and the
  script lines `remote-listen lan <port>` and `pairing <code>` (no new
  script line is needed).

## Order of work and docs (one commit; the app runs at each step)

1. `RemoteRequest`/`RemoteReply` fields, `token`, the Origin rule,
   `/device`, `transport: "cookie"` (native paths unchanged).
2. Static paths on the connection thread, the rate window, the 404 for
   strays; `web/` files as a stub page that loads.
3. `Identity` fields, `reissue_certificate`, `bind_remote`.
4. `invite_url`, the dialog, `PairingInvite.parse`, the CLI usage text.
5. The page.
6. Docs: `docs/remote-client.md` §4.1 (static pool and window,
   `Connection: close`), §4.3 (table rows for the page and `/device`), §5
   (QR is a URL, the browser trust step and what the key check does not
   prove, the cookie, the Origin rule, the reissue); `docs/design.md`
   "Remote client" status (what is built, plus the gaps: address change
   while running, no code-entry form, no `.local` URL, polling when `wss`
   fails, the manual run's findings); `README.md` Layout rows for `web/*`
   and the changed rows for `remote.rs` (core, adapters, ui) and
   `tests/remote.rs`, the dev-aid text for "Copy invite" (now a link; the
   CLI still takes it) in the `SWITCHBOARD_REMOTE_PORT` entry;
   `client/README.md` for the CLI's argument; `CHANGELOG.md` Unreleased:
   "The remote port serves a web page: scan the pairing window's QR code
   with a phone to see the sessions, which are waiting, and to reply."

## For the owner (follow-ups, not in this commit)

- **A (Safari, `wss`):** if the phone run shows `wss://` fails after the
  warning, install-the-certificate (a download route or a profile) is the
  follow-up. It is a second trust mechanism and changes §5.
- **B (`.local`):** if `https://<name>.local:<port>/` resolves from the
  phone, put it in the URL. The certificate already carries it.
- **D (cookie):** if the cookie fails on `fetch` in Safari with an IP
  host, the fallback is a follow-up decision, not a second transport here.
- Cookie reach across ports on one host (Settled): is it acceptable, or
  should the page move to a `.local` name so the cookie is host-scoped to
  one name?
- A code-entry form, and Dispatch tickets, trust, resume, new session and
  the draft on the page.

## Commit message

`Serve a browser page on the remote port: pairing by link, cookie token, a /device read, a reissued certificate`

## Contract additions

- `RemoteRequest` (adapter, not the shared contract): `cookie`, `origin`,
  `host` and `token()`; `RemoteReply`: `set_cookie` and `Default`.
- `GET /api/v1/device` → `{device_id, name, scope, server_id}` (spec §4.3).
- `POST /api/v1/pair` request field `transport` (`"cookie"`); reply omits
  `token` and sets `Set-Cookie: __Host-sb-<server_id>`.
- Error code `forbidden_origin` (403).
- Static `GET /`, `/app.js`, `/app.css` on the remote port (spec §4.1, §5).
- `Identity.certificate_names`, `Identity.certificate_expires` (both
  `#[serde(default)]`); `IDENTITY_SCHEMA_VERSION` = the value in
  `src/ports/remote.rs` at implementation time, plus one.
- `AppAction::CertificateReissued { certificate, names, expires }`; no new
  `Effect`.
- `core::remote::invite_url`; `remote::reissue_certificate`,
  `remote::local_names`.
- Pairing QR payload: an `https://` link replaces the JSON object;
  `PairingInvite.parse` (Swift) accepts both.
- No `model.rs`, `SessionView`, `control/` or fixture change.

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
