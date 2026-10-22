You are scoring one implementation plan for a GitHub issue, blind: you do not know who or what wrote it, and you score this plan on its own.
The repository at the issue's base commit is checked out at /Users/sully/code_repos/plan-trial/sb-230-base (read-only; change nothing there). Every file path in the plan is relative to that tree.

Score by the rubric, column by column. Length is not thoroughness. For grounding, take the first five distinct file or line references the plan makes, in order of appearance, open each in the tree, and say what you found. For departures and decisions, use the reference as the yardstick; where you disagree with the reference, say so in the evidence rather than silently scoring your own view.

Your final message must be exactly this shape and nothing else:

```
| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| R | <0-3> | <0-3> | <0-3> | <0-3> | <yes|note|no> |
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

# The plan (R)

# Plan for #230: a web control surface on the remote port

The host serves a small browser page from the remote port it already has. The page pairs from a URL the pairing window's QR carries, keeps its token in the page's own storage, reads and writes through the same `/api/v1/` endpoints the headset uses, and follows `/api/v1/events`. One commit. Every file and line below was checked in this worktree; "the facts" is `facts.md`.

## Where this departs from the issue

- **"The one-line excerpt the card shows."** `SessionView` has no excerpt (`control/src/lib.rs:637-666`: `card` is the card's word, `waiting_reason` is a string, and there is no `last_message`; the Stop-hook text exists only in `src/adapters/hooks.rs:71` and `src/ports/events.rs:24`; the facts say the same). The page shows `card` and, when `waiting`, `waiting_reason`. No contract change. This rests on a line under **For the owner** (does the card need `last_message`?).
- **"No new read model unless the page cannot do without one."** The page cannot do without one. The header needs the device's scope and the reply box needs it after every reload, and `/info` carries no per-device scope (`src/app.rs:3568-3600`; the scope appears only in the `/pair` reply, `:3650`). Scope also changes under the page from the Mac's dialog, so what the page stored at pairing goes stale. The plan adds `GET /api/v1/me`.
- **The facts say `/pair` applies no cap to `device_name`** (`src/app.rs:3650-3656`). The cap is one call down: `AppCore::new_device` trims and takes 64 characters (`src/core/remote.rs:169`). The page sends its own name (`Safari on iPhone`). Control characters are not stripped there, and the page chooses the name, so `new_device` also drops them (`.filter(|c| !c.is_control())` before the 64-character take; a core test). The name reaches exactly three places, all checked: the dialog (`src/ui/remote.rs:242`, plain text), the pairing log line (`src/app.rs:3666`, formatted with `{:?}`, which escapes), and the new `/me` reply (JSON, escaped). There is no device listing endpoint, and the Swift client only sends a name (`ConnectionManager.swift:371`), never reads one.

## The record

No record changes and no schema bump (`SCHEMA_VERSION` 16, `VIEWS_SCHEMA_VERSION` 7, `IDENTITY_SCHEMA_VERSION` 1 today). A browser is a `PairedDevice` (`src/ports/remote.rs:96`) in `devices.json`, made by `remote_pair`, listed and revoked in the dialog as today.

The page's only state is the credential, in `localStorage` under the key `switchboard.device`: `{token, device_id, server_id, spki_sha256}`. Everything else is a cache rebuilt from the REST snapshots, the way the headset does it (spec §4.4, "Subscribe first, then snapshot").

**Decision 2: the token is held by the page, not a cookie.** This rests on a line under **For the owner**, and the choice is made here:

- The facts (Settled) say a cookie goes to every port on the host's IP and can be overwritten by another service there, and needs an `Origin` check, a `Set-Cookie` field on `RemoteReply` and the `me` read anyway. `localStorage` is per origin, and an origin includes the port.
- The cost is that script on the origin can read the token. The defence is a page with no inline script and no external resource (the CSP below), and a rule that nothing from the host (a session name, a transcript, a screen) ever reaches `innerHTML`; the page writes text only with `textContent`. The `app.js` test checks that `innerHTML`, `outerHTML`, `insertAdjacentHTML`, `document.write` and `eval` do not occur. `style-src 'self'` blocks a `style` attribute set from script (`setAttribute('style', ...)`, `cssText`) and the page is written to need none: it changes appearance only with `classList`, and the test bans `setAttribute('style'`, `cssText` and `.style.` in `app.js`. This is a premise of the plan, not a fact from the facts file.
- The WebSocket carries the token as a `Sec-WebSocket-Protocol` value (facts, Settled: a base64url token qualifies as a subprotocol token). The page offers `["switchboard.v1", "<token>"]`; the host echoes only `switchboard.v1`, so the token is never reflected in a response. Every `fetch` sends `Authorization: Bearer`. No credential is attached by the browser on its own, so no `Origin` check or CSRF defence is needed (facts, Settled: `Origin` matters only for a cookie-carrying upgrade).

**Decision 1: the trust step is the click-through, said in the dialog.** This rests on **Unresolved** (whether Safari, Chrome and Firefox allow a click-through of a SAN-less, IP-host certificate; no browser could be driven) and on **For the owner** (click-through or download). Built: the certificate is not changed. The dialog says that a browser warns about the certificate and, if it lets the owner continue, to do so on this address only (wording under "The UI"); it does not promise the click-through works. What this buys and what it does not:

- The `key` in the URL is checked against `/info`'s `spki_sha256` by the page, and `remote_pair` refuses a body whose `spki_sha256` is not the host's own (`src/app.rs:3631`). That catches a stale QR and a mistyped or reassigned address.
- It does not catch an impostor: a page served by an impostor is not the genuine page, and its `/info` can say anything. A browser cannot pin a key; the owner's click is the trust step, as the issue says.
- Fallback if a browser refuses to continue, or the owner chooses download: a later change reissues the certificate from the same key with a SAN and serves it at `GET /cert.der`. Not built here.

**Decision on the code in the URL (For the owner).** Built as the issue says: `?code=` is in the URL, removed by `history.replaceState` as the first statement of `app.js`. The code is single-use, lives five minutes and three wrong tries close the window (spec §5), and the token is never in a URL. If the owner says no, the fallback is one edit in `src/ui/remote.rs`: leave `code` out of the URL. The page already has a code entry (below) for a visit with no code.

## The signals

All on the host. The page adds none to the core.

- **`GET /` , `/app.js`, `/app.css`** are answered on the connection thread, after the rate check and before the frame sees the request (`connection`, `src/adapters/remote.rs:770`). The rate check stays the first thing in the request loop (`:788`) and `Limits::limited` (`:631`) gains the three page paths mapped to the `page` window, so the window runs before the static branch and a flood is answered 429 first; the static branch follows it. The files are fixed bytes, so the frame's 8-per-pass drain and 64-deep inbox (`:437`, `INBOX` at `:578`) are not spent on a page load. Methods: `GET` and `HEAD` (no body, true `Content-Length`); any other method on these paths gets 405 with `Allow: GET, HEAD`. Every other path still goes to the frame and is refused 404 JSON as today (`src/app.rs:3024`).
- **Headers on all three:** `Content-Type` (`text/html; charset=utf-8`, `text/javascript; charset=utf-8`, `text/css; charset=utf-8`), `Cache-Control: no-store`, `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`, `Content-Length`, and `Connection: close`.
- **CSP**, on the HTML: `default-src 'none'; script-src 'self'; style-src 'self'; img-src data:; connect-src 'self' wss://<host>; base-uri 'none'; form-action 'none'; frame-ancestors 'none'`. `<host>` is the request's `Host` header, kept by `read_request` as a new field and accepted only when it matches `[A-Za-z0-9.:\[\]-]{1,255}`; otherwise the CSP carries `connect-src 'self'` alone. The facts (Settled) say the host is an IP that varies, which is why it is taken per request.
- **Pool and window (Decision 4).** A static reply is unauthenticated, so it counts in the 4-slot unauthenticated pool (spec §4.1) while its connection lives; `Connection: close` makes that milliseconds, which is the mitigation the facts name. This rests on **Unresolved** (parallel connections of a real page load need a real browser); the acceptance run is the check. A third window `page` (120 per minute per listener, `PAGE_PER_MINUTE`) is added to `Limits` beside `pair` and `info` (`:623`), so a page flood cannot starve `/info`; it answers 429 JSON as the others do.
- **The paired pool (`MAX_PAIRED` 12, `src/adapters/remote.rs:568`; a connection moves there on its first authenticated reply, `:835-848`; past 12 the host answers 503 `full`, `:838-842`).** Nothing in the facts says a page's connections fit, so the page is built to use few and to read `full` correctly. All its `fetch` calls go through one queue with at most two in flight, so with the one WebSocket a tab holds at most three connections and normally two, since a browser reuses an idle keep-alive connection. A 503 `full` (or any 503) is never "revoked": the page shows "The host has too many connections; close other tabs", and retries with the capped back-off. Only a 401 on a request that carried a token means "revoked". `MAX_PAIRED` is not changed.
- **No code, no token.** The page loads, calls `/info`, and shows "This device is not paired", the host's name and the short key, a code field, and where to open the pairing window. A typed code pairs against the key `/info` presented, as the headset's Bonjour path does (spec §5, the owner compares the short key by eye).
- **`GET /api/v1/me`** (read scope, after `authenticate`): `{device_id, name, scope, server_name}` in the usual envelope. `authenticate` returns the device as it is now (`src/app.rs:3127`), so a scope changed in the dialog is seen on the next `me`.
- **Token on the upgrade.** `read_request` (`:444`): when `Authorization` is absent and the request is a WebSocket upgrade, the `Sec-WebSocket-Protocol` list is split on commas; when it holds `switchboard.v1`, the other entry is the bearer and `RemoteRequest.echo_protocol` is set. `respond_upgrade` (`:519`) takes that flag and writes `Sec-WebSocket-Protocol: switchboard.v1` only then, so tungstenite and the headset, which offer none, get the same 101 as today. The bearer stays inside `RemoteRequest.bearer`, never logged with the headers.

## The actions and effects

- **Core:** none. No `AppAction`, no `Effect`, no change to `src/core/`, except one pure function in `src/core/remote.rs`: `invite_url(addr: SocketAddr, server_id: &str, spki_sha256: &str, code: &str) -> String`, producing `https://<addr>/?code=<code>&key=<spki>&id=<server_id>`. CLAUDE.md puts logic out of `ui/`, so the builder lives here with a unit test.
- **Ports:** none.
- **Adapters.** New `src/adapters/web.rs`: the three `include_str!` assets from `web/`, a `Page` table of path to content type, `fn response(method: &str, path: &str, host: Option<&str>) -> Option<Vec<u8>>` returning the whole HTTP reply (pure, unit-testable with no socket), and the CSP builder. Registered in `src/adapters/mod.rs`. `src/adapters/remote.rs`: the `connection` branch, the `page` window, `RemoteRequest.host` and `.echo_protocol` (additive; `a_request_reads_with_its_query_bearer_and_body` at `:1045` builds the expected value, so it gains the two fields).
- **App.** `src/app.rs`: the arm `("GET", ["me"])` after `authenticate` in `serve_remote` (the function begins at `:3017`; the arm goes after its `authenticate` call), `remote_me`, the doc comment at `:3014-3016` rewritten ("`/info`, `/pair` and the page itself to anyone"). `pairing` in `src/ui/remote.rs` is the only caller of the URL builder.
- **Swift.** `PairingInvite.parse` (`client/Sources/SwitchboardProtocol/Info.swift:114` (the doc comment is at `:113`)) accepts the URL as well as the JSON: when the trimmed text starts with `https://`, read `URLComponents` for host and port (re-bracket an IPv6 host) and the query items `code`, `key` and `id`, build `addresses` as the one `host:port`, and throw the existing decode error when any is missing. The memberwise `init` is untouched.
- **Decision 3: one QR.** The QR, the text beside it and `Copy invite` all carry the URL. The JSON form is no longer written. It stays readable by `parse`, so an invite already on a clipboard or in a script still pairs.

## The UI

**The dialog (`src/ui/remote.rs`).** `pairing` (`:141`) builds the URL with `invite_url` in place of the JSON at `:147-155`; the QR is made from it (`qr_texture`, `:280`) and `Copy invite` copies it. With no LAN address (`local_address()` is `None`) the URL uses `127.0.0.1:<port>`; the "Address" line is shown only when an address was found, as now. Whether a phone reads the QR at `QR_SCALE` is **Unresolved** and is part of the acceptance run. Under the code the hint reads: "Scan with a phone's camera. A browser warns about this host's certificate; if yours lets you continue, do so on this address only." It does not promise that every browser lets the owner continue, because that is untried. The footer that states what redaction covers (`:73-83`) gains one sentence: a browser paired this way is a device like any other, listed above, and Revoke ends it on its next request.

**The page (`web/index.html`, `web/app.css`, `web/app.js`).** Plain HTML, CSS and one script, no framework, no build step. Logic that a test must reach (grouping, ordering, applying an event by revision, the state machine, the device name from the user agent) is in pure functions at the top of `app.js`; the DOM code follows, exported for Node.

- **Start.** The first statement reads `location.search` and calls `history.replaceState(null, "", location.pathname)`; nothing else runs before it. Then `GET /info`.
- **States, each worded plainly:**
  - *unpaired*: described above, with the code field.
  - *pairing*: URL had `code`, `key`, `id`. If `/info`'s `server_id` or `spki_sha256` differs from the URL's: "This address is not the host whose code was scanned." and no `POST`. Else `POST /api/v1/pair {code, spki_sha256: key, device_name}`, with the errors the host gives (`wrong_code`, `pairing_closed`, `wrong_key`, 429) each said in a sentence.
  - *identity changed, pair again*: a stored credential and `/info` shows a different `spki_sha256` or `server_id`. The token is not sent and the stored credential is cleared.
  - *revoked*: the key matches and `/me` or any call answers 401. "This device was removed on the Mac. Pair again."
  - *paired*: the list.
  A URL with a code replaces any stored credential after the comparisons pass.
- **The list.** `GET /projects`, then `GET /projects/{id}/sessions` for each, grouped by project; within a group the waiting sessions first, marked with a "Waiting" tag and `waiting_reason`, then the rest by name; each row shows the name, `kind`, `card` and the reason. The header shows the host name and the device's `scope` from `/me`.
- **A session.** `GET /sessions/{id}/last-response` for a Claude Code session, `GET /sessions/{id}/screen?lines=60` for anything else, in a `<pre>` through `textContent`; refetched when a `changed` names the session or its `conversation` or `screen` kind, plus the 30-second refetch the spec gives a visible content view.
- **Live.** Open the socket, wait for `hello {epoch, revision}`, then snapshot, then apply buffered events by revision, exactly as spec §4.4 says; a changed `epoch` or `resync_required` refetches everything; a close reads `/me` first (revoked or not), then reconnects with a capped back-off. A session that starts waiting moves to the top because `changed {waiting}` and `changed {session}` refetch the project's sessions, and the order is a pure function of what was fetched.
- **Reply.** Shown only when `/me` says `control`. It posts `{op, text}` to `/sessions/{id}/send` with a fresh UUID as `op`, disables the box until it is accepted, and on a timeout reads `/ops/{op}` before it ever sends again (spec §4.5). A 403 `scope` re-reads `/me` and hides the box. Text 1 byte to 16 KiB, as the host enforces.
- **The device name** is `<browser> on <device>` from `navigator.userAgent` (`Safari on iPhone`, `Chrome on Android`, `Firefox on Mac`, else `Browser`), at most 64 characters.
- Every endpoint the script calls is one `/api/v1/…` string in a table at the top, for the endpoint test.

## The tests

- **Core** (`src/core/tests.rs` beside the other remote tests): `invite_url` for an IPv4 address, an IPv6 address (brackets), and the loopback fallback address; `new_device` drops control characters and still takes 64.
- **Adapter units** (`src/adapters/web.rs`, `src/adapters/remote.rs`): `response` for each path and for an unknown one; the CSP names `wss://<host>` for a valid `Host` and falls back for a bad one; `index.html` has no inline `<script>`, no `on*=` attribute, no `style=` attribute, `app.js` has none of `setAttribute('style'`, `cssText`, `.style.`, and `index.html` references only paths in the table (or `data:`); `read_request` keeps `host`, takes the token from `Sec-WebSocket-Protocol` only when `Authorization` is absent, and sets `echo_protocol` only for `switchboard.v1`; the `page` window slides like `the_rate_window_slides` (`:1064`).
- **The port, end to end** (`tests/remote.rs`). `http` (`:245`) reads only the status and body, so a variant `http_head` returns the response headers; a request there can carry `Host` and `Sec-WebSocket-Protocol`.
  - The three paths answer 200 to an unpaired client with the headers above, and `GET /anything-else` is still 404 `not_found`. `the_allowlist_refuses_everything_else_with_no_side_effect` (`:682`) is amended to say so, and `info_is_served_to_anyone_and_every_other_read_is_refused_unpaired` (`:426`) gains the page paths.
  - The pairing flow as the page performs it, using the host's own pinned client as the CLI does: open a window, read `/info`, compare the key, `POST /pair` with a UA-style `device_name`, then `GET /me` with the token (name, scope), a `read` device refused `send` with 403 `scope`, the scope raised and `/me` showing `control`, revoke and `/me` answering 401.
  - A subscription offered `["switchboard.v1", "<token>"]` and no `Authorization` header gets the 101 with `Sec-WebSocket-Protocol: switchboard.v1` and `hello` (`subscribe_at`, `:1960`, takes an optional subprotocol); a wrong token gets 401 before any upgrade; a subscription with the bearer header and no subprotocol gets a 101 without the header.
  - Page flood: more than `PAGE_PER_MINUTE` requests answer 429, and `/info` still answers.
  - Every `/api/v1/…` string in `web/app.js`, with `${…}` replaced by ids seeded in the test's own workspace, answers something other than the 404 with `error.code == "not_found"` that `refuse` gives an off-allowlist path (`src/app.rs:3024`), for a `control` device.
- **UI** (`tests/ui.rs:7474-7492`): the "Copy invite" assertions change from JSON to the URL: it starts with `https://`, and carries `code=<the window's code>`, `key=<the identity's key>` and `id=<server id>`.
- **Swift** (`client/Tests/SwitchboardProtocolTests/FixturesTests.swift:161`): `testInviteParsesTheQRURL` beside the JSON test (host, port, code, key, id; an IPv6 URL; a URL missing `code` throws). Run with `swift test` in `client/`.
- **The page's logic** (`web/app.test.js`, run with `node --test web/`): grouping and ordering with a waiting session moving to the top, applying a buffered event only when its revision is past the snapshot's, the state machine's five states, a 503 `full` shown as a retry and never as revoked, at most two fetches in flight, the device name for four user agents, and that the source contains none of the banned HTML-writing calls. Node is on the dev machine (facts, Settled); CI was not checked, so this is not added to the workflow and is a documented local command. `node --check web/app.js` is run before the commit.
- **Not claimed.** No test here drives a browser. The acceptance run (scan, accept the warning, see the sessions, a session starts waiting and moves up, reply from the phone, revoke) needs a signed bundle and a phone, which is a **For the owner** line; if neither is available, what is claimed is the host-side tests and the Node checks only.

## Docs, README, changelog

- `docs/remote-client.md` §4.1, §4.3, §5 and §11 are updated for the page, `/me`, the subprotocol and the URL invite.
- `docs/design.md`: a bullet at the end of the `## Remote client (2026-10-12)` section (it ends before `## Versioning and updates`, line 3585) saying what is built and the known gaps, stated as gaps: no browser has been tried, so whether Safari, Chrome and Firefox let the owner continue past this SAN-less certificate on an IP address is unknown (the written fallback is the SAN-bearing certificate and `/cert.der`), nor whether the exception survives an address change, nor whether hand-built responses read in all three; the camera app may see the code before `replaceState`; the excerpt is `card` and `waiting_reason`; the paired pool is unchanged. Lines 3339 and 3571 (the invite is pasted) are reworded for the URL.
- `README.md`: Layout table rows for `src/adapters/web.rs`, `web/index.html`, `web/app.css`, `web/app.js`, `web/app.test.js`; the `remote.rs` adapter row (page, `me`, subprotocol); the dev-aid text at lines 276-278 (`Copy invite` puts the pairing URL on the clipboard; `switchboard-remote pair "$(pbpaste)"` still pairs it); the `tests/remote.rs` row. `client/README.md` and the usage text in `client/Sources/switchboard-remote/main.swift` say "invite json" and are reworded.
- `CHANGELOG.md` under `## [Unreleased]`: "The remote port serves a web page: scan the pairing QR with a phone's camera and the page lists the sessions with the waiting ones first, shows a session's last response or screen, and sends a reply. The pairing QR and `Copy invite` now carry a URL. Not yet tried in a browser: the host's certificate is self-signed and a browser may refuse to continue past its warning."

## Commit

`Serve a web control surface on the remote port, paired from a URL in the QR`

Contract additions (copy into the commit body):

- `GET /api/v1/me` (read scope): `{device_id, name, scope, server_name}` in the envelope; not listed in `info`'s `capabilities`.
- Static, unauthenticated `GET`/`HEAD` of `/`, `/app.js`, `/app.css` on the remote port, answered on the connection thread with a CSP, `no-store`, `nosniff`, `no-referrer` and `Connection: close`; a `page` rate window of 120 a minute per listener.
- `Sec-WebSocket-Protocol: switchboard.v1, <token>` accepted as the bearer on the `/api/v1/events` upgrade when `Authorization` is absent; the 101 echoes `switchboard.v1` only when offered.
- `RemoteRequest.host: Option<String>` and `RemoteRequest.echo_protocol: bool` (additive).
- `core::remote::invite_url`; `AppCore::new_device` drops control characters from a device name; the pairing QR and `Copy invite` carry `https://<addr>/?code=&key=&id=` in place of the JSON.
- Swift: `PairingInvite.parse` accepts the URL form as well as the JSON.
