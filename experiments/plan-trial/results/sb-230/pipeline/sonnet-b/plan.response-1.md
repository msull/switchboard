# Response to feedback 1

Only `plan.md` in this directory was edited. The earlier plan is untouched.

1. **Trust step proves nothing.** Agreed, and the plan now says so. I took your options 2 and 4 together.
   - *Where it changed:* "Where this departs from the issue" (first bullet) and "Decision 1: the certificate".
   - The instruction to compare the key on the page is gone. The page's key is labelled "as the host reports it" and only tells two Switchboards apart.
   - Decision 1 now writes the weakening of §5 and design.md's "key verified before the code is sent" as a deliberate, scoped exception for a browser. It says what bounds it: the code's limits, the scope chosen in the window, the device showing in the dialog, and Revoke.
   - The plan marks the exception as needing owner sign-off, and the order has a step 0 that stops everything without it. I can't give that sign-off.
   - Option 1 (a real binding) is not available. A phone's certificate dialog shows neither the SPKI hash nor a usable fingerprint. The plan adds an optional "Certificate" SHA-256 line in the pairing window for desktop browsers, and says it isn't required.

2. **Dropping the JSON is a removal.** Agreed, and the plan no longer drops it.
   - *Where it changed:* "Decision 3: one URL", the UI section, the tests and "Contract additions".
   - "Copy invite" still copies the JSON, unchanged, with the `addresses` array. A new "Copy link" copies the URL, and `PairingInvite.parse` accepts both.
   - Only the QR payload becomes the URL. I searched `client/Sources` for a camera or scanner and found none, so no shipped client reads the QR.
   - The host's invite carries one address today, so nothing is lost.

3. **Reachability.** Fixed. I checked `bind_remote`: it binds loopback unless `identity.listen.lan` is set.
   - *Where it changed:* "The UI", new "Reachability" paragraph, with a test in "The tests".
   - On loopback the pairing window hides the QR and "Copy link" and tells the owner to allow the local network. With no network address it says that instead. The code, key and "Copy invite" stay.
   - A second UI test covers the loopback case, and the existing test now sets `listen.lan`.

4. **Origin rule incomplete.** Agreed, and the rule is wider now.
   - *Where it changed:* "Decision 2", the `authenticate` paragraph, and the Origin tests.
   - The rule now covers every cookie-authenticated request, reads and upgrades included. It needs an IP-literal `Host`, `Sec-Fetch-Site: same-origin` when the browser sends it, and a matching `Origin` when present.
   - A cookie `GET` with neither header is refused. A browser too old to send either can't use the page, and the page says so.
   - Another port on the same IP is `same-site`, so it is refused.
   - I didn't make `Origin` mandatory on every `GET`. Same-origin `fetch` GETs omit it in some browsers, so `Sec-Fetch-Site` is the check there. `RemoteRequest` gains `sec_fetch_site`.

5. **One cookie for two hosts.** Fixed.
   - *Where it changed:* "Decision 2" and a new two-hosts test.
   - The cookie is `__Host-sb-<server_id>`, which `__Host-` allows, and `authenticate` reads only its own.
   - A collision can no longer produce the page's "revoked" state. A 401 means this host doesn't know the token.

6. **`/me` and fixtures.** Agreed. I said "no fixture" because only the page reads it, which was wrong.
   - *Where it changed:* "Port changes" (`/me`) and the tests' "Fixtures" bullet.
   - Added `client/Fixtures/me.json` from `write_fixtures`, a small `DeviceMe` type in Swift, and a line in `FixturesTests.swift`.

7. **Rate-limit accounting.** Specified now.
   - *Where it changed:* "Pools and rate".
   - `limited` first asks `web::is_page(path)`, the same fixed `match` that serves the files, and uses a separate `page` window.
   - Only page requests count. A WebSocket reconnect storm or API traffic can't lock out a reload, and a reload loop can't starve `/info` or `/pair`.
   - A page 429 is the JSON error with `no-store` and `nosniff`, and no CSP.

8. **JS untested; the spike can sink Decision 2.** I did not split the commit, because the task asks for one. The gate is now explicit instead.
   - *Where it changed:* "Spike first", "Order inside the one commit", "Decision 2" and "Contract additions".
   - The cookie and `wss://` rows must be measured on Safari on an iPhone and on Chrome before the cookie step starts.
   - If they fail, or there is no phone, the work stops with nothing committed and goes back to the owner.
   - The cookie items in the contract are marked conditional on the spike and the sign-off.
   - The tests section now says plainly what the Rust tests do and don't cover. I added a test that `replaceState` comes before the first `fetch` or `WebSocket` in `app.js`. The browser-only claims rest on the spike and the acceptance run.

9. **Process and docs.**
   - *Line 3573:* I checked it. It is the "Known gaps at the first merge" bullet inside the single "Remote client (2026-10-12)" section, which ends at `:3584`. The remote port has no "Milestone N status" block. The plan now says this and leaves that bullet alone. It adds a "The web surface" bullet before it and a "Known gaps of the web surface" bullet after it.
   - *Literal count:* it was a line count and was wrong. `grep -o` gives 28 literals in `src/app.rs` and 1 in the adapter (`:829`), 29 in all, plus any the compiler finds in `tests/` or the fakes.
   - *Embedding:* the plan now says `include_str!` in `src/adapters/web.rs`. `scripts/bundle.sh` copies binaries only, so it needs no change. `Cargo.toml` has no `include` list, and the Linux CI build needs nothing outside the crate.
   - *Spike in the same commit:* I kept it, as the task asks for one commit. The gate in point 8 is what keeps the order honest.
