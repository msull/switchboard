You are scoring one implementation plan for a GitHub issue, blind: you do not know who or what wrote it, and you score this plan on its own.
The repository at the issue's base commit is checked out at /Users/sully/code_repos/plan-trial/sb-230-base (read-only; change nothing there). Every file path in the plan is relative to that tree.

Score by the rubric, column by column. Length is not thoroughness. For grounding, take the first five distinct file or line references the plan makes, in order of appearance, open each in the tree, and say what you found. For departures and decisions, use the reference as the yardstick; where you disagree with the reference, say so in the evidence rather than silently scoring your own view.

Your final message must be exactly this shape and nothing else:

```
| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| v3-sonnet-a | <0-3> | <0-3> | <0-3> | <0-3> | <yes|note|no> |
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

# The plan (v3-sonnet-a)

# A web control surface on the remote port (issue #230)

Line numbers are as they are in this worktree at 7fe8949. "Facts" is
`inputs/facts.md`: *Settled* lines are cited as such, *Unresolved* N and
*For the owner* X say so where a decision rests on them.

One rule runs through the plan: **the page works with REST polling, a
token the script holds, and a certificate warning clicked through. Each
thing that needs a favourable answer from a browser or a phone (the
events socket, the cookie, a SAN accepted on an IP) is built behind that
default, chosen at run time by a probe, never assumed.** The acceptance
steps hold on the default.

## Where this departs from the issue

- **"The one-line excerpt the card shows" is not on the wire.**
  `SessionView` (`control/src/lib.rs:636-688`) has no excerpt, and spec
  §4.3 (the `session.last_response` bullet) says the card's excerpt is
  `last_message` and "moves with `changed {session}`", which nothing
  exports. The record has `last_message` (Facts, Settled, the tree; the
  hook writes it at `src/bin/switchboard-hook.rs:37`). Plan: no change to
  `SessionView`, so no fixture change on either side. The list shows
  the card word and `waiting_reason`; for the **waiting** sessions only
  (at most eight at once), the page also reads `last-response` and shows
  its first line. Adding `last_message` to `SessionView` is the later
  fix if the owner wants it on every card.
- **"The device's scope in the header" has no source after pairing.** No
  endpoint returns it (`remote_info`, `src/app.rs:3568`, is
  unauthenticated and carries none), so a scope change in the dialog
  would stay invisible. Plan: an authenticated `GET /api/v1/device`
  answering `{id, name, scope}`, one allowlist arm in `serve_remote`.
- **"Revoked" cannot be told from "never paired" by the host's answer.**
  `authenticate` (`src/app.rs:3125-3153`) refuses both with
  `unauthorized`. Plan: no new code. The page keeps a non-secret marker
  of its pairing (server id, key, device id, name), so a 401 with a
  marker is "revoked or removed on the Mac, pair again" and without one
  is "open the pairing window and scan".
- **"Any path outside `/api/v1/` is refused"** is true
  (`src/app.rs:3023`) but on the frame, after the request was queued. The
  page's files are answered on the connection thread instead, never
  queued, which is also what §4.1 ("the frame does core state only")
  leans toward.
- **The certificate cannot be assumed to be clickable.** The issue says
  the owner clicks through a warning. `make_identity`
  (`src/adapters/remote.rs:95-127`) makes a certificate with no
  subjectAltName, a CN only, and a `not_after` in 4096; Apple's rules
  need a SAN, `serverAuth` and at most 825 days (Facts, Settled, Apple's
  TLS rules), and whether a click-through works at all on this
  certificate is Unresolved 1. Plan: reissue it over the same key (so
  `spki_sha256` and every native pin are unchanged; Facts, Settled, the
  tree) with SANs, `serverAuth` and a bounded validity. Unresolved 1–3
  say this is cheap and helps either way.
- **§5's sentence "the host refuses a pairing whose `spki_sha256` is not
  its own, so a relayed request … fails" does not hold for a browser.**
  A page cannot read the TLS certificate it talks over, so the key it
  compares is what the host's `/info` *says*, and under a relay the
  `/pair` body comes from the impostor's page (`remote_pair`,
  `src/app.rs:3639-3647`, compares what it is sent). The `key` in the URL
  catches a genuine page on the wrong genuine host (a second
  installation at that address, a rotated key); it does not catch an
  impostor. The spec says so (below).
- **`crypto.randomUUID` is secure-context only** (Facts, Settled, secure
  contexts), and Unresolved 4 leaves a click-through origin open. The
  page uses it only when `window.isSecureContext` and otherwise builds a
  v4 id from `crypto.getRandomValues`.
- **The Swift side is bigger than "takes the URL too".** `PairingInvite`
  is `Decodable`-only JSON (`client/Sources/SwitchboardProtocol/Info.swift:93-116`)
  and is used by `ConnectionManager`, `BrowserView`, `switchboard-remote`
  and eight test files (Facts, Settled, the tree). `parse` gains the URL
  form and keeps the JSON form; the struct and `addresses` as a list do
  not change.

## The record

No new record kind, no `model.rs` change: `SCHEMA_VERSION` stays 16
(`src/core/model.rs:12`) and `VIEWS_SCHEMA_VERSION` 7. A browser is one
more `PairedDevice` (`src/ports/remote.rs:96-107`) in the unversioned
`devices.json`, named by the page from its user agent ("Safari on iPhone",
cut to 64 by `new_device`, `src/core/remote.rs:166-175`) and sent as
`device_name`, so no `kind` field is added (decision 6). It is listed and
revoked in the dialog as any device is, and rehydrates as one
(`IdentityLoaded`).

**The one record change is `identity.json`.** `Identity`
(`src/ports/remote.rs:26-42`) gains, both `#[serde(default)]`:

- `certificate_names: Vec<String>`: the SANs the stored certificate was
  issued for;
- `certificate_not_after: Option<SystemTime>`: when it lapses.

`IDENTITY_SCHEMA_VERSION` (1 today, `src/ports/remote.rs:17`) is bumped
to the value in that file at implementation time, plus one; a ticket in
parallel may take the next number first. There is no migrate step
because `identity.json` has none (`RemoteFiles::load` just deserializes,
`src/adapters/remote.rs:55`): an old file loads with empty names, which is
what says "reissue". Written through `RemoteStore::save_identity`'s
atomic path, as now.

**The certificate** is reissued by a new `remote::reissue_certificate(identity,
key_pem, names, now)` beside `make_identity`: the key from
`KeyPair::from_pem` (as `spki_sha256_of_key` does at `:136`), the same
CN, SANs from `names`, `ExtendedKeyUsagePurpose::ServerAuth`, `not_before`
a day back, `not_after` 800 days out (under Apple's 825; Unresolved 3: if
the rule does not apply to a manually trusted leaf nothing is lost). The
names are `localhost`, `127.0.0.1`, `local_address()` when there is one,
and `<hostname -s>.local` (`default_server_name`'s command), so the
`.local` name is in the certificate whichever way For the owner B comes
out; the URL host stays the IP (Unresolved 9). Read the rcgen 0.14 source
under `~/.cargo/registry/src/*/rcgen-0.14*` for how `CertificateParams::new`
sorts a string into an IP or DNS SAN and what `not_before`/`not_after`
take; `time` is in `Cargo.lock` (0.3.55) but not a direct dependency.
`remote::certificate_is_stale(identity, wanted_names, now)` is the pure
test: names differ, no `not_after`, or fewer than 30 days left.

A reissue never changes `spki_sha256`, `server_id`, the Keychain item or
`devices.json`; paired native clients see nothing. A browser exception is
per address (Unresolved 7): an address change means another warning, and
the page and dialog say so.

## The signals

No new event, hook or poll on the host. The page consumes §4.4 as it is:

- **Default (polling).** While visible, the list is refetched every 5 s
  (`GET projects`, then `GET projects/{id}/sessions` per project; a
  `waiting` flag is on each `SessionView`, so a session that starts
  waiting sorts to the top on the next poll, with no reload) and an open
  session's content every 5 s. Each reply carries `{epoch, revision}`;
  the page holds the highest revision per resource and drops an older
  reply, and an `epoch` change refetches everything (§4.4, "Subscribe
  first, then snapshot").
- **Upgrade (events socket, Unresolved 1 and For the owner A).** The
  page opens `wss://<host>/api/v1/events` and, if it opens, follows §4.4
  exactly: wait for `hello`, snapshot, apply buffered events by
  revision, `watch {kind: "conversation" | "screen", id}` for the open
  session and fetch only after `watching`, resend watches after a
  reconnect, `resync_required` refetches all. With the socket live,
  polling slows to the spec's 30 s refetch and an event refetches at
  once. If the socket does not open, or closes and will not reopen after
  three tries, the page falls back to the 5 s poll and says "live updates
  unavailable", never an error. Closing a session view sends `unwatch`.
  The watch set is capped at 128 keys for the whole port (§4.1), so the
  page watches only the one open session.
- **Token on the socket.** A browser cannot set a header on
  `new WebSocket()`. The page offers the subprotocols
  `["switchboard.v1", "token.<token>"]` (base64url, so a legal RFC 6455
  token; Facts, Settled, subprotocol), and the host answers 101 with
  `Sec-WebSocket-Protocol: switchboard.v1` only: the client must fail if
  the 101 names a subprotocol it did not offer, and echoing the
  non-secret one means the token is never echoed. In cookie mode (below)
  the page offers `["switchboard.v1"]` alone.

## The actions and effects

**Core (`src/core/`).** `AppAction::CertificateReissued(Identity)` sets
`self.identity` and emits nothing (the app already saved the file).
`Effect::CheckCertificate` is pushed by `OpenPairing` so that the I/O
stays in `app.rs`: the core cannot ask for an address. Both go in
`src/core/remote.rs`, tests in `src/core/tests.rs` (`CertificateReissued`
sets the identity and returns no effects; `OpenPairing` returns
`[CheckCertificate]`; the existing `OpenPairing` tests at
`src/core/tests.rs:11878-11936` are checked for exact-effect assertions
and updated).

**App (`src/app.rs`).**

- `bind_remote` (`:2926`) calls `refresh_certificate` before `tls_config`:
  if `certificate_is_stale`, it reissues, saves with
  `services.remote.save_identity`, and dispatches `CertificateReissued`.
  It does not go through `Effect::SaveIdentity`, whose arm
  (`:2893-2910`) rebinds, which would recurse. The Keychain key is read
  there already (`:2941`).
- `Effect::CheckCertificate` (arm next to `SaveIdentity` in
  `run_remote_effect`, routed at `:680`): when the port is bound to the
  network and the certificate is stale for the current
  `local_address()`, call `bind_remote()`, which reissues. This is the
  only way the address changing under a running app is noticed; the
  rebind drops open connections, which happens only when the address
  really moved.
- `serve_remote` (`:3017`): a new arm `("GET", ["device"])` after
  `authenticate`, answering `{id, name, scope}`; `info` (`:3568`) lists
  `"device"` among `capabilities`. `client/Fixtures/info.json` is
  rewritten with `SWITCHBOARD_WRITE_FIXTURES=1`; the Swift `capabilities`
  is a `[String]` (`Info.swift:8`), so nothing there breaks.
- `authenticate` (`:3125`) reads the token through one reader: `bearer`
  (the header or the subprotocol token; both are held by script, so
  neither is ambient authority), else the cookie named
  `sb-<server_id, simple form>` from the raw `Cookie` header. A token that
  came from the cookie is accepted only when `Origin`, if present, equals
  `https://<Host>`, and for any non-GET request and for the upgrade
  `Origin` must be present and equal; otherwise `403 bad_origin`. A
  same-origin `GET` carries no `Origin` in browsers, which is why GET is
  the exception. SameSite alone is not enough: a service on another port
  of the same host is same-site (Facts, Settled, cookies are not
  isolated by port), and `remote_pair`/`remote_command` parse the body as
  JSON whatever its content type.
- `remote_pair` (`:3606`): when the body has `"cookie": true` the reply
  carries `Set-Cookie: sb-<id>=<token>; Path=/api/v1/; Max-Age=31536000;
  HttpOnly; Secure; SameSite=Strict`. A native client does not send the
  field and gets no cookie. The token is also in the body as now (the
  page needs it in the default mode).
- Nothing logs a query, a cookie or a token: `refuse` (`:3754`) logs
  method, path and code, and the page's files never reach it.

**Adapter (`src/adapters/remote.rs`).**

- `RemoteRequest` (`:360`) gains `cookie: Option<String>`,
  `host: Option<String>`, `origin: Option<String>`, `subprotocol: bool`
  (the client offered `switchboard.v1`); `bearer` also takes a
  `token.<t>` subprotocol value when no `Authorization` header is
  present. `read_request` (`:444`) is built in one place and no test
  constructs the struct (Facts, Settled, the tree), so only the adapter
  changes. `RemoteReply` (`:389`) gains `set_cookie: Option<String>`; the
  literal sites (grep `RemoteReply {` and skip the signatures; `src/app.rs`
  at about a dozen places and `:829` here) get `set_cookie: None`,
  mechanical. Neither type is in `model.rs`, `AppAction`/`Effect` or
  `ports/`, so CLAUDE.md's additivity rule does not name them (Facts,
  Settled); the change is additive anyway.
- `respond` (`:530`) takes the reply's `set_cookie` and adds
  `Cache-Control: no-store` and `X-Content-Type-Options: nosniff` to every
  JSON reply (private data; a browser must not keep it). `respond_upgrade`
  (`:519`) echoes `Sec-WebSocket-Protocol: switchboard.v1` when
  `request.subprotocol`.
- **The page, on the connection thread.** In `connection` (`:763`), after
  the rate check and before the request is queued, `web::page(method,
  path)` answers `GET` of exactly three paths and closes the connection
  (`Connection: close`, so a page load holds a slot of the pool of 4 only
  for the request itself; Unresolved 8 says the real count needs a
  device):

  | Path | Content-Type |
  |---|---|
  | `/` | `text/html; charset=utf-8` |
  | `/app.js` | `text/javascript; charset=utf-8` |
  | `/app.css` | `text/css; charset=utf-8` |

  Every other path outside `/api/v1/` (`/favicon.ico` included) and every
  other method on these three stays what it is today: the frame's 404
  `not_found`. The query on `/` is ignored and never logged. Headers on
  all three: `Cache-Control: no-store` (the page is compiled in and
  changes with the binary), `X-Content-Type-Options: nosniff`,
  `Referrer-Policy: no-referrer`, and
  `Content-Security-Policy: default-src 'none'; script-src 'self';
  style-src 'self'; img-src 'self' data:; connect-src 'self' wss://<Host>;
  base-uri 'none'; form-action 'none'; frame-ancestors 'none'`. The
  `wss://<Host>` source is added because older Safari does not let
  `'self'` cover `wss:`; it is the request's own `Host` when that is
  letters, digits, `.`, `-`, `:` and brackets, and dropped otherwise. No
  inline script, no inline style, no external resource.
- `Limits` (`:621`) gains a `page` window, 60 a minute (about twenty page
  loads), answered 429 past it, separate from `pair` (6) and `info` (60) so
  a page reload loop cannot use up `/pair`. Static replies are cheap and
  need no frame, but they still count in the pool of 4 while they are
  open; that pool is shared with the strangers and `/pair`, as §4.1 says,
  and the failed handshake behind the warning holds a slot up to 5 s
  already.
- The files are `web/index.html`, `web/app.js`, `web/app.css`, embedded
  with `include_str!` by a new `src/adapters/remote_web.rs` (the table
  above, `page()`, the header block). Plain JavaScript, no framework, no
  build, no dependency. `Cargo.toml` has no `include` list, so the new
  directory ships as-is.

**Pairing URL.** `src/ui/remote.rs:155-173` builds
`https://<SocketAddr>/?code=<code>&key=<spki>&id=<server_id>` instead of
the JSON, where `SocketAddr::new(local_address(), port)` brackets an IPv6
address by itself; with no address it falls back to `127.0.0.1`. Code,
key and id are Crockford, hex and a UUID, so nothing needs encoding. The
QR, the text beside it and "Copy invite" all carry this one string
(decision 3: one QR; the JSON form is no longer produced, and the Swift
parser keeps reading it for text copied from an older host).

**Swift.** `PairingInvite.parse` (`Info.swift:111`) tries the URL first
(`URLComponents`: `code`, `key`, `id`, host and port become the single
`addresses` entry) and the JSON second. `switchboard-remote pair` takes
either as its one argument (`main.swift:107-125` unchanged beyond the
usage text); the headset's paste field goes through the same `parse`.

**The page (`web/app.js`), step by step.**

1. First statement: read `location.search` into a local, then
   `history.replaceState(null, "", location.pathname)`. The code is
   never in the address bar after that. Whether the browser recorded the
   URL before the script ran is Unresolved 6; the plan assumes it did,
   and the code is single-use and gone in five minutes or three failures
   (Facts, Settled, the tree), so a recorded URL is dead after pairing.
2. `GET /api/v1/info`. With a code and key in the URL: if `info.spki_sha256`
   or `server_id` differs, show "this is not the host whose code was
   scanned" and send nothing. If the stored marker has this `server_id`
   with another key, show "identity changed, pair again", drop the token
   and send nothing.
3. `POST /api/v1/pair {code, device_name, spki_sha256: key, cookie:
   true}`. `device_name` is built from `navigator.userAgent` and
   `maxTouchPoints` (an iPad reports `Macintosh`). Errors are worded from
   the code: `pairing_closed`, `wrong_code`, `wrong_key`, `rate_limited`.
4. **Transport probe** (Unresolved 5, For the owner D). The token from
   the reply is held in memory. The page probes `GET /api/v1/device` with
   no `Authorization` header, then an events socket with no token. If both
   succeed, the page is in **cookie mode**: the token is dropped and only
   the non-secret marker is stored. Otherwise **token mode**: the token is
   stored in `localStorage` (per origin including port, unlike a cookie;
   readable by script on the origin, which has no third-party script, by
   the CSP) and goes in `Authorization` on `fetch` and in the subprotocol
   on the socket. The mode is stored with the marker. The host accepts all
   three transports whichever is chosen.
5. With neither a code nor a marker: a short page saying "open Remote
   devices on the Mac, Allow pairing, scan the QR code", the host's short
   key from `/info` in the dialog's grouped form, and a field for the
   eight-character code. This is the headset's typed-code flow (spec §5):
   the owner compares the key by eye with the dialog's Key line, and the
   page pairs with the key `/info` showed. With a marker and no code, the
   page goes straight to the list.
6. List: sessions grouped by project, waiting first and marked, each
   with kind, card word, `waiting_reason`, and the first-line excerpt for
   the waiting ones (departure 1). Header: `server_name` from `/info`, the
   device name and scope from `/device` (refetched on every poll, so a
   scope change shows within 5 s), the live/polling note.
7. Session view: `last-response` for a Claude Code session, `screen?lines=60`
   for anything else (spec §4.3: a non-Claude session answers "no
   conversation"). The reply box appears only for scope `control` and
   only when the view has neither `trust_question` nor `prompt_refusal`
   (those show the reason instead, and the trust answer and resume stay on
   the Mac: out of this cut). Send is `POST sessions/{id}/send {op, text}`
   with a fresh op id; on a timeout the page asks `GET ops/{op}` and
   follows §4.5 (`done` use it; `interrupted` never resend and show the
   state; `unknown` resend the same op). It clears the box only on
   success.
8. A 401 on any request: marker present, "revoked, or removed on the Mac;
   pair again" and the stored token is dropped; no marker, the step-5
   page. A 403 `bad_origin` is shown as a plain error.

## The UI

`src/ui/remote.rs` only:

- The pairing window draws the QR from the URL (`:155-173`); the
  "Copy invite" tooltip and the helper line are reworded: "Scan with a
  phone camera to open the page (a browser warns about the certificate
  the first time; check that the key on the page matches the Key line
  here), or type the key and code on the headset. Grants {scope} scope.
  The link works once and for five minutes at most."
- The paired-devices section gains the trust sentence (decision 1): "A
  browser trusts this Mac by the certificate warning you click through
  on the first visit. That encrypts the connection but does not prove
  who answered; the key on the page against the Key line above does.
  The certificate is renewed before it lapses with the same key, and
  moving to another network address asks again."
- A browser appears in the device list by its user-agent name, with scope
  and Revoke, as any device.

**Decision 1: the click-through is the trust step, not a certificate
download.** It buys an encrypted channel to whoever answers at that
address, plus the page's refusal to send the code to a host whose `/info`
key differs from the URL's. It does not authenticate the host against an
impostor serving its own page; only the owner's by-eye key comparison
does, as on the headset. A download would add a fourth thing served before
pairing, needs a profile and the root-trust toggle on iOS, and whether
Apple would accept this certificate that way is Unresolved 1 and 3. The
live-updates cost is on the table in Facts (Safari `wss` after a
click-through, WebKit bug 41419 and the code-server doc); polling is
the answer to it. If For the owner A comes back unfavourable and the
owner wants the socket on iOS, a later issue adds the download. §5's
"before anything but `/info` and `/pair`" becomes "… and the page's three
files", and no non-loopback bind serves anything else before pairing.

## The tests

`cargo test --locked --workspace` and clippy as in CLAUDE.md. No JS runner
is added (the issue rules out a dependency), so the page's logic is
checked by hand (below) and the host side by the tests.

`tests/remote.rs` (the `http` helper at `:245` keeps only the status and
`Content-Length`; a sibling returns the headers, and `subscribe_at`
(`:1960`) takes extra request headers):

- `the_page_is_served_to_anyone_with_its_headers`: the three paths
  unpaired, with the table's content types, the CSP string, `no-store`,
  `nosniff`; `/favicon.ico`, `/index.html` and `POST /` answer 404; the
  query on `/` changes nothing; answered with no `serve_pending` running
  (the connection thread, not the frame).
- `the_embedded_page_references_only_paths_the_host_serves`: every
  `/api/v1/…` string in `app.js` (collected from the `include_str!`
  text, ids replaced by seeded ones) is sent with a paired token and none
  answers `not_found`; `index.html` references only the three paths; no
  `http://` or `https://` URL, no inline `<script>` body, no `on…=` or
  `style=` attribute.
- `a_browser_pairs_over_the_hosts_own_client_as_the_cli_does`: `/info`,
  key compared, `/pair` with `cookie: true`: the `Set-Cookie` carries
  `HttpOnly`, `Secure`, `SameSite=Strict`, `Path=/api/v1/` and the
  server-id name; the token reads `/device` as `read` or `control`; a
  native pair (no `cookie` field) sets none; the code is single-use.
- `the_cookie_is_accepted_only_from_its_own_origin`: cookie-only `GET`
  with no `Origin` 200; with a foreign `Origin` 403 `bad_origin`; cookie
  `POST send` with no `Origin`, or a foreign one, 403 with nothing written
  to the fake host; with `Origin: https://<Host>` it runs; the bearer
  header needs no `Origin`.
- `the_events_socket_takes_the_token_from_the_subprotocol_or_the_cookie`:
  `["switchboard.v1", "token.<t>"]` upgrades, the 101 names
  `switchboard.v1` only and never the token; cookie + matching `Origin`
  upgrades; cookie without `Origin` is refused; the header path of every
  existing socket test is unchanged.
- `revoking_a_browser_refuses_its_cookie_and_token`: after `RevokeDevice`
  both transports answer 401 `unauthorized` and its socket closes (as
  `revoke_closes_the_devices_socket`, `:2892`).
- `the_page_window_is_its_own`: past 60 page requests a minute the page
  answers 429 and `/pair` still answers; `stalled_strangers_fill_their_own_pool_and_never_a_paired_clients`
  (`:3226`) passes unchanged.
- `device_answers_the_scope_it_has_now`: `/device` before and after
  `SetDeviceScope`; `GET /api/v1/device` unpaired is 401; `info` lists
  `device`. `the_client_fixtures_decode_on_both_sides` (`:1829`) passes
  with `info.json` rewritten.

`src/adapters/remote.rs` unit tests: `reissue_certificate` keeps the
`spki_sha256` and the `server_id`; its names, `serverAuth` and a validity
of at most 825 days are checked on the DER (`x509-parser` is in
`Cargo.lock` at 0.18.1; use it as a dev-dependency if it is only
transitive, or `rustls-webpki`'s name check); `certificate_is_stale` over
its four cases; an `identity.json` without the new fields loads; the
`read_request` parse of `Cookie`, `Host`, `Origin` and
`Sec-WebSocket-Protocol`, the bearer taken from a `token.` value, and a
request with all of them unset behaving as today; `respond` writes the new
headers and a `Set-Cookie` only when given.

`tests/ui.rs`: the pairing window shows an `https://` invite with `code=`,
`key=` and `id=`, the new helper line, and the trust sentence in the
devices section.

Swift (`swift test` in `client/`, run locally; Linux CI does not run it):
`PairingTests` gains the URL form with an IPv4 and a bracketed IPv6 host,
and the JSON form still parsing.

**By hand, recorded in the status section, not in CI.** Script the
instance with `SWITCHBOARD_SCRIPT` lines `remote-listen lan <port>` and
`pairing ABCD2345` and a short `SWITCHBOARD_DATA_DIR`, then open the URL
in a desktop browser: warning, sessions, a waiting session moving to the
top within 5 s, a reply arriving in the pane, Revoke and the page
saying so on its next poll. The iPhone run is For the owner A, C and D;
nothing in the commit waits on it, and the status section says plainly
that no phone run has been made.

## Docs, in the same commit

- `docs/remote-client.md`: §4.1 (the page's three files, answered on the
  connection thread, their own 60-a-minute window); §4.3 (a `GET
  /api/v1/device` row, and one line that `/`, `/app.js` and `/app.css` are
  served outside the allowlist); §5 (the pairing URL replacing the JSON
  invite, "before anything but `/info`, `/pair` and the page", the
  cookie, the subprotocol, the `Origin` rule, the corrected relay sentence
  for browsers, the certificate reissue over the same key).
- `docs/design.md`: a bullet "A web control surface (2026-10-22)" under
  "Remote client (2026-10-12)" (`:3107`) with what is built and the gaps:
  no phone run, polling is the proven path and the socket is a probe, no
  excerpt on every card, no Dispatch tickets or conversation beyond the
  last response, drafts, dictation, runs and pinned files, no trust or
  resume from the page, the cookie shared across ports of the host.
- `README.md`: Layout rows for `web/`, `src/adapters/remote_web.rs`, and
  the `tests/remote.rs` row; the dev-aid text at `:274-279` ("Copy
  invite" is now a URL; open it in a browser, or `switchboard-remote pair
  "$(pbpaste)"` still works). `client/README.md` for `pair`.
- `CHANGELOG.md` under Unreleased: the page, and that the invite is now a
  link.

## Order inside the one commit (the app runs after each)

1. `Identity` fields, the version bump, `reissue_certificate`,
   `certificate_is_stale`, `refresh_certificate` in `bind_remote`,
   `CertificateReissued` and `CheckCertificate`. Native pairing unchanged.
2. `RemoteRequest`/`RemoteReply` fields, the one token reader,
   `Origin` rule, `Set-Cookie`, the subprotocol echo, `/device`.
3. `remote_web.rs`, `web/*`, the page arm and its window.
4. The pairing URL in the UI and the Swift `parse`.
5. Tests and docs.

## Commit message

Serve a browser control surface on the remote port: a page embedded in the binary, paired from a link, with a reissued certificate over the same key

## Contract additions

- `Identity` (`src/ports/remote.rs`): `certificate_names: Vec<String>` and `certificate_not_after: Option<SystemTime>`, both `#[serde(default)]`; `IDENTITY_SCHEMA_VERSION` bumped to its value at implementation time plus one (1 today). `SCHEMA_VERSION` (16) and `VIEWS_SCHEMA_VERSION` (7) unchanged.
- `AppAction::CertificateReissued(Identity)` and `Effect::CheckCertificate` (`src/core/action.rs`).
- `RemoteRequest`: `cookie`, `host`, `origin`, `subprotocol`; `RemoteReply`: `set_cookie` (`src/adapters/remote.rs`, outside the shared-contract files).
- `GET /api/v1/device` → `{id, name, scope}`, and `"device"` in `info.capabilities`.
- `POST /api/v1/pair`: optional body field `cookie: bool`; with it, a `Set-Cookie: sb-<server_id>=<token>` (`HttpOnly; Secure; SameSite=Strict; Path=/api/v1/`).
- Refusal `403 bad_origin` for a cookie-carried token with a missing or foreign `Origin` (non-GET and upgrade) or a foreign one (GET).
- Token transports: `Authorization: Bearer`, `Sec-WebSocket-Protocol: token.<t>` (101 echoes `switchboard.v1`), and the cookie; all three accepted.
- Unauthenticated static paths `GET /`, `/app.js`, `/app.css`, with a 60-a-minute rate window, `Cache-Control: no-store`, `nosniff` and a CSP; `no-store` and `nosniff` added to every JSON reply.
- Pairing invite is the URL `https://<host:port>/?code=<code>&key=<spki_sha256>&id=<server_id>`; `PairingInvite.parse` reads it and still reads the JSON.

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
