# Response to the review of the issue #230 plan

The revised plan is `/Users/sbgragent/Library/Application Support/Dispatch/tickets/b49c34f1/review/1/repo/plan.md`. The earlier plan under `plan/1` is unchanged. Every edit named below was re-read in the revised file before this response was written.

## 1. The key-confirmation step rests on nothing

Accepted. The reviewer is right on every count: `/info` is JSON the answering server writes, an impostor echoes the genuine key, and the key is in the QR URL the impostor's script reads. A browser cannot read its connection's key, so the page cannot pin.

Changed sections:

- **Where this departs from the issue, item 5**: rewritten. It says a browser cannot pin the host's key, so nothing on the page authenticates the host. The acceptance is now "accept the warning, see the sessions".
- **Pairing: the trust step (Decision 1)**: rewritten. It opens by saying what the first visit trusts and how an on-path attacker takes the code and the token. The default is the click-through, stated as the trust step. The URL key check is described as catching a different Mac answering the address (a stale QR after a DHCP change), and as not catching an impostor. The tap step and the "fallback if the owner declines the tap" paragraph are gone, along with the claim that the automatic comparison protects anything.
- **The mechanism that works**: the install option is named. The host serves its certificate at `GET /cert.der`, and the owner installs it and trusts it on the phone. It is the only option that stops an on-path attacker, at the cost of installing before scanning. It is listed for the owner, not built by default, because the issue asks for the click-through.
- **Spec and design amendments**: §5 pairing now says the browser path does not pin, and that "verifies the key before the code is sent" holds for native clients only.
- **Devices dialog**: the sentence now says the first visit trusts the certificate and nothing checks the host's identity.
- **Unresolved table**: the trust row is rewritten, with the install option as its fallback.

## 2. Departure 1 overstates what the fragment buys

Accepted. Reworded in **Where this departs from the issue, item 1**. The fragment keeps the code out of the request line, the host's logs and the host's caches. It does not stop the on-path attack; whoever answers the address still serves the page script, which posts the code. The spec §5 attack is named as not stopped, and Decision 1 says what the plan does about it.

The table row about history is also corrected (Unresolved table, "History, autocomplete or tab restore"). The old "None needed for the code" is gone. The code is still in the browser's history store, and the QR code is single-use and expires in five minutes, which bounds the exposure.

## 3. `__Host-` does not pin a cookie to the port

Accepted. The plan no longer says `__Host-` pins the cookie to a port. **Port: the URL, the cookie and the Origin check → Reply for `via: browser`** now says `__Host-` does not isolate ports, cites RFC 6265 §8.5, and names the cost: any other `Secure` service on this host, on any port, that reads cookies is sent `__Host-sb`. The cookie is sent only to this host's address, so a service on another machine never gets it.

The subprotocol alternative is described in the same section, with its cost stated honestly. Only the addressed server sees the token in the handshake, which avoids the sibling-service exposure. But REST calls cannot set a subprotocol, so the page would hold the token in script-readable storage, which loses `HttpOnly`.

The owner decision is in the Unresolved table, as a row "`__Host-sb` cookie read by other `Secure` services on this host, any port", with the default kept (cookie) and the subprotocol as the fallback. The "Why a cookie" line no longer claims port isolation. The plan keeps the cookie as its default because `HttpOnly` against script is the main threat, but the owner decides.

## 4. The Origin check fails open

Accepted. **Port → Pairing → Origin check** now requires an `Origin` on cookie-authenticated non-GET requests and on the events upgrade. It must equal `https://<Host>` exactly, string-equal, with the port. A missing `Origin` is 403 `origin`. Bearer requests are exempt, since native clients send neither an `Origin` nor a cookie.

On `Host`: the URL carries an IP address, never a name, because the certificate has no name to match (`src/adapters/remote.rs` 102). So `<Host>` is always an IP, bracketed for IPv6, with its port (`192.0.2.7:43187` or `[fd00::7]:43187`). A name the host does not serve fails the equality check. The test list has a missing-`Origin` case.

## 5. The camera-opens-a-bad-cert URL has no row or fallback

Accepted. Two changes:

- **Unresolved table**: a new row, "Phone camera opens an `https://` URL with a bad certificate", run item, default "URL in the QR", fallback "the URL as selectable text in the window; the owner types it into the phone's browser".
- **UI → Pairing window**: the URL is shown as selectable text beside the QR, not only under "Copy invite". That is the fallback, and it costs one label. "Copy invite" still copies the URL. The JSON form is not reintroduced; a phone cannot use it, which the reviewer noted.

## 6. `wss:` in `connect-src` is justified wrongly

Accepted. I took the second option the reviewer offered. **Port → Headers** now builds `connect-src 'self' wss://<Host>` per response from the request's `Host` header. The socket is allowed to the one host and port the request came in on, and to nothing else. The earlier sentence that a static policy "needs no host name" is removed, and so is the bare `wss:`.

Validation is in the same bullet: a `Host` containing anything outside `[A-Za-z0-9.:\[\]-]`, or no `Host`, gets 400 and no page. That blocks header injection. The page is `no-store`, so a per-request policy is never cached. A test in **Tests → `tests/remote.rs`** covers the bad-Host case and the CSP naming the test's own `Host`.

## 7. Line references that do not match the worktree

Accepted. Each cited range was checked against this worktree; here is what changed.

- **`src/app.rs` 3606**: the plan now says `remote_pair` builds the body at 3606–3672, with `token` going into it at 3661 and the reply sent at 3672. 3606 is the function signature, as the reviewer said.
- **`authenticate`**: the plan now says it reads only `request.bearer` today (`src/app.rs` 3128–3129) and that the cookie is a new second branch. The earlier "bearer first, as now" wording is gone.
- **`RemoteRequest` and `RemoteIncoming`**: `RemoteRequest` is at 360 and `RemoteIncoming` at 398, both in `src/adapters/remote.rs`. I checked `src/ports/`: neither type has a copy there. The plan says so, and that the frame-bound `RemoteIncoming` carries the new fields.
- **`src/ui/remote.rs` 85–100 and 230**: the host block with "Allow devices on the local network" is at 85–100, and `devices` is at 230. The Devices dialog section now cites both.
- **`docs/design.md` 3130–3160**: wrong. The LAN statement is at 3143–3148 ("the port does not listen"), and the plan cites that now. The 3130–3160 range is spike prose.
- **`tests/remote.rs` 245, 327, 406, 1587**: checked. `http` is 245, `get` 327, `pair` 406, `write_fixtures` 1587.
- **`tests/ui.rs` 7424 and 7474**: checked. The test is at 7424 and "Copy invite" is at 7474.
- **Schema versions**: `SCHEMA_VERSION` 16 (`src/core/model.rs` 12), `VIEWS_SCHEMA_VERSION` 7 (612), `IDENTITY_SCHEMA_VERSION` 1 (`src/ports/remote.rs` 17). Confirmed.
- **The limiter**: `limited` is at `src/adapters/remote.rs` 631, and it is called at 788. The plan now cites 788, not "~784".

## 8. Static answering versus the rate limiter is unspecified

Accepted, and the plan changes rather than only stating the order.

- **The order** is now explicit in **Port → Order in `connection`**: `read_request` (444, line 783), then the `limited` check (788, which answers 429 and continues), then the static answer, then the frame queue (`reply_tx` and `requests.try_send`).
- **The rate window** is new. Static paths get 120 requests a minute per listener (`STATIC_PER_MINUTE`, beside `PAIR_PER_MINUTE` at 583). The 121st gets 429 from the connection thread. The plan previously had no window; the reviewer's churn point is why it has one now.
- **The starvation exposure** is stated in **Pools**: a `Connection: close` flood of `/` can churn the four slots and starve `/info` and `/pair`. The plan says this exposure already exists for `/info` and `/pair` and §4.1 accepts it. The window bounds the churn rate; it does not remove the exposure.
- **Owner Q6** is rewritten in the Unresolved table with that default, and the fallback is "no window; the flood exposure is the same as `/info` and `/pair` today". The reviewer's point that the Q6 row covered it only half is answered by the bullet in Pools, which names the exposure in full.
- The "no rate window" sentence from the previous plan is gone.

I did not cut the static path. Cutting it would take the page with it, and the issue asks for the page.

## 9. The Swift change is undeclared

Accepted. The plan now declares it in a new section, **Swift edits**, and the commit message stays "Rust and Swift together". The list:

- `client/Sources/SwitchboardProtocol/Info.swift` (`PairingInvite.parse`, 114): accepts the URL form in addition to the JSON. The three callers do not change: `main.swift` 111 and `BrowserView.swift` 106 and 883 call `parse`.
- `client/Sources/switchboard-remote/main.swift` line 11, the usage text: `pair '<invite json>'` becomes `pair '<invite url or json>'`.
- `client/Tests/SwitchboardProtocolTests/FixturesTests.swift` (161): one case for the URL form.

I took the URL-everywhere option. The cheaper "keep JSON Copy invite" option would leave the URL out of the window, which the phone needs. The CLI and both headset paste sites keep their JSON form, because `parse` still accepts it.

On the CLAUDE.md contract note: none is added. The CLAUDE.md contract is `src/core/model.rs`, `src/core/action.rs` and `src/ports/`, and none of them changes. The wire form is in `docs/remote-client.md` §5, which the spec amendments change.

## Owner questions this round added

The plan now names these for the owner, each with its default and fallback in the Unresolved table: the install option for the trust step, the `__Host-sb` cookie's exposure to other `Secure` services on the host, and the static-path flood exposure.
