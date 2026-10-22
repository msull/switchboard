You are scoring one implementation plan for a GitHub issue, blind: you do not know who or what wrote it, and you score this plan on its own.
The repository at the issue's base commit is checked out at /Users/sully/code_repos/plan-trial/sb-230-base (read-only; change nothing there). Every file path in the plan is relative to that tree.

Score by the rubric, column by column. Length is not thoroughness. For grounding, take the first five distinct file or line references the plan makes, in order of appearance, open each in the tree, and say what you found. For departures and decisions, use the reference as the yardstick; where you disagree with the reference, say so in the evidence rather than silently scoring your own view.

Your final message must be exactly this shape and nothing else:

```
| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| O | <0-3> | <0-3> | <0-3> | <0-3> | <yes|note|no> |
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

# The plan (O)

# Plan: a web control surface on the remote port (#230)

The host serves a small page from the remote port's own listener. A phone
opens it from the pairing window's QR, pairs, lists the sessions with the
waiting ones first, reads a session's last response or screen, and sends a
reply. The page speaks the protocol the headset speaks. One commit; the app
runs at every step.

Line numbers are from this worktree at 7fe8949 (Release 0.4.1). "Facts"
means `facts.md` for this ticket; a decision that rests on its Settled
section cites the line, and one that rests on Unresolved or For the owner
says so where it is made and names the fallback.

## Where this departs from the issue

1. **The pairing values go in the URL fragment, not the query.** The issue
   writes `/?code=…&key=…&id=…`. `read_request` (`src/adapters/remote.rs:444`)
   splits the query off and hands it to the frame, and the browser puts the
   whole request line in its history and, by the issue's own account, sends
   the code to the host before `replaceState` runs. A fragment is not part of
   the request target, so a client does not send it (RFC 3986 §3.5 defines it
   as resolved client-side; RFC 9110 §7.1 builds the request target from the
   path and query), and a `Referer` carries no fragment (RFC 9110 §10.1.3).
   Those three citations are the whole basis for "the host never sees it"; I
   have not fetched them, and Facts has no line for them. The previewer half
   stays **Unresolved** (Facts, the iOS-camera line): a previewer or the
   camera might read the URL itself, fragment included, before the owner taps,
   and nothing in this plan stops that. The page and both native parsers
   accept either form, so the issue's URL still works (an old-form URL does
   send the code in the request line). The shape is
   `https://<ip>:<port>/#code=<code>&key=<spki_sha256>&id=<server_id>`.
2. **The card's one-line excerpt is not on the wire, and the spec line that
   says it is does not match the tree.** `docs/remote-client.md` §4.3 (the
   `session.last_response` paragraph) says the excerpt "is `last_message` from
   the `Stop` hook on the record, a field of the session's metadata, and moves
   with `changed {session}`". `last_message` is carried by the hook event
   (`src/ports/events.rs:24`, `src/adapters/hooks.rs:71`) and nothing in
   `src/core` keeps it; `SessionView` (`control/src/lib.rs:637`) has no such
   field. The card's text is `agent_excerpt` (`src/ui/cards.rs:558`) over a
   parsed conversation, and a shell's is a `UiState` caption. This is also
   For the owner in Facts ("Excerpt on the wire"), unanswered. **Plan:** no
   new field. The page shows the state word and `waiting_reason` for every
   row, and for a Claude Code session the first line of
   `GET /sessions/{id}/last-response` (`text`, `src/content.rs:827`), fetched
   lazily for rows on screen, two at a time, because a read goes through the
   one content worker and enters the watch set (cap 128, `WATCH_CAP`
   `src/content.rs:42`). A shell, command or service row has state only.
   Fallback, built only if the owner asks: an additive `SessionView` excerpt.
   The commit also corrects the §4.3 sentence.
3. **A page needs to learn its scope later, and there is no way to.** Facts
   (Unresolved, last line) allows remembering the `/pair` reply's scope. That
   goes stale in the direction the owner most wants (raising a phone to
   `control` in the dialog, which §5 says takes effect "on the device's next
   request"). **Plan:** one additive authenticated read, `GET /api/v1/me` →
   `{device_id, name, scope}`. It is also the page's boot probe.
4. **The pairing window hands a phone an address that does not answer when
   the port is loopback-only.** Facts (Settled, `cx.remote_port` line) says the
   plan should close this. The QR's address comes from `local_address()` even
   when `identity.listen.lan` is false. The plan puts `127.0.0.1` in the URL
   in that case and the dialog says why a phone cannot use it (UI).
5. **The QR becomes URL-only; the JSON form is dropped from the QR.** This is
   the "One URL or JSON too" question, For the owner in Facts and unanswered;
   the plan takes the "drop the JSON" branch for what the QR encodes and
   keeps parsers accepting both. The cost: a headset or `switchboard-remote`
   built before this change cannot read the new QR or the copied invite,
   because `PairingInvite.parse` (`Info.swift:114`) is JSON-only, and those
   apps do not self-update with the Mac app. They are not stranded: the
   headset pairs from the Bonjour record and a typed code, and the CLI has
   its three-argument form `host:port code spki` (`main.swift:107`).
   Fallback, built only if the owner asks: "Copy invite" keeps copying JSON.

## The record

No new record and no `SCHEMA_VERSION` change (`src/core/model.rs:12`, 16
today; `VIEWS_SCHEMA_VERSION` `:612`, 7 today). A browser is a `PairedDevice`
(`src/ports/remote.rs:96`) like any other, named by the page in
`device_name`, which `remote_pair` (`src/app.rs:3606`) already reads at
`:3650`. The page derives the name from `navigator.userAgent` (`Safari on
iPhone`, `Chrome on Android`, `Safari on Mac` for an iPad, which reports a
Mac: Unresolved in Facts, the For-the-owner question on naming stands). It
shows the name in an editable field before the code is sent, so the owner can
type "Sully's iPad". No `User-Agent` is kept by the host and `PairedDevice`
gains nothing.

**The certificate does change, and `identity.json` with it.**

- *Decision 1 (click-through or download), which is For the owner.* The plan
  builds **click-through as the trust step**, said in the dialog (UI), and
  makes the certificate acceptable to the browsers that check. No download
  route: it needs an iOS profile-install flow that Facts lists as Unresolved.
  Fallback, built only if the owner asks: `GET /cert.der`.
- *If click-through hard-fails even with the SAN.* Facts leaves two lines
  Unresolved (whether Safari offers "Visit this website"; whether `fetch` and
  `wss://` keep working after a click-through), and the plan builds for the
  answer being yes. If a browser refuses the certificate with no way through,
  or trusts it for the page but not for `wss://`, **the page is unusable on
  that browser, and the plan accepts that**: the native clients are
  unaffected, and the design status says which browsers were tried and
  failed. No plain `http:` listener: it would carry the pairing code and the
  bearer token in the clear on the local network, against §5. Facts line 9
  (a bare-IP URL needs an IP-type SAN to match) is **an inference**: Apple's
  page speaks only of DNS names. The IP SAN is harmless if it is wrong; the
  unit test below checks it against `rustls`'s verifier, which is not a
  browser, and only a device settles it.
- *What is built.* Facts (Settled, Apple row): the current certificate has no
  SAN, no `serverAuth` and a year-4096 expiry, against Apple's 825 days.
  `make_identity` (`src/adapters/remote.rs:95`) and a new
  `reissue_certificate(identity, key_pem, addresses, now)` share one parameter
  builder: SAN of each address given plus `127.0.0.1`, `::1` and `localhost`;
  `serverAuth` EKU; not_before `now`, not_after `now` + 800 days (`time`,
  already in `Cargo.lock` through `rcgen`, becomes a direct dependency; `now`
  is a parameter so the function is deterministic). The reissue reuses the
  same key from the Keychain, so `spki_sha256` is unchanged and every pairing
  stays valid: §5 pins the key and the `Pin` verifier
  (`src/adapters/remote.rs:183–197`) looks at nothing else.
- *`Identity` (`src/ports/remote.rs:25`) gains* `#[serde(default)] pub sans:
  Vec<String>` (the addresses the certificate names) and `#[serde(default)]
  pub certificate_issued: Option<SystemTime>`. `IDENTITY_SCHEMA_VERSION`
  (`src/ports/remote.rs:17`) is 1 today, so the bump is to 2: written as the
  value in that file at implementation time, plus one, because a ticket
  landing in parallel may take 2 first. Nothing reads `identity.json`'s
  `schema_version`; `RemoteFiles::load` (`src/adapters/remote.rs:55`) is plain
  serde, so a version-1 file loads with both fields empty and no migration
  step is needed.
- *When it runs.* Every path that mints a certificate gets the SAN, so the
  QR never shows a certificate without one:
  - `remote::make_identity` gains a parameter `sans: &[IpAddr]`. Its four call
    sites: `src/app.rs:2866` (the one wrapper, `App::make_identity`, which
    both the key-lost arm `:2810` and the first-run arm `:2836` use, and which
    passes `remote::local_address()`), `src/adapters/remote.rs:982` and
    `:1012` (unit tests, passing `&[]`), `tests/remote.rs:1423` (`&[]`).
  - The existing-identity arm of `load_identity` (`src/app.rs:2781`), after
    the key check succeeds and not when `read_only()`, calls
    `reissue_certificate` when `sans` is empty, when `local_address()` is set
    and not in `sans`, or when `certificate_issued` is over 700 days old, and
    saves as `make_identity` does. A reissue never touches `devices.json`. A
    LAN address that changes while the app runs is not caught until the next
    launch; the dialog says so (UI) via a new `Identity::covers(ip) -> bool`.

## The signals

No new core signal, `AppAction` or event. The page consumes the existing
`/api/v1/events` socket (`hello`, `changed`, `removed`, `resync_required`,
§4.4) and sends `watch`/`unwatch` for the open session's `screen` or
`conversation` key. What is new is on the wire into the host:

**Static paths, answered on the connection thread** (`connection`,
`src/adapters/remote.rs:770`, after the rate check and before
`shared.requests.try_send`). They are constants and decide nothing, which
CLAUDE.md puts in the adapter. The frame's `/api/v1/` refusal at
`src/app.rs:3023` is untouched, so any other path, `/other` included, keeps
its JSON 404.

| Path | Content-Type | Source |
|---|---|---|
| `GET /` | `text/html; charset=utf-8` | `web/index.html` |
| `GET /app.js` | `text/javascript; charset=utf-8` | `web/app.js` |
| `GET /app.css` | `text/css; charset=utf-8` | `web/app.css` |

The query and fragment do not matter. Any other method on these paths is
`405` JSON. A new `src/adapters/web.rs` holds the table (`include_str!` of
`web/`, the first text assets in the tree; `include_bytes!` is used only at
`src/main.rs:167` and `src/ui/theme.rs:203`) and the header set; `respond`
(`:530`) gains a sibling `respond_page(w, content_type, body)` that writes:

- `Content-Security-Policy: default-src 'none'; script-src 'self'; style-src
  'self'; connect-src 'self' wss:; img-src data:; base-uri 'none'; form-action
  'none'; frame-ancestors 'none'`. Facts (Settled, CSP line) says `'self'` does
  not cover `wss:` in all browsers, so the plan names the scheme.
- `Cache-Control: no-store` (no caching across versions),
  `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`.

**Pool and rate.** Facts (Settled, `Limits::limited` line): a page load
counts against no rate window and only the 4-slot unauthenticated pool. The
plan adds a window, `PAGE_PER_MINUTE = 120` per listener (about 40 loads
with `no-store`), in `Limits` (`:621`) and `limited` (`:631`) for the three
paths, answering the existing 429. The unauthenticated pool is unchanged at
4 (`MAX_UNAUTHENTICATED`, `:567`): a connection that has only fetched files
is "unauthenticated" until a paired request, then moves to the paired pool
like any other. Whether a phone's parallel page load fits in 4 within the
30 s `IDLE_TIMEOUT` is Unresolved in Facts and needs a phone. **If it starves**
the implementer does not raise the pool; they set `Connection: close` on
the three file replies, which frees the slot as soon as the file is sent.

**Without a code and without a token**, the page loads and shows: the host
cannot be known, "Open the pairing window on the Mac (Settings, Remote
devices, Allow pairing for 5 minutes) and scan its code with this device's
camera." No fields to type a code into.

**Decision 2: the token reaches the WebSocket as a subprotocol value.** For
the owner in Facts, so the choice and its fallback:

- *Chosen.* `new WebSocket(url, ["switchboard.v1", "bearer." + token])`. In
  `read_request`, for an upgrade request with no `Authorization` header, a
  `Sec-WebSocket-Protocol` list containing `switchboard.v1` and an entry
  `bearer.<token>` (token characters `[A-Za-z0-9_-]`, at most 64) sets
  `bearer`; `authenticate` is unchanged. The 101 echoes the constant
  `switchboard.v1`, never the token. Facts (Settled, RFC 6455 block) requires
  exactly this: the server must echo one offered value or the browser fails
  the connection; base64url tokens are valid token characters. Whether to
  echo is not constant: a native client that sent `Authorization` offered no
  subprotocol, and a 101 naming one it did not offer is a handshake error. So
  `RemoteRequest` gains `via_subprotocol: bool`, set in `read_request`, read
  by `connection` before the request moves into `RemoteIncoming`, and passed
  to `respond_upgrade`. The frame never reads it.
- *Why not the cookie.* A cookie is ambient: the browser attaches it to any
  request to that origin, so authentication stops being a header a
  cross-site page cannot send, and Facts (Settled, cookie block) says
  `SameSite` ignores the port, so another service on the same IP is
  same-site. That needs `Set-Cookie` on `RemoteReply`, a `Cookie` field, and
  an `Origin` check on every `POST` and the upgrade. The subprotocol keeps
  today's property that nothing authenticates unless the page's own script
  puts the token there. Whether a `Secure` cookie survives a click-through
  certificate is Unresolved in Facts; the chosen route does not depend on it.
  Fallback, built only if the owner asks: the `HttpOnly; Secure;
  SameSite=Strict` cookie with those `Origin` checks.
- *Where the token lives.* `localStorage` under the page's origin, readable by
  script on that origin. The mitigation is the CSP above and a page that never
  assigns `innerHTML`.

**The page's pairing steps** (`web/app.js`), in this order and nothing
before step 1:

1. Read `location.hash` (and `location.search`), then `history.replaceState(
   null, "", location.pathname)`. Whether that keeps the code out of Safari's
   history and recents is Unresolved in Facts and needs a phone.
2. `GET /api/v1/info`; refuse and say so unless `body.spki_sha256` equals
   `key` (case-insensitive) and `body.server_id` equals `id`. The page cannot
   see the certificate, only what `/info` claims (the issue's own caveat).
3. `POST /api/v1/pair {code, spki_sha256: key, device_name}`. A `403
   wrong_key`/`wrong_code`/`pairing_closed` is shown as that.
4. Store `{token, scope, server_id, key}` in `localStorage`, then `GET /me`
   for the live scope and start the session list.

Later visits: `/info` first; a key differing from the stored one → "identity
changed, pair again" and nothing is sent, token included (§5); otherwise
`/me`; a 401 says "revoked or removed", because `authenticate`
(`src/app.rs:3127`) answers one 401 for a revoked and an unknown token and a
rotated key deletes the devices (§5), so the issue's plain "revoked" cannot
be told from removal; a network error says "can't reach the host" with a
retry. The events socket reconnects with a backoff of 1 s doubling to 30 s,
reruns `/me` (this is how a revoke, which closes the socket from the host
side in `dispatch_inner` `src/app.rs:486`, shows), and refetches on a new
`epoch` or `resync_required`. The reply box exists only when `/me` says
`control`; it posts `{op, text}` to `/sessions/{id}/send` with `op` from
`crypto.getRandomValues` (the idempotency key, `src/app.rs:3159`), and a
`403 scope` hides the box again.

## The actions and effects

None in `AppAction`/`Effect`. One frame-side route: `serve_remote`
(`src/app.rs:3017`) matches `("GET", ["me"])` after `authenticate` and
answers `envelope(200, {device_id, name, scope}, true)`.

## The UI

Only `src/ui/remote.rs`, and the page itself.

- `pairing` (`:141`): `payload` becomes `invite_url(...)`, a pure function
  in `src/core/remote.rs` (`ip`, `port`, server id, key hash, code), because
  the dialog and the fixtures both need the same string; it formats
  `SocketAddr::new(ip, port)` (so IPv6 is bracketed) as
  `https://{addr}/#code=…&key=…&id=…`. The QR (`qr_texture` `:280`) encodes
  it. The text beside the code shows the address and short key as now. "Copy
  invite" copies the URL and its hover says "the QR code's address, to paste
  into a client". The scope line says "Scan with the camera, or paste the
  address into a client."
- When `!identity.listen.lan` the address is `127.0.0.1` and a line under
  the code says "Listening on this Mac only; a phone cannot reach it. Turn
  on Allow devices on the local network, above."
- When `identity.listen.lan` and `remote::local_address()` is set but
  `!identity.covers(ip)` a line says "This Mac's address changed since
  launch. Restart Switchboard so the certificate names it."
- The trust step, a sentence near the pairing window: "A browser will warn
  about this Mac's certificate the first time. Open the address only from the
  code on this screen and accept the warning for it; the page then checks the
  key against the one shown here before it sends the code. A page from a
  different host is not protected by that check." The hover on the LAN
  checkbox (`:113`) and §5's "Require pairing before anything but `/info` and
  `/pair`" name the three page files.

The page, `web/index.html`, `web/app.js`, `web/app.css`: no inline script or
style attribute. Text from the host is set with `textContent` only. It is a
single column that works at phone width (16 px side gutter, no horizontal
scroll) with `prefers-color-scheme` colors defined once on `:root`. Views:
pairing/status messages, the session list (waiting first, `card`,
`waiting_reason`, `kind`), the session view, the header (host name from
`/info`, device scope from `/me`).

## The spike

`spikes/23-web-surface/` (the next free number; 20, 21, 22 exist) holds the
one check no later test repeats: **a page against the pool of 4.** Six
parallel keep-alive connections fetching the three files from a listener
built like `connection`: how many are refused or time out, with and without
`Connection: close`. Its README holds the command and the numbers. A phone's
actual behavior stays Unresolved. The certificate-against-a-verifier check
and the 101 echo are the unit test and `a_subprotocol_token_is_checked_like_a_bearer`
below; the README points at them rather than repeating them.

## The tests

`tests/remote.rs` (a new helper beside `http()` `:245` that returns response
headers, since `http()` discards them):

- `the_page_is_three_files_with_their_headers`: `GET /`, `/app.js`,
  `/app.css` with no token answer 200, the content types in the table, and
  exactly the CSP string exported from `adapters::web`, `Cache-Control:
  no-store`, `nosniff`, `no-referrer`; `GET /?code=X` answers the same page;
  `POST /` is 405; `/other`, `/index.html` and `/app.js.map` stay the JSON
  404. A page request makes no `RemoteIncoming`.
- `the_page_references_only_what_the_host_serves`: every `src`/`href` in
  `index.html` is one of the three paths; no `http:`/`https:` URL in any of
  the three files; no `<script>` without `src`, no `style=` attribute, no
  `innerHTML`, `outerHTML`, `document.write` or `eval(` in `app.js`; every
  `/api/v1/<segment>` literal in `app.js` has a first segment in `{info,
  pair, me, projects, sessions, waiting, events}`.
- `the_page_is_rate_limited_per_listener`: `PAGE_PER_MINUTE` + 1 requests →
  429, shaped like `pair_is_rate_limited_per_listener` (`:1406`).
- `a_browser_pairs_and_follows_the_host_end_to_end`: the issue's acceptance
  from the host's side with the host's own client, as the CLI does. Build the
  URL with `invite_url`, split its fragment, `GET /info` and compare, `POST
  /pair` with `device_name: "Safari on iPhone"`, `GET /me` → the scope from
  the window; open the socket with the subprotocol list and no
  `Authorization`, assert the 101 echoes `switchboard.v1` and not the token,
  receive `hello`; change the session to waiting through the control socket
  and see its `changed`; raise the device to `control` with
  `SetDeviceScope` and see `/me` say so without re-pairing; `POST /send`
  and see it typed once; revoke, assert `/me` is 401 and the socket closes
  (the pattern of `revoke_closes_the_devices_socket` `:2892`).
- `a_subprotocol_token_is_checked_like_a_bearer`: a wrong token → 401 and no
  101; an `Authorization` header wins over the subprotocol; a subprotocol on
  a non-upgrade request authenticates nothing; a token over 64 characters or
  with a character outside `[A-Za-z0-9_-]` is ignored; a client offering no
  subprotocol gets a 101 with no `Sec-WebSocket-Protocol` line.
- `me_is_read_scope_and_unauthenticated_is_refused`.
- The fixtures test (`:1829`) gains `me.json` and `pairing_url.txt` in
  `write_fixtures` (`:1587`), the second written from `invite_url` with fixed
  inputs.

`src/adapters/remote.rs` unit tests: a fresh certificate and a reissued one
are accepted by `rustls`'s `WebPkiServerVerifier` (the certificate as its own
root) for `ServerName::IpAddress` of a SAN address and for `localhost`, are
refused for an address not in the SAN and 801 days after issue, and keep the
same `spki_sha256`. If webpki will not take a leaf as its own root, assert on
the `CertificateParams` the shared builder returns instead. A reissued
identity round-trips through `RemoteFiles`; an older file with neither new
field loads.

`src/core/tests.rs`: `invite_url` for an IPv4 and an IPv6 address; the three
`Identity { .. }` literals (`src/core/tests.rs:11769`, `tests/ui.rs:7436`,
`src/adapters/remote.rs:117`) gain the new fields. `Identity::covers`. A
fresh identity from the first-run and key-lost arms carries the local
address in `sans` (through the fake `local_address` seam the app already
uses, or by asserting on `make_identity`'s `sans` argument).

`tests/ui.rs` (the test around `:7474`): "Copy invite" now yields a URL;
parse its fragment and assert `code`, `key` equal to the identity's and
`id`; a loopback identity shows the "Listening on this Mac only" line and
puts `127.0.0.1` in the URL; an identity whose `sans` lack the local address
shows the restart line.

Swift (`client/`): `PairingInvite.parse` (`Info.swift:114`) accepts an
`https://host:port/` URL with `code`, `key`, `id` in the fragment or the
query — authority → `addresses: [authority]`, `key` → `spkiSha256`, `id` →
`serverID` — and the JSON as before, so no call site changes
(`main.swift:107`, `BrowserView.swift:106/883`). Tests in
`FixturesTests.swift` read `pairing_url.txt` and a JSON invite, and cover
the query form. These tests are in neither the hook nor CI; the implementer
runs `swift test` in `client/` if the toolchain is there and says in the
commit body if it was not.

The page's JavaScript has no test tooling in the tree, so only the owner's
phone run checks its behavior. What that run must settle is listed once, as
"Not run", in the design-status bullet below.

## Docs and files the commit touches

`docs/remote-client.md`: §4.1 bounds (the page window), §4.3 table (`GET /`,
`/app.js`, `/app.css`, `GET /api/v1/me`) and the corrected `last_response`
sentence, §5 (QR is a URL; the pairing sentence; the token sentence names the
subprotocol carrier for the events upgrade; the browser trust paragraph; the
"Require pairing before…" line), and a subsection after 4.7 for the page.
`docs/design.md`: a "Web page" bullet at the end of "Remote client
(2026-10-12)" (before `## Versioning and updates`, line 3585) with what is
built, "Not run" for every device-only check (each browser's click-through
and its `fetch`/`wss://` after it, the iOS camera opening the URL,
`replaceState` and Safari's history, a phone's parallel connections, the
socket after sleep, the acceptance run), and the gaps listed under Pool and
rate, The record and departure 2.
`README.md`: Layout rows for `web/`, `src/adapters/web.rs`,
`spikes/23-web-surface` (if the README lists spikes), the changed
descriptions of `src/ui/remote.rs`, `src/adapters/remote.rs` and
`tests/remote.rs`; the dev-aid paragraph at `:275` (the invite is now a
URL: `switchboard-remote pair "$(pbpaste)"` still works, and a desktop
browser can open it). `CHANGELOG.md`, under Unreleased: "A browser on the
local network can pair from the remote devices dialog's QR code, list
sessions with the waiting ones first, read a reply and send one."

## Commit

`Serve a web control surface from the remote port, paired from a URL in the QR code`

### Contract additions (for the commit message)

- `GET /`, `GET /app.js`, `GET /app.css`: static, unauthenticated, answered
  by the adapter.
- `GET /api/v1/me` → `{device_id, name, scope}` (read scope).
- WebSocket upgrade accepts `Sec-WebSocket-Protocol: switchboard.v1,
  bearer.<token>` when there is no `Authorization` header, and echoes
  `switchboard.v1`.
- Pairing invite is a URL, `https://<addr>/#code=&key=&id=` (the query form
  also accepted); `PairingInvite.parse` takes it or the JSON.
- `Identity.sans: Vec<String>` and `Identity.certificate_issued:
  Option<SystemTime>` (serde default), with `IDENTITY_SCHEMA_VERSION` bumped;
  the certificate is reissued over the same key with a SAN, `serverAuth` and
  800 days.
- `RemoteRequest.via_subprotocol: bool` (adapter-local); `remote::make_identity`
  gains `sans: &[IpAddr]`.
- `core::remote::invite_url`, `Identity::covers`, `adapters::web`,
  `adapters::remote::reissue_certificate`, `PAGE_PER_MINUTE`,
  `spikes/23-web-surface`.
