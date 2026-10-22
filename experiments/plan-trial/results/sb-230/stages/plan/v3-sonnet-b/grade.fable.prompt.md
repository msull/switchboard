You are scoring one implementation plan for a GitHub issue, blind: you do not know who or what wrote it, and you score this plan on its own.
The repository at the issue's base commit is checked out at /Users/sully/code_repos/plan-trial/sb-230-base (read-only; change nothing there). Every file path in the plan is relative to that tree.

Score by the rubric, column by column. Length is not thoroughness. For grounding, take the first five distinct file or line references the plan makes, in order of appearance, open each in the tree, and say what you found. For departures and decisions, use the reference as the yardstick; where you disagree with the reference, say so in the evidence rather than silently scoring your own view.

Your final message must be exactly this shape and nothing else:

```
| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| v3-sonnet-b | <0-3> | <0-3> | <0-3> | <0-3> | <yes|note|no> |
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

# The plan (v3-sonnet-b)

# Plan: a web control surface on the remote port (#230)

Line numbers are as they are in this worktree at 7fe8949. The facts file
is cited as `facts.md` with its section; "Settled" lines are used as
written, "Unresolved" and "For the owner" lines are named where a decision
rests on one.

The default everywhere below is the design that works whichever way the
open questions are answered: the page pairs and reads over `fetch` with a
bearer header, and treats the events socket as an upgrade it may lose. The
cookie transport and the socket are built, and switch on by a constant
once the owner has run the page on a phone.

## Where this departs from the issue

- **"The one-line excerpt the card shows" does not exist.** The notes and
  `facts.md` (Settled, The tree) say `last_message` is on the session
  record. It is not. It is a field of the hook event
  `EventKind::Stopped { last_message, .. }` (`src/ports/events.rs:24`) and
  the core drops it: `src/core/events.rs:132` matches
  `Stopped { pending, .. }`. `SessionRecord` has no such field and
  `SessionView` (`control/src/lib.rs:637-688`) has none either. So the
  list shows what the wire has: kind, `card` (the card's word), `waiting`
  and `waiting_reason`. The last response appears only in the session view.
  No new read model, no `SessionView` field, no fixture change.
- **"The device's scope in the header" has no source after pairing.** No
  endpoint returns it (`remote_info`, `src/app.rs:3568`, is unauthenticated).
  The page keeps the scope from the `/pair` reply and corrects it when a
  `send` is refused with code `scope` (`src/app.rs:3166-3171`).
- **"Revoked" cannot be told from "never paired" by the code.** Both
  refusals in `authenticate` (`src/app.rs:3127-3152`) are `unauthorized`.
  The page keeps a non-secret marker (server id, key hash, device id, name,
  scope). A 401 with a marker says "this device is no longer paired"; a
  401 without one says "open the pairing window on the host". The host
  adds no code.
- **"Any path outside `/api/v1/` is refused" is true, but late.**
  `serve_remote` does it (`src/app.rs:3023`) after the request took a slot
  in the frame's inbox of 64. The static paths and the 404 for every other
  non-API path are now answered on the connection thread.
- **§5's relay sentence does not hold for a browser.**
  `docs/remote-client.md` §5 says the host's own key check in `/pair` makes
  a relayed request fail. `remote_pair` (`src/app.rs:3636`) compares the
  key in the request body, and under a relay the body comes from the
  impostor's page. The page's `key` check catches a genuine page talking to
  the wrong genuine host (another installation at that address, a rotated
  key). It cannot see the certificate it is on. The spec text is amended
  to say so.
- **`facts.md` says `RemoteReply` is built at about 15 sites.** The tree
  has 11 struct literals: ten in `src/app.rs` (`:1664`, `:1679`, `:2365`,
  `:2371`, `:2376`, `:2754`, `:3570`, `:3723`, `:3736`, `:3767`) and one in
  `src/adapters/remote.rs:829`. No file under `tests/` builds one.
- **An existing test changes.** `tests/ui.rs:7474-7490` parses the copied
  invite as JSON; it becomes a URL.

Decisions the issue asks the plan to settle:

1. **Trust: the click-through is the trust step, said in the dialog; the
   certificate is not offered for download.** A download only helps if the
   device then trusts a manually installed certificate, which needs the
   certificate to meet Apple's rules (`facts.md`, Settled, Apple's TLS
   rules; the tree confirms the current one meets none of SAN, `serverAuth`
   or 825 days: `make_identity`, `src/adapters/remote.rs:102-110`). Whether
   Safari lets the owner proceed at all, and whether the exception then
   covers `fetch` and `wss`, is Unresolved 1 and 2 and For the owner A; the
   plan does not wait on them. It reissues the certificate over the same
   key with the IP and the `.local` name as SANs, `serverAuth` and a
   validity of at most 825 days (Unresolved 1, 2, 3: helps in every
   branch, costs nothing if unneeded). A profile or a download is a later
   issue if A says the click-through is not enough. §5's rule becomes: no
   non-loopback bind serves anything but `/info`, `/pair` and the page's
   three files before pairing; the files are the same bytes for everyone
   and carry no secret.
2. **Token transport: the bearer header on `fetch`, a subprotocol value on
   the socket, the cookie built behind them.** The default needs no browser
   behaviour that `facts.md` leaves open: headers on `fetch` work
   everywhere, and a base64url token is a legal subprotocol token (Settled,
   RFC 6455). The cookie needs Unresolved 5 and For the owner D to be
   favourable, so it is the enhancement (`TRANSPORT` in `app.js`, `'token'`
   by default). The token sits in `localStorage` under the default,
   readable only by this origin's scripts: the CSP (no inline script, no
   external one) leaves the page's own script as the only reader. Under the
   cookie it is not stored at all.
3. **One QR, the URL.** Nothing generates the JSON any more. The Swift
   parser keeps decoding JSON (the visionOS paste field and old pasted
   text), so no client breaks.
4. **Static paths** are in The signals below.
5. **Where the page lives and how it is tested:** `web/`, embedded; the
   tests are in The tests.
6. **The device record: the name only.** The page sends `device_name` from
   its user agent ("Safari on iPhone"); `new_device` already cuts it to 64
   characters (`src/core/remote.rs`, `new_device`). No `kind` field.

## The record

- **`identity.json` gains two additive fields**, `certificate_names:
  Vec<String>` and `certificate_expires: Option<SystemTime>`, both
  `#[serde(default)]`, so the host knows what the stored certificate
  covers without parsing it. `IDENTITY_SCHEMA_VERSION`
  (`src/ports/remote.rs:17`) is bumped: **the value in that file at
  implementation time, plus one** (it is 1 today; a ticket landing in
  parallel may take the next number first). An older file reads with empty
  names, which means "reissue at the next bind". `RemoteFiles::load_identity`
  (`src/adapters/remote.rs:74`) already refuses a newer file. `model.rs`
  and its `SCHEMA_VERSION` (16 today) are untouched; `devices.json` is
  unversioned and unchanged.
- **Rehydration.** The key stays in the Keychain under
  `Identity::key_account`. `bind_remote` (`src/app.rs:2936`) already reads
  the key and builds the TLS config; it now first compares the identity's
  names and expiry with what this machine answers to (the outward IP from
  `local_address()`, `127.0.0.1`, `localhost`, and `<LocalHostName>.local`)
  and a 30-day margin. On a miss it reissues over the same key with
  `rcgen::KeyPair::from_pem` (as `spki_sha256_of_key` does,
  `src/adapters/remote.rs:138-141`), so `spki_sha256` and every pairing are
  unchanged (`facts.md`, Settled: the certificate can be reissued over the
  same key). It saves `identity.json` through the existing atomic path
  directly, as `make_identity` in `app.rs` does at load, then dispatches
  `AppAction::CertificateReissued`. It does not go through
  `Effect::SaveIdentity`: that arm rebinds when `remote_bound` differs
  (`src/app.rs:2895-2898`), and `bind_remote` has just cleared it.
- An address that moves after the bind is not covered until the next
  bind. The pairing dialog says so when `local_address()` is not in
  `certificate_names` and names the fix (turn "Local network" off and on,
  which rebinds); the page's warning interstitial is then the one the owner
  already knows (Unresolved 7: an address change is another warning).
- The page keeps one non-secret marker in `localStorage`: server id, key
  hash, device id, name, scope, host name. The token is a second entry
  under the default transport.

## The signals

No new event. The page speaks the existing protocol: `GET info`, `POST
pair`, `GET waiting`, `GET projects`, `GET projects/{id}/sessions`, `GET
sessions/{id}/last-response` (`screen` for a non-Claude kind), `POST
sessions/{id}/send`, `GET ops/{op}`, and the `events` socket with `watch`
frames for the content key (`conversation` for `last-response`, `screen`).

- **Reads and revisions.** Subscribe first, then snapshot (§4.4). Each
  reply carries `epoch` and `revision`; a reply older than what the page
  holds for that resource is dropped, a new `epoch` refetches everything,
  `resync_required` reconnects and refetches.
- **Polling is the floor, the socket the upgrade** (`facts.md`
  Unresolved 1; the plan never waits on it). The list and the open
  session are refetched every 3 s when no socket is connected and every 30
  s when one is (§4.4's visible-view refetch). A socket that fails to
  open, or closes, shows "live updates unavailable" and the page carries
  on polling; it retries the socket every 30 s. Acceptance step 4 (a
  session moves to the top without a reload) therefore holds on either
  branch, within 3 s without the socket and at once with it.
- **Operation ids.** `crypto.randomUUID()` only when
  `window.isSecureContext`, else 16 bytes of `getRandomValues` as hex
  (Unresolved 4). A `send` that times out is checked with `GET ops/{op}`,
  never resent (§4.5).
- **The pairing URL.**
  `https://<ip>:<port>/?code=<code>&key=<spki_sha256>&id=<server_id>`, an
  IPv6 host in brackets. The host part is the outward IP from
  `local_address()`; the `.local` name goes in only if the owner answers
  For the owner B favourably, a one-line change in the dialog, since the
  certificate already covers both (Unresolved 9). On load the page, before
  any request: reads the query, calls `history.replaceState(null, '',
  location.pathname)`, then `GET /api/v1/info`, then compares `key` with
  `spki_sha256` and `id` with `server_id`, and posts `/pair` only on a
  match. On a mismatch it says "this is not the host whose code was
  scanned" and sends nothing. With a marker whose server id matches but
  whose key differs from `/info`'s, it says "identity changed, pair again"
  and sends nothing, token included (§5).
- **The code in history.** Unresolved 6 and For the owner C: whether the
  browser records the URL before `replaceState` is unknown. The plan
  assumes it does. The code is single use and the window closes at the
  first pairing, three wrong codes or five minutes (`facts.md`, Settled:
  `src/core/remote.rs:46-72`), so a recorded URL is dead after pairing; the
  dialog says that. The connection-thread handler never logs a query
  (`read_request` has already split it off, `src/adapters/remote.rs:495`)
  and `refuse` logs path only (`src/app.rs:3762-3766`).
- **Static paths, on the connection thread** (the frame does core state
  only, §4.1): `GET /` (`text/html; charset=utf-8`), `GET /app.js`
  (`text/javascript; charset=utf-8`), `GET /app.css` (`text/css;
  charset=utf-8`). Each answers with `Cache-Control: no-store`,
  `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`,
  `Connection: close`, and `Content-Security-Policy: default-src 'none';
  script-src 'self'; style-src 'self'; connect-src 'self'; base-uri 'none';
  form-action 'none'; frame-ancestors 'none'`. No inline script or style,
  no external resource. If a browser's `'self'` does not cover `wss:`, the
  socket is blocked and the page is on its polling floor, which it already
  handles. Other methods on these paths: 405 with `Allow: GET`. Any other
  path outside `/api/v1/` (a `/favicon.ico` included): 404 with the
  existing `not_found` body, also on the thread. Paths under `/api/v1/`
  go to the frame as today.
- **Pool and rate (`facts.md` Unresolved 8).** A static request is
  answered with `Connection: close`, so its slot in the pool of four
  (`MAX_UNAUTHENTICATED`, `src/adapters/remote.rs:567`) is freed as soon as
  the reply is written; a page load is three short requests. Static paths
  get their own window, `STATIC_PER_MINUTE = 120` in `Limits`
  (`:623-639`), so a reloading browser cannot spend `/pair`'s six a minute
  or `/info`'s sixty. Whether an iPhone's parallel connections still
  overrun the pool at accept (`:699-706`) needs a device; it is on the
  owner's run list, and the plan raises no limit on a guess.
- **Without a code and without a token** the page says: "Open the pairing
  window on the host (Settings, Remote devices, Allow pairing) and scan
  its QR code." No code-entry form: pairing needs the key, which only the
  link carries.
- **Origin.** Cross-site hijack needs an ambient credential, and the
  default transports are not ambient. The host still checks `Origin`
  where an ambient one is in play: on the events upgrade, and on any
  request authenticated by cookie, `Origin` when present must equal
  `https://<Host header>` (the same origin by IP, `.local` name or
  loopback alike), else 403 `origin`. Native clients send no `Origin` and
  are unaffected.

## The actions and effects

- `AppAction::CertificateReissued(Identity)`: the core replaces the
  stored identity's certificate, names and expiry, keeping `server_id`,
  `server_name`, `listen` and `spki_sha256`. It pushes no effect: the file
  is already written (above). A core test in `src/core/tests.rs`.
- No new `Effect` and no new port method: the reissue uses
  `services.remote.save_identity` and `services.secrets.get`, which exist.
- **Adapter (`src/adapters/remote.rs`).**
  - `RemoteRequest` (`:360`) gains `host`, `origin`, `subprotocols:
    Vec<String>` and `cookie_tokens: Vec<String>`. `read_request`
    (`:444-514`) fills them: `Sec-WebSocket-Protocol` split on commas, a
    `bearer.<token>` entry taken as the token when no `Authorization`
    header is present, and every cookie named `sb_*` collected (the rest of
    the `Cookie` header is dropped, never stored or logged, since cookies
    are shared across ports on a host, RFC 6265 per `facts.md`). Only the
    adapter builds `RemoteRequest` (`facts.md`, Settled), so nothing else
    changes. It is not in `ports/` or `model.rs`, so CLAUDE.md's additivity
    rule does not name it; the additions are additive anyway.
  - `RemoteReply` (`:389`) gains `headers: Vec<(String, String)>` and a
    constructor `RemoteReply::new(status, body, authenticated)`; the 11
    literals above call it. Only `/pair` sets a header (`Set-Cookie`).
  - `respond` (`:530`) takes extra headers; a new `respond_static` writes
    the three files. `respond_upgrade` (`:519`) echoes
    `Sec-WebSocket-Protocol: switchboard.v1` when the request offered it
    (RFC 6455: the client fails on a subprotocol it did not offer, and the
    server never echoes the token entry; `facts.md`, Settled). A native
    client offers none and gets none.
  - `connection` (`:770`) answers static and non-API paths before the
    frame, after the rate check.
  - `make_identity` (`:95`) and a new `reissue_certificate(identity, key_pem,
    names, now)` share one certificate builder: SANs, `serverAuth`,
    `not_after` = now + 825 days minus a day. `time` may need to become a
    direct dependency for `OffsetDateTime`; check whether `rcgen` re-exports
    it. `certificate_names()` and `invite_url(ip, port, code, key,
    server_id)` are pure functions beside `local_address()` (`:962`).
- **Router (`src/app.rs`).**
  - `authenticate` (`:3127`) reads `request.bearer`, then
    `cookie_tokens`, and applies the origin rule for the cookie case.
  - `remote_subscribe` (`:2723`) applies the origin rule.
  - `remote_pair` (`:3606`) honours an optional `"cookie": true` in the
    body: the reply keeps the token in the body (native clients read it) and
    adds `Set-Cookie: sb_<first 8 hex of server_id>=<token>; HttpOnly;
    Secure; SameSite=Strict; Path=/api/v1/; Max-Age=31536000`. The name
    carries the server id so two instances on one Mac do not clash.
  - `bind_remote` (`:2936`) reissues first.
  - The `/api/v1/` prefix arm (`:3023`) stays as the backstop.
- **New files:** `web/index.html`, `web/app.js`, `web/app.css`, and
  `src/adapters/web.rs` (`include_str!("../../web/...")`, one lookup
  function from method and path to a file, and the header set). No build
  step, no dependency for the page.

## The UI

- **The page** (`web/`): the header shows the host name and the device's
  scope; sessions are grouped by project, waiting ones first and marked,
  each row with its kind, `card` word and `waiting_reason` (`hide_workers`
  already applies, `src/app.rs:2687`); a session view shows the last
  response (Claude Code) or the screen (anything else), refreshed per The
  signals; a reply box posts `send` when the scope is `control` and is
  hidden otherwise. Plain statements for "identity changed, pair again",
  "this device is no longer paired", "live updates unavailable". A
  `prompt_refusal` or `trust_question` session shows its reason and no
  reply box; resume and the trust answer are not in this cut.
- **The pairing window** (`src/ui/remote.rs:155-206`): the QR and "Copy
  invite" carry the URL, the `json!` call at `:160-166` is replaced by
  `remote::invite_url`, and the address label shows the URL's host and
  port. Under it, three sentences: a browser shows a certificate warning
  once, and accepting it on the local network is the trust step (the page
  compares the key but cannot see the certificate it is on); the link holds
  the code, which works once and for five minutes, so a copy left in the
  browser's history is dead after pairing; the certificate is renewed over
  the same key, so a renewal does not unpair anyone. When
  `local_address()` is not in `certificate_names` it adds the restart note.
- **The paired-devices list** (`:230-275`) needs no change: a browser is a
  row with a scope selector and Revoke, and revoking closes its socket
  (existing test `revoke_closes_the_devices_socket`, `tests/remote.rs:2892`).
- **Swift text path:** `PairingInvite.parse`
  (`client/Sources/SwitchboardProtocol/Info.swift:93-116`) tries a URL first
  (`URLComponents`: host and port into `addresses`, `code`, `key`, `id`)
  and JSON second, so `main.swift:111`, `BrowserView.swift:106,883` and
  every `PairingInvite(...)` call (`facts.md`, Settled: it is built as a list
  of one) work unchanged. The CLI usage string (`main.swift:13`) and the
  README's "Copy invite" text say the QR holds a URL.

## The tests

Rust (the commands in CLAUDE.md; Linux CI runs them):

- `src/adapters/remote.rs` unit tests: a reissued certificate keeps
  `spki_sha256` (`spki_sha256_of_cert` equals the key's hash); its DER
  holds the SAN names and the `serverAuth` OID; `certificate_expires` is at
  most 825 days out; reissue is skipped when names and margin are met;
  `invite_url` for IPv4 and a bracketed IPv6 host; `read_request` takes
  the three token transports and drops other cookies.
- `src/core/tests.rs`: `CertificateReissued` replaces certificate fields
  and keeps `spki_sha256`, `server_id` and `listen`, and returns no
  effect.
- `tests/remote.rs`: extend `http` (`:245`) to return headers (a thin
  wrapper keeps current callers). Then: the three paths answer with the
  headers named above; a path with `?code=` answers the page and reaches no
  frame; other methods get 405; `/favicon.ico` gets 404 without a frame
  pass; the static window answers 429 past 120; four sequential static
  loads leave the pool free (model: `stalled_strangers_fill_their_own_pool_and_never_a_paired_clients`,
  `:3226`); the embedded `index.html` names only `/app.js` and `/app.css`,
  has no inline script and no `http` URL, and every `/api/v1/...` literal
  in `app.js` is an endpoint the host serves. The pairing flow as the page
  does it, with the host's own client as the CLI does: `/info`, key
  compare, `/pair` with a browser-style `device_name`, a read with the
  bearer header; a socket upgrade offering `switchboard.v1, bearer.<token>`
  gets 101 echoing `switchboard.v1` and not the token (extend
  `subscribe_at`, `:1960`); `/pair` with `"cookie": true` sets the cookie
  with `HttpOnly; Secure; SameSite=Strict`, a cookie-authenticated read
  works, a cookie-authenticated request with a foreign `Origin` gets 403,
  a bearer request with a foreign `Origin` is unaffected; after
  `RevokeDevice` the next request is `unauthorized` and the socket closes.
  `pair_is_rate_limited_per_listener` (`:1406`) and
  `the_allowlist_refuses_everything_else_with_no_side_effect` (`:682`) stay
  as they are and still pass.
- `tests/ui.rs:7474`: the copied invite is a URL with `code`, `key` and
  `id` equal to the window's code, the identity's key hash and server id;
  the dialog shows the trust sentences.
- No fixture change: no view or wire body changes. The Swift fixtures stay
  as they are.

Swift (`client/`, run as `client/README.md` says; not run by CI on
Linux): `FixturesTests.swift:156` gets URL cases beside the JSON one, an
IPv6 host, and a URL with an unknown query key.

The page's own logic (query parse, `replaceState`, reconcile) has no
runner and the issue rules out a JS dependency, so it is checked by the
manual run below, and the embedded-page test above is the guard against
the page naming a path the host does not serve.

Manual acceptance, against a test data dir and tmux socket
(`SWITCHBOARD_DATA_DIR`, `SWITCHBOARD_TMUX_SOCKET`, script lines
`remote-listen lan <port>` and `pairing <code>`): open the window, scan
the QR with a phone, accept the warning, see the sessions; start a session
waiting with `switchboard-hook PermissionRequest` and watch it move to the
top without a reload (within 3 s if the socket is unavailable); send a
reply from the phone with `control` scope and watch it arrive; revoke the
phone in the dialog and the page says so on its next request. None of
these steps depends on the socket, the cookie or the `.local` name. The
owner's run also answers For the owner A through D, recorded in the design
status section: whether the page loads after the warning, whether `wss`
connects, whether `.local` resolves, whether the cookie works. A, B and D
each flip one constant (`TRANSPORT`, the host in the URL) and add no code.

## Docs, in the same commit

- `docs/remote-client.md`: §4.1 gets the static-file bullet and the
  thread it runs on; §4.3 lists the three paths; §5 changes "Require
  pairing before anything but `/info` and `/pair`" to add the page's files,
  the QR line to the URL, the token line to the three transports, and the
  relay sentence as in the departures; the certificate line gets the SANs,
  `serverAuth` and the 825-day bound, with the pin unchanged by a reissue.
- `docs/design.md`: a "Web control surface" item in the Remote client
  section (starts `## Remote client (2026-10-12)`, `docs/design.md:3107`),
  with the Known gaps at the end of that section (`:3573`) extended: no
  excerpt on rows, scope stale until a refused send, no certificate
  download or profile, no JS tests, an address change needs a rebind,
  Unresolved 1 through 9 and A through D as open until the phone run.
- `README.md`: Layout rows for `src/adapters/web.rs` and `web/*`; the
  `SWITCHBOARD_REMOTE_PORT` paragraph's "Copy invite" text now says URL, and
  that opening it in a browser pairs it; the data directory entry for
  `identity.json` mentions the certificate names.
- `CHANGELOG.md`, under Unreleased: "The remote port serves a web page:
  open the pairing window's QR code on a phone or any browser on the local
  network to see the sessions, which are waiting, and reply."

## Order inside the one commit (the app builds and runs after each)

1. `web/`, `src/adapters/web.rs`, the connection-thread static answers,
   the static window, `RemoteReply::new`, the extended `http` helper.
2. The `RemoteRequest` fields, the three token transports, the subprotocol
   echo, the origin rule, the cookie on `/pair`.
3. The certificate builder, `reissue_certificate`, the two `Identity`
   fields and the version bump, `CertificateReissued`, the `bind_remote`
   call.
4. `invite_url`, the dialog, the Swift parser.
5. The tests and docs above; `cargo fmt --all`, `cargo clippy --locked
   --workspace --all-targets -- -D warnings`, `cargo test --locked
   --workspace`.

## Commit message

Serve a browser page from the remote port, paired from a URL in the pairing QR

## Contract additions (for the commit message body)

- `AppAction::CertificateReissued(Identity)` in `src/core/action.rs`; no
  new `Effect`, no new port method.
- `Identity.certificate_names: Vec<String>` and
  `Identity.certificate_expires: Option<SystemTime>` (`#[serde(default)]`)
  in `src/ports/remote.rs`; `IDENTITY_SCHEMA_VERSION` set to the value in
  that file at implementation time, plus one. No change to `model.rs` or
  `SCHEMA_VERSION`; `devices.json` unchanged.
- `RemoteRequest` fields `host`, `origin`, `subprotocols`, `cookie_tokens`;
  `RemoteReply.headers` and `RemoteReply::new` (both in
  `src/adapters/remote.rs`, outside the shared contract).
- `/pair` body: optional `cookie` (bool). `/pair` reply: optional
  `Set-Cookie`, the body unchanged.
- Events upgrade: a `Sec-WebSocket-Protocol` of `switchboard.v1` is echoed;
  `bearer.<token>` is accepted as a token carrier. Refusal code `origin`
  (403) for a cookie-authenticated request or an upgrade whose `Origin` is
  not the host's own.
- Served paths: `GET /`, `/app.js`, `/app.css`; every other non-API path
  is 404 on the connection thread.
- New files `src/adapters/web.rs`, `web/index.html`, `web/app.js`,
  `web/app.css`; new pure functions `remote::invite_url`,
  `remote::certificate_names`, `remote::reissue_certificate`.
- Swift: `PairingInvite.parse` accepts a URL as well as JSON; no new type
  and no changed initializer.
- Wire bodies, `control/`, and the shared fixtures: no change.

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
