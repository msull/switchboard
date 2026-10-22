1. **The key-confirmation step (Decision 1) rests on nothing, and the tree and spec argue against it.** The plan says the page shows "the `spki_sha256` the host presented (from `/info`)" and compares it with the URL's key (steps 2–3), and that an impostor "would have to show the owner the Mac's key, which it does not have". Both claims fail.
   - `/info` is a JSON body the answering server writes (`src/app.rs` `remote_info`, ~3100). Page JavaScript cannot read the TLS certificate's SPKI. An impostor that serves the page and `/info` simply echoes the genuine key.
   - The impostor has the key anyway. It is in the QR URL's fragment, which the impostor's own script reads, and the Mac's window shows it (`src/ui/remote.rs`).
   - Step 3's automatic comparison is therefore vacuous, and the owner's tap compares two strings the attacker controls.
   - Spec §5 (`docs/remote-client.md` ~310) requires the client to compare the key of the connection it is actually on, which a browser cannot do. The plan contradicts that settled line without citing the tree against it, and it reaches the amendment in "Spec and design amendments" only as "add the browser trust step".
   - Either say plainly that a browser cannot pin and drop the claim that this step authenticates the host, or name a mechanism that works. Examples are the owner comparing the key in the browser's own certificate viewer, or serving the Mac's certificate for install (facts "For the owner" 1).
   - The same flaw hits departure 5 and the "Fallback if the owner declines the tap" paragraph. That paragraph's "automatic comparison" protects nothing.

2. **Departure 1 overstates what the fragment buys.** The plan says the query string "hands the code to whoever answers the phone's first TLS connection". With a fragment, whoever answers still serves the page script, and that script reads the fragment and posts the code. The fragment only keeps the code out of the host's request line, logs and caches, and out of an impostor's logs. It does not stop the §5 intermediary attack. Reword it. As written, the table row "None needed for the code" understates the exposure.

3. **`__Host-` does not pin a cookie to the port, and the plan says it does.**
   - The plan says "`__Host-` pins the cookie to this host and port's origin". The facts file marks this as a lead to confirm (Primary sources, Cookies). I settled it: RFC 6265 §8.5 (https://www.rfc-editor.org/rfc/rfc6265) says "Cookies do not provide isolation by port", and the cookie "is also readable by a service running on another port of the same server."
   - `__Host-` requires `Secure`, `Path=/` and no `Domain`. It does not isolate ports.
   - The session token would therefore go to any other TLS service on the Mac's address. `SameSite=Strict` does not help either, because same-site ignores the port.
   - The plan needs to accept this and write it down, or keep the token out of a cookie. The subprotocol fallback has the opposite problem (token in the 101 headers), so this is a real tradeoff for the owner. It is not a settled "Why a cookie".

4. **The Origin check fails open.** The plan refuses only an `Origin` that is "present and differs". A cookie-authenticated non-GET or `/events` upgrade with no `Origin` is accepted. For cookie auth, require `Origin` to be present and equal to `https://<Host>` on POST and on the upgrade. Only bearer requests are exempt. Also say what `Host` is compared against, since the host can be an IP, an IPv6 literal or a `.local` name.

5. **Camera-opens-a-bad-cert URL is unresolved and has no row or fallback.**
   - The facts file lists "Whether a phone camera opens an `https://` URL that has a bad certificate" as unresolved.
   - The plan replaces the QR payload with that URL (UI section) and drops the JSON form from the QR, but its Unresolved table has no row for it.
   - The only fallback is "Copy JSON" under the one-QR-or-two row, which a phone cannot use.
   - Add a row and a fallback, such as keeping a second QR or the JSON payload in the window.

6. **`wss:` in `connect-src` is justified wrongly.**
   - The facts file says `'self'` does not reliably cover the socket and suggests `wss://<host>:<port>` or learning the host from `location.host`.
   - The plan picks bare `wss:` and says a static policy "needs no host name". That permits a connection to any `wss` host, which weakens the exfiltration protection the CSP is there for.
   - Either justify the width, or have the host build the header per request from `Host`. The connection thread already has `Host` once the plan keeps it on `RemoteRequest`.

7. **Line references that do not match the worktree.**
   - `src/app.rs` 3606 is cited as where `/pair` returns the token body. It is the `fn remote_pair` signature. Locate the reply construction.
   - The plan lists "`src/app.rs` 3023" for the 404. The refusal is at 3023–3025 and is fine, but the "bearer first" claim about `authenticate` must be re-cited, because `authenticate` is at 3127 and begins with `request.bearer` only.
   - `RemoteRequest` is at `src/adapters/remote.rs` 360 and `RemoteIncoming` at 398. Both sit in the adapter, so "not the shared contract" holds. However, `src/app.rs` constructs them, so the plan should confirm there is no `ports/` copy.
   - Re-check the remaining cited ranges (`src/ui/remote.rs` 85–100 and 230, `docs/design.md` 3130–3160, `tests/remote.rs` 245/327/406/1587, `tests/ui.rs` 7424/7474) against the tree before the plan is final. I did not verify them all.
   - Schema versions match: `SCHEMA_VERSION` 16, `VIEWS_SCHEMA_VERSION` 7, `IDENTITY_SCHEMA_VERSION` 1.

8. **Static answering versus the rate limiter is unspecified.** `limited` (`src/adapters/remote.rs` 631) returns false for every path but `/pair` and `/info`, so static files are unlimited. The plan states this ("no rate window"), but its placement "after `read_request` and before `requests.try_send`" sits after the `limited` call at ~784. State the order explicitly, and note that a `Connection: close` flood of `/` can churn the 4-slot pool and starve a legitimate `/info` or `/pair`. That is an owner question the Q6 row only half covers.

9. **The Swift change is undeclared.** The commit says "Rust and Swift together" and the tests rely on `PairingInvite.parse` accepting a URL (`client/Sources/SwitchboardProtocol/Info.swift` 94–114). The plan lists no Swift edit (no `main.swift` 111 or `BrowserView.swift` 106/883 changes). Facts "For the owner" 3 offers keeping JSON "Copy invite" so nothing in Swift changes. The plan chose the opposite without stating that scope. List the Swift edits and the CLAUDE.md contract note, or take the cheaper option.
