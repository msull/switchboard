You are scoring one implementation plan for a GitHub issue, blind: you do not know who or what wrote it, and you score this plan on its own.
The repository at the issue's base commit is checked out at /Users/sully/code_repos/plan-trial/sb-230-base (read-only; change nothing there). Every file path in the plan is relative to that tree.

Score by the rubric, column by column. Length is not thoroughness. For grounding, take the first five distinct file or line references the plan makes, in order of appearance, open each in the tree, and say what you found. For departures and decisions, use the reference as the yardstick; where you disagree with the reference, say so in the evidence rather than silently scoring your own view.

Your final message must be exactly this shape and nothing else:

```
| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| Q | <0-3> | <0-3> | <0-3> | <0-3> | <yes|note|no> |
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

# The plan (Q)

# Plan for #230: a web control surface on the remote port

One commit. The app runs at every step of the order at the end. "Facts"
below is `facts.md` for this ticket, cited as `facts:<line>`. Tree
references were checked in the worktree at "Release 0.4.1".

## Where this departs from the issue

1. **The excerpt is not on the wire.** The issue gives each session "the
   one-line excerpt the card shows". `SessionView`
   (`control/src/lib.rs:637`) has no excerpt field and `last_message`,
   which `docs/remote-client.md` §4.3 (`session.last_response` bullet)
   calls "a field of the session's metadata", exists nowhere in the
   wire crate (facts:8). The card's line is drawn on the Mac by
   `caption_of` (`src/app.rs:4105`). Owner question 6 (facts:91) is
   unanswered, so the plan builds its "no" branch: a row shows the
   session's name, kind, card word and waiting reason. The other
   branch is an additive `SessionView` field and is not built here.
2. **The page cannot learn its scope from anything.** The issue puts
   "the device's scope" in the header and hides the reply box without
   `control`. No endpoint says it (facts:9), though `docs/remote-client.md`
   §5 ("the host answers with the device's scope") says every reply
   does; the envelope carries only `epoch`, `revision` and `body`
   (`src/app.rs:3730`). Owner question 7 (facts:94) is unanswered; the
   plan builds its "yes": `GET /api/v1/device`. The "no" branch (the
   page stores the scope from the `/pair` reply and learns of a change
   from a 403) is worse for a page, whose cookie it cannot read, so
   it is not built.
3. **"Revoked" cannot come from the host.** A revoked token and an
   unknown one get the same 401 and `devices.json` keeps no tombstone
   (facts:10; `authenticate`, `src/app.rs:3127`; `RevokeDevice`,
   `src/core/remote.rs:86`). Owner question 5 (facts:88) is
   unanswered; the plan builds the "no" branch with a twist that
   needs no record: the page remembers, in `localStorage`, that it
   paired with this host. A 401 on a browser that
   remembers pairing says "not paired, or revoked on the Mac, or this
   browser dropped the pairing; pair again", and a 401 on one that does
   not says "not paired". The tombstone branch (a durable record of
   revoked token hashes) is not built.
4. **The page does not compare the key itself.** The issue has the
   page "compare the key in the URL with the one `/info` presents". A
   page cannot read the TLS certificate or its SPKI (facts:67,
   unresolved; the plan assumes it cannot), so the only comparison
   available is with a field the answering server writes. The host
   already refuses a `/pair` whose `spki_sha256` is not its own
   (`wrong_key`, `src/app.rs:3632-3639`), which is the same job done
   where it can be enforced: the page sends the link's key and maps
   `403 wrong_key` to "this is not the Mac whose code was shown". The
   wrong genuine Switchboard has then seen the code and counted a
   failure against its own window; the code is single-use and the
   window is five minutes. It stops no impostor. See decision 1.
5. **There is no all-sessions read.** "Sessions grouped by project"
   costs `GET /projects` and one `GET /projects/{id}/sessions` per
   project (facts:9); the page does them one at a time (see the
   signals). No new read model.
6. **"The host's own HTTP client as the CLI does".** The CLI
   (`client/Sources/switchboard-remote/main.swift:106`) is Swift. The
   Rust test side is the hand-rolled `http` in `tests/remote.rs:245`,
   which reads only the status and `Content-Length` (facts:19). The
   plan extends that helper; it adds no client to `src/`.
7. **Device names.** The issue wants "Safari on iPhone" from the user
   agent. The host reads no `User-Agent` and does not need to: `/pair`
   already takes `device_name` from the body (`src/app.rs:3650`,
   facts:14), so the page builds the name. An iPad's Safari sends a
   Mac's user agent (facts:59), so the page tells it apart with
   `navigator.maxTouchPoints > 1`, a heuristic the plan states in the
   code's comment; a wrong guess only mislabels a row the owner can
   see. Owner question 10 (facts:101) is open; its other branch (the
   page asks for a typed name) is a field on the pairing view and is
   not built.

## The record

Nothing on disk changes. A browser is a `PairedDevice`
(`src/ports/remote.rs:96`) like any other: name from the page, scope
from the pairing window, listed and revoked in the existing dialog. No
field is added to `PairedDevice` or `Identity`, so no
`IDENTITY_SCHEMA_VERSION` change (1 today, `src/ports/remote.rs:17`)
and none to `SCHEMA_VERSION` (16, `src/core/model.rs:12`) or
`VIEWS_SCHEMA_VERSION` (7, `:612`); `model.rs` is not touched.

What the page keeps, per device, none of it from a host's data:
the cookie (set by the host, `HttpOnly`, so script cannot read it) and
one `localStorage` flag `sb.paired`, written after a successful
pairing, which only picks the wording of a 401 (protocol step 4).
`localStorage` is wrapped in `try`/`catch`; without it the page works
and says "not paired". Everything else is refetched on every load, as
the Swift client does (`docs/remote-client.md` §7, "What the client keeps").

## The signals

The wire gains three static paths, one route, one request field, and
one authentication source. `REMOTE_PROTOCOL` stays 1: every addition is
additive.

**Decision 1: browser trust in the self-signed certificate. Option A: the
click-through on the local network is the trust step.**
This rests on two lines. The certificate has no SAN, no `serverAuth`
EKU and a year-4096 expiry (facts:16), and iOS trusts only a certificate
that names the server in the SAN, carries the EKU and lives 825 days or
less (facts:55), so option B (the host offers its certificate) is a new
certificate per LAN address, a profile install and Certificate Trust
Settings on each device (facts:54) for a thing the owner does once per
phone anyway. And whether Safari lets a WebSocket through after the
page's click-through is unresolved (facts:64, bug 145819); only a device
settles it. That is owner question 1 (facts:76), and it decides
nothing about the build: **the page falls back to polling if the
socket fails to open** (below), which is the fallback facts:78 names
for Safari. Option B (a reissued certificate, facts:55) stays unbuilt.
What option A buys: the connection is encrypted against anyone
watching the LAN, and the `key` in the URL makes a wrong genuine
Switchboard (a reused DHCP address, two Macs) refuse the pairing. What
it does not buy: a page served by an active
impostor is not a genuine page, and an impostor can relay `/pair` to
the real host inside the window and keep the cookie the real host sets.
Doing that needs a man in the middle of the Mac's address and the owner
clicking through the warning for it within five minutes. The dialog
says so (see the UI). `docs/remote-client.md` §5's "the client verifies
the pin on every connection" gains "a browser cannot; see §5.1".

**Decision 2: the token reaches the socket in a cookie. Cookie, with the
Origin rule below; the bearer header stays for the native clients.**
The owner's question 2 (facts:79) is open; the plan takes the cookie
branch because it keeps the token out of script and out of the storage
Safari prunes after seven days (facts:58), and the browser sends it on
the WebSocket handshake (facts:30), which a subprotocol would need
`respond_upgrade` (`src/adapters/remote.rs:519`) to echo (facts:34).
- `POST /api/v1/pair` takes a new string body field
  `credential: "cookie"`, read with the same string `field()` helper as
  the other body fields (`src/app.rs:3637`); any other value or none
  is today's reply. `"token"` is reserved for the fallback below, which
  then adds a value rather than changing a type. The reply for a
  cookie pairing carries no `token` in the body (script never sees it)
  and sets
  `sb-<server_id>=<token>; Path=/api/v1; Max-Age=34560000; HttpOnly;
  Secure; SameSite=Strict`. The 400 days is a choice, not a fact: it is
  the longest lifetime Chrome keeps, and no line of the facts file
  supports it. The cookie slides: `GET /device` on a cookie
  credential re-sends the same `Set-Cookie`, so a page in use does not
  expire. The name carries the `server_id` because
  cookies are not isolated by port (facts:24), so two instances on one
  address (the dev aid `SWITCHBOARD_REMOTE_PORT`, the tests) must not
  share one. `Path=/api/v1` keeps the cookie off the static requests.
  Without `credential` the reply is today's, byte for byte, and no
  `Set-Cookie` is sent.
- **Where the cookie goes, which is the cost of this choice.** The same
  line (facts:24) means the browser sends `sb-<server_id>` to any other
  https server on this IP whose request path is under `/api/v1` (a
  development server on the Mac, say), which then holds the token in
  clear; and because `server_id` is public in `/info`
  (`src/app.rs:3575`) such a server can set a cookie of the same name
  and replace the device's credential with its own. The spec's "sent
  only as a bearer header over the pinned connection" (§5, line 312) is
  amended to say this, and the dialog's trust sentence says it. The
  owner who finds it unacceptable takes the fallback in the last bullet
  here; it needs no new code in this commit.
- `authenticate` (`src/app.rs:3127`) reads the bearer if present and
  ignores any cookie then; with no bearer it reads the cookie named
  for this host's `server_id`. A request whose credential came from the
  cookie must pass the **Origin rule**: for a non-`GET` request or an
  upgrade, `Origin` must equal `https://` plus the request's `Host`
  header, else 403 `origin`. The rule is needed because `SameSite`
  ignores the port, so a page on another port of this IP is same-site
  (facts:25), and the browser sends `Origin` on every POST and every
  socket handshake (facts:31, 32). A `GET` carries none (facts:32),
  so the rule cannot cover it. A cross-origin page cannot read a
  `GET`'s reply (the port never sends a CORS header, and answers no
  `OPTIONS`), but a `GET` is not free of effects: `authenticate` runs
  `DeviceSeen` for every authenticated request (`src/app.rs:3150`),
  which sets `last_seen` and, past `LAST_SEEN_SAVE_INTERVAL_SECS`,
  rewrites `devices.json` (`src/core/remote.rs:72-84`). So:
  **`DeviceSeen` is skipped for a cookie-authenticated `GET` that
  carries no `Origin`** (which is every same-origin `GET` from the
  page too), and "seen" in the dialog moves only on a bearer request, a
  cookie `POST` or the socket handshake, all of which are checked. A
  page in poll mode therefore shows the device as seen at its last send
  or socket; the plan accepts that. Accepted, and written into §5: a
  foreign same-site page can still make a cookie `GET` that fills the
  12-slot paired pool (a cookie `GET` replies `authenticated: true`,
  `src/adapters/remote.rs:835-848`) and the frame's 64-deep inbox. It
  leaks nothing and changes no record, and the pool and inbox bounds
  are what contain it. The bearer path needs no rule: a cross-origin
  page cannot attach the header.
- A 401 for a cookie credential the host does not know also sends
  `sb-<server_id>=; Path=/api/v1; Max-Age=0`, so a revoked device's
  browser stops sending a dead cookie on every poll.
- The cookie's `Secure` and `HttpOnly` on a click-through origin are
  stored and sent per the standard (facts:27) but no vendor says
  Safari or Chrome do (facts:65). If a device does not keep it, the
  page's first call after pairing (`GET /device`) is a 401 and the page
  says "this browser would not keep the pairing; open the pairing window
  and try again", having spent the code. The fallback the implementer
  builds if the owner's device run shows this is a follow-up, not this
  commit: the `credential` field takes the value `"token"`, the reply carries the token,
  the page holds it in `localStorage` (and so loses it after seven
  days idle, facts:58), sends `Authorization: Bearer` on fetch, and
  polls because the socket cannot carry it. No host change beyond the
  field's second value. This is stated so the owner can pick it with
  the device result in hand.

**New route** `GET /api/v1/device`, authenticated, any scope: body
`{ "device_id", "name", "scope" }` of the caller, in the usual envelope
(`authenticated: true`). It is added to the route table in
`serve_remote` after `authenticate`, answered by `remote_device(&self,
&PairedDevice)`. The scope is read from the core on every call, so a
change in the dialog shows on the page's next request, as §5 says.

**Decision 4: static paths** — `GET /`, `GET /app.js`, `GET /app.css`
and nothing else. `/index.html`, `/favicon.ico` and every other path
outside `/api/v1/` stay `404 not_found` from `serve_remote`
(`src/app.rs:3023`), so `tests/remote.rs:464` holds. A query string on
the three is ignored (the pairing link is `/?code=...`). A method other
than `GET` is 405 with `Allow: GET`. Each answer is `200` with
- `Content-Type`: `text/html; charset=utf-8`,
  `text/javascript; charset=utf-8`, `text/css; charset=utf-8`;
- `Content-Security-Policy: default-src 'none'; script-src 'self';
  style-src 'self'; connect-src 'self'; img-src data:; base-uri
  'none'; form-action 'none'; frame-ancestors 'none'`. No inline
  script, no inline style, no external resource. `default-src` does not
  cover the last three (facts:41) and `frame-ancestors` works only as
  a header (facts:42), so they are listed. `img-src data:` is for the
  page's `<link rel="icon" href="data:,">`, which saves a
  `/favicon.ico` request. `connect-src 'self'` covers `wss:` to the
  page's own host and port in the spec (facts:39); older Safari builds
  are unresolved (facts:68). If one blocks it the socket fails and the
  poll fallback runs, the same path as a Safari certificate failure;
- `Referrer-Policy: no-referrer`. The page's own subresource
  requests are made while the URL still carries the code, and a same-origin
  `Referer` is the full URL by default (facts:48); this header on the
  HTML is what keeps `/app.js` and `/app.css` from sending it;
- `Cache-Control: no-store`, so a page never outlives its version, and
  `X-Content-Type-Options: nosniff`;
- `Connection: close`.

*Where they are answered.* On the connection thread, before the frame
hand-off in `connection` (`src/adapters/remote.rs:770`), as the 429 and
503 already are. The files are compile-time bytes, so no frame state is
read, the frame's 64-deep inbox and 8-per-pass drain are not spent on
them (`docs/remote-client.md` §4.1: "the frame does core state only"),
and the code in a `/?code=` request line never reaches the frame's
memory or a `log::` call. The adapter's only `log::` is the
connection-error line (`src/adapters/remote.rs:714`, which prints the
`io::Error`, not a request), and `refuse` logs method, path and code
only (`src/app.rs:3754`); the page paths reach neither with a query.
*Pool and rate.* A page load is an unauthenticated connection like
`/info`'s, so it sits in the 4-slot pool (`MAX_UNAUTHENTICATED`,
`src/adapters/remote.rs:567`); `Connection: close` returns the slot
as soon as the file is written, so a load holds a slot for a few
milliseconds rather than the 30-second idle timeout, and the
HTTP/1.1-without-ALPN fan-out (facts:17, 71) cannot wedge the pool.
How many connections a given browser opens is unmeasured (facts:71);
the spike records it. A new `Limits.page` window, 60 a minute per
listener like `/info` (`src/adapters/remote.rs:623`), counts the
three paths together and answers 429 beyond it. The paired pool is 12
shared with the headset and the CLI, and a tab holds one slot for its
socket; the page therefore fetches **one request at a time** (a small
queue in `app.js`), so a tab holds at most a socket and one fetch
connection.
*With no code and no token* the page shows a static "Pair from the Mac"
panel and no host name; the same bytes go to everyone and the page
makes no request to find out who answers. This is the "a page" branch
of owner question 8 (facts:97). The "nothing" branch is `asset("/")`
returning `None`, one line. *Both binds serve it.* The files carry no
host data, so there is nothing to withhold on a loopback bind, and the
LAN rule of §5 becomes: before pairing a non-loopback bind serves
`/info`, `/pair` and these three files, which hold no host data. Owner
question 9 (facts:100) is open; the separate web switch is not built.

**The page's protocol** is the one the headset speaks, `docs/remote-client.md`
§4.4, re-implemented in `web/app.js`:
1. Open with `?code&key&id`: before any `await`, read the three,
   then `history.replaceState(null, "", location.pathname)`
   (allowed for the path and query of an https URL and it replaces the
   entry rather than adding one, facts:45, 46). The browser's global
   history, a camera app's prefetch and a link scanner may see the URL
   first and are unresolved (facts:69, 70); the code is single-use
   (facts:11), so what they keep is dead once the device pairs. The
   query form is the issue's; owner question 4 (facts:85) is open, and
   the fragment branch is `location.hash` in the page and a `#` for
   `?` in the one function that builds the link: the plan keeps the
   query and adds `Referrer-Policy: no-referrer` as facts:87 asks.
2. `POST /api/v1/pair` with `{code, spki_sha256, device_name,
   credential: "cookie"}`, the key being the link's. `403 wrong_key`
   is "this is not the Mac whose code was shown"; `403 pairing_closed`
   is "the pairing window on the Mac is closed; open it again". On
   success write `sb.paired`.
3. Opened without a link: go to step 4. A different Switchboard at the
   same address ignores a cookie named for another `server_id`, so
   step 4's 401 reports a changed host; the page makes no identity
   check of its own.
4. `GET /api/v1/device`, then `GET /api/v1/info` for the host's name.
   A 401 is worded "not paired, or revoked on the Mac, or this browser
   dropped the pairing; pair again" when `sb.paired` is set, and "not
   paired" when it is not. The page cannot tell these apart, and a
   device row may still exist when the browser lost the cookie; the
   design.md status paragraph notes the stale row. 503 `full` is "the
   Mac has too many connections open".
5. Open `wss` at `/api/v1/events` by a relative URL (resolved against
   the page, facts:36). Wait for `hello`, then snapshot (`projects`, then
   each `projects/{id}/sessions`), then apply the buffered events, per
   "Subscribe first, then snapshot". One held revision per key
   (`projects`, `sessions:<project>`, `session:<id>`, `waiting`,
   `conversation:<id>`, `screen:<id>`); a reply is applied only at or
   past the held revision; a new `epoch` drops everything;
   `changed` past the held revision refetches that key; `removed` drops
   it; `resync_required` or a close reconnects with 1 s doubling to 30 s,
   and a close followed by a 401 is the revoked state, which is how a
   revocation in the dialog (it drops the subscriber's queue,
   `src/app.rs:486`) reaches the page at once.
6. The open session sends `{"type":"watch","resource":{"kind":
   "conversation"|"screen","id":<record id>}}` (the frame format at
   `src/app.rs:1704-1735`), fetches after `watching`, and sends
   `unwatch` when closed. A visible content view also refetches every
   thirty seconds, as §4.4 says.
7. **Poll fallback** (the cost of owner question 1's other answer): if
   the socket gives no `hello` within five seconds on two attempts in a
   row (a socket the CSP blocks never sends one), the page says "Live
   updates are off; refreshing every 3 seconds" and refetches `waiting`,
   the project lists and the open view every three seconds, applying by
   revision as before. It retries the socket on a reload only.
8. Send: `POST /api/v1/sessions/{id}/send` with `{op, text}`, `op` a
   fresh UUID per attempt held until the reply; after a timeout the page
   reads `GET /api/v1/ops/{op}` and never resends blind (§4.5). Shown
   only when `device.scope` is `control`; a 403 `scope` hides it and
   re-reads `/device`. The 16 KiB cap is `info.limits.send_bytes`.

## Actions and effects

No `AppAction`, no `Effect` and no port trait changes. The core is not
involved except for one pure function: pairing is an adapter and frame
matter, and `AppAction::Paired` and `RevokeDevice` already do the whole
record. Everything below is adapter and frame.

- `src/adapters/remote.rs`
  - `RemoteRequest` gains `cookie`, `origin`, `host`: `Option<String>`,
    read in `read_request` (`:444`) from the lowercased headers beside
    `authorization`. `read_request` is the only place that builds one.
  - `RemoteReply` gains `set_cookie: Option<String>`; the ten literals
    in `src/app.rs` and the one in the adapter (`:829`) get
    `set_cookie: None`.
  - `respond` (`:530`) becomes a call to a new
    `respond_with(w, status, content_type, headers, body)`; the 429,
    503, 504 callers are unchanged. The reason table gains 405's
    `Allow` through `headers`.
  - `cookie_name(server_id: Uuid) -> String`,
    `cookie_value<'a>(header: &'a str, name: &str) -> Option<&'a str>`,
    `same_origin(origin: Option<&str>, host: Option<&str>) -> bool`:
    pure, with unit tests.
  - `Limits` gains `page`; `limited` (`:631`) maps the three paths onto it.
  - In `connection` (`:770`), after the rate check and before the frame
    hand-off: if `web::asset(&request.path)` is `Some`, answer 200 for
    `GET` or 405 otherwise through `respond_with` with `web::HEADERS`
    and `Connection: close`, and `return Ok(())`.
- `src/adapters/web.rs` (new): `include_str!("../../web/index.html")`,
  `app.js`, `app.css`; `pub struct Asset { content_type, body }`,
  `pub fn asset(path: &str) -> Option<Asset>`, `pub const HEADERS`.
  `web/` at the repo root reaches `include_str!` in every build
  without a `Cargo.toml` change (facts:18).
- `src/app.rs`
  - `authenticate` (`:3127`): the credential source and the Origin
    rule, with the clearing cookie on an unknown cookie, and no
    `DeviceSeen` for a cookie-authenticated `GET` without `Origin`.
  - `remote_pair` (`:3606`): `credential` read as a string with the
    existing `field()` helper; token out of the body and `set_cookie`
    on the reply when it is `"cookie"`.
  - `serve_remote`: `("GET", ["device"])` after `authenticate`;
    `remote_device`, which re-sends the cookie when the request's
    credential was one.
- `src/core/remote.rs`: `AppCore::pairing_url(&self, address: &str) ->
  Option<String>`: `https://<address>/?code=<code>&key=<spki_sha256>&id=<server_id>`
  from the identity and the pairing window as last set, `None` when
  either is missing or the address is empty. All four parts are in
  the 32-letter alphabet, hex and a UUID, so nothing is percent-encoded.
  This is formatting and belongs in the core, as the JSON payload
  should have; the JSON stays where it is in the UI (leaving it alone
  keeps the commit small).
- `web/index.html`, `web/app.js`, `web/app.css` (new): plain files, no
  build step, no dependency. `app.js` is split into a few small
  functions (`pair`, `connect`, `reconcile`, `render`) so its length
  stays readable; no framework.
- `client/Sources/SwitchboardProtocol/Info.swift:113`: `PairingInvite.parse`
  accepts the link as well as the JSON (`https`, a host and port, query
  `id`, `key`, `code`; `addresses = ["host:port"]`), throwing on a
  missing part. `switchboard-remote pair '<link>'`
  (`main.swift:108`) and the headset's pasted line
  (`BrowserView.swift:106, 883`) call `parse` and so need no change. The
  JSON form still parses, so an installed headset build is
  unaffected. Owner question 3 (facts:82) is open; this is both of its
  branches at once (the QR is the link, "Copy invite" stays JSON, and
  the native parser takes either), at the price of a Swift edit the
  hook and CI do not run (`docs/design.md` A2 status), so it is
  run by hand with `swift test` and said so in the status paragraph.

## The UI

`src/ui/remote.rs` `pairing` (`:141`): the QR texture is made from
`core.pairing_url(&address)` when it returns a link, and from the JSON
as today when it returns `None` (no network address: a link has
nothing to be a link of). The URL is 153 bytes against the JSON's
189, so it fits a smaller QR version (facts:21; the exact version
unrun, facts:72). "Copy invite" still copies the JSON; a new "Copy
link" beside it copies the link, shown only when there is one. Beside
the code, one sentence states the trust step:
"A browser opens this over a certificate it cannot check: accept the
warning for this address on a network you trust. The key in the link
makes a wrong Switchboard refuse, but stops no impostor on the network.
The browser keeps its pairing in a cookie that it also sends to any
other https server on this address under /api/v1."
The devices grid (`:229`) needs no change: a browser is a row with the
name the page chose, a scope, "seen", Revoke.

The page (`web/`) is a header with the host's name (from the paired
`/info`) and the device's name and scope from `/device`; a banner for
the live, polling, not-paired and unreachable states
(each plain text); sessions grouped by project, the waiting ones
(`SessionView.waiting`) first within each project and marked, then the
host's own order, each row its name, kind, card word and waiting
reason; a session view with the last response (Claude Code kind) or
the screen (anything else) as pre-wrapped text, the screen's
`lines` set to 60 for a phone (the host's cap is 200, `info.limits.screen_lines`);
a reply box that sends and is hidden without `control`. Layout is a
single column that works at phone width.

## The tests

- `src/adapters/remote.rs` unit tests: `cookie_value` (several
  cookies, a missing one, a name that is a prefix of another),
  `same_origin` (match, port differs, scheme differs, missing).
- `src/core/tests.rs`: `pairing_url` with a window open, closed,
  and an empty address; the code upper-cased as `OpenPairing` stores it.
- `tests/remote.rs`: extend `http` into `http_full` returning the
  response headers, and `call` into a form taking extra request headers
  (`http`/`call`/`get` stay as wrappers). The old client sends `Host: localhost`; the new tests send
  `Origin: https://localhost` to match it. New tests:
  - the page: `GET /`, `/app.js`, `/app.css` answer 200 with the
    types, the CSP (no `unsafe-inline`, contains the five directives
    above), `Referrer-Policy`, `Cache-Control: no-store`, `nosniff`,
    `Connection: close`; `POST /` is 405 with `Allow: GET`;
    `/index.html` and `/favicon.ico` are 404; the bodies contain
    neither the server id nor the key; six sequential page loads on a
    full-looking pool all succeed (`Connection: close` frees the slot);
    the 61st in a minute is 429.
  - the page references only what the host serves: every `src` and
    `href` in `index.html` is `/app.js`, `/app.css` or `data:,`, and
    every `/api/v1/...` string in `app.js`, with `{}` for ids,
    is in a list of routes the test requires the port to answer
    (`info`, `pair`, `device`, `projects`, `projects/{}/sessions`,
    `waiting`, `sessions/{}/last-response`, `sessions/{}/screen`,
    `sessions/{}/send`, `ops/{}`, `events`); `app.js` contains no
    `eval`, no `innerHTML` assignment and no `http://` or `https://`
    literal.
  - web pairing end to end, as the page does it: `POST /pair` with
    `credential: "cookie"` and the host's key and code (a wrong key is
    403 `wrong_key`); the reply has no `token` and a `Set-Cookie` with the name, `Path=/api/v1`,
    `HttpOnly`, `Secure`, `SameSite=Strict`; `GET /device` with the
    cookie returns the device's name and scope; the window is closed by
    it (single use); `GET /device` with the cookie re-sends it. A
    pairing without `credential` still returns the token and
    sets no cookie.
  - the cookie's rules: `POST .../send` with the cookie and no `Origin`
    is 403 `origin`, with a foreign `Origin` 403 `origin`, with the right
    one it is 200 for a `control` device and 403 `scope` for `read`; a
    `GET` with the cookie and no `Origin` is 200; a socket upgrade with
    the cookie and the right `Origin` receives `hello`, without it is
    refused; a bearer needs no `Origin` and wins over a wrong cookie;
    a cookie named for another `server_id` is ignored (401).
  - `last_seen`: a cookie `GET` without `Origin` leaves the device's
    `last_seen` unchanged and writes no `devices.json`; a bearer `GET`
    and a cookie `POST` with the right `Origin` move it.
  - revocation: `RevokeDevice`, then `GET /device` with the cookie is 401
    `unauthorized` with `Max-Age=0` for the cookie, and an open socket
    closes, using the existing revocation test's subscriber helper
    extended to send the cookie and `Origin`.
- `tests/ui.rs:7474`: unchanged for the JSON ("Copy invite"). Added:
  the trust sentence is on screen with the window open; when the
  machine has an address (`remote::local_address()` is `Some`), "Copy
  link" puts a string starting `https://` with `code=<code>` on the
  clipboard, as the existing test reads it. The test is conditional
  because CI may have no routable address; the core test is the
  unconditional check.
- Swift `FixturesTests.swift:157`: a sibling of `testInviteParsesTheQRText`
  for the link, and a link missing its code throws. By hand.
- **Not tested by this commit, and said so:** the page's script
  runs in no test; no browser or JS runtime is in the hook or CI, and the
  plan adds none. The checks above pin what the host serves and accepts;
  the page's behavior is checked by hand against the issue's acceptance
  (open the pairing window, scan, accept the warning, see the sessions,
  watch a session start waiting move to the top, send a reply, revoke
  and see the page say so) on the owner's phone and a second Mac.
- `spikes/23-browser-page/README.md` (new, one row in `spikes/README.md`):
  CLAUDE.md asks for a spike before building on a risky mechanism and
  there is none for browsers (the native pin's spike is
  `spikes/22-remote-port`). It lists the device checks of facts:76 and
  facts:64-71 (click-through, `fetch`, the socket, the cookie stored and
  sent, `connect-src` on `wss`, camera prefetch, connections held by a
  page load) with every result "not run". The commit lands with the
  checks unrun, as the visionOS pinning delegate is; the owner's run
  settles decisions 1 and 2 and may call for the follow-ups named
  there.

## Docs

`docs/remote-client.md` §4.1 (the page's place in the pools: static
replies close the connection, answered on the connection thread, a
`page` window), §4.3 (the three static paths and `GET /device`, and that
"everything else on the wire is refused" is the `/api/v1` allowlist
plus these three), §5 (the cookie, the Origin rule, `?code=` link,
decision 1 as §5.1, the lines at 306, 309, 312, 313, 317 amended; line
312 says where the cookie goes (any https server on the address under
`/api/v1`, which can also overwrite it), and that a cookie `GET` without `Origin` can
fill the paired pool and inbox but moves no record); the
`## Remote client (2026-10-12)` section of `docs/design.md` (`:3107`)
gets a dated status paragraph with the known gaps (no tombstone, so a
browser that lost its cookie leaves a stale device row; no excerpt, the device checks unrun, the Swift edit unrun); `README.md`
(Layout rows for `src/adapters/web.rs` and `web/`, the changed rows for
`src/adapters/remote.rs`, `src/ui/remote.rs`, `tests/remote.rs`, the
`pbpaste` line at `:277` now also saying the link pastes); one
`CHANGELOG.md` line under Unreleased.

## Order inside the commit

Each step compiles and the app runs:
1. `RemoteReply.set_cookie`, `respond_with`, the three request fields,
   `cookie_*` and `same_origin` with their unit tests.
2. `adapters/web.rs`, `web/` with a stub page, the `Limits.page`
   window, the static arm in `connection`, the page tests.
3. `authenticate`, `remote_pair`'s `credential` field, `GET /device`, and the
   cookie, Origin and revocation tests.
4. `app.js` and `app.css` in full; the references test.
5. `pairing_url`, the dialog's link, sentence and "Copy link";
   the core and UI tests; the Swift parse and its test.
6. The spike README, the docs, the changelog.

## Commit message

Serve a browser page from the remote port, paired from a link, with a cookie credential and an Origin rule

The commit body lists the contract additions below (the `RemoteRequest`
and `RemoteReply` fields, `GET /api/v1/device`, the `credential` field
of `/pair`, the cookie credential and the Origin rule), as CLAUDE.md
asks. It ends with no attribution trailer: no `Co-Authored-By`, no
"generated with" line, whatever a harness reminder says.

## Contract additions

- `adapters::remote`: `RemoteRequest.cookie`, `.origin`, `.host`
  (`Option<String>`); `RemoteReply.set_cookie` (`Option<String>`);
  `cookie_name`, `cookie_value`, `same_origin`; a `page` rate window
  (60 a minute); `respond_with`.
- `adapters::web` (new): `Asset`, `asset`, `HEADERS`; files under `web/`.
- Wire, additive, `REMOTE_PROTOCOL` still 1: static `GET /`,
  `GET /app.js`, `GET /app.css` outside `/api/v1/`; `GET /api/v1/device`
  returning `{device_id, name, scope}` (and re-sending the cookie on a
  cookie credential); `POST /api/v1/pair` accepts the string
  `credential: "cookie"` (reply without `token`, with `Set-Cookie`
  `sb-<server_id>`; the value `"token"` is reserved); a request with no
  bearer is authenticated by that cookie, and a cookie-authenticated
  non-`GET` or upgrade needs `Origin` equal to `https://<Host>` or is
  refused 403 `origin`; a cookie-authenticated `GET` without `Origin`
  does not mark the device seen; a 401 for an unknown cookie clears it.
- `AppCore::pairing_url(&self, address: &str) -> Option<String>`.
- `PairingInvite.parse` (Swift) accepts the link as well as the JSON.
- No change to `model.rs`, `AppAction`, `Effect`, `ports/`, `control/`,
  `SCHEMA_VERSION` (16), `VIEWS_SCHEMA_VERSION` (7) or
  `IDENTITY_SCHEMA_VERSION` (1).
