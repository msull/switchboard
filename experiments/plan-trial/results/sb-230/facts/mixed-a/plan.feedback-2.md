## Earlier points

1. Settled: a cookie `GET` without `Origin` no longer runs `DeviceSeen`, the effects on the pool and the inbox are written down as accepted, and a `last_seen` test covers both.
2. Settled: the "Where the cookie goes" bullet, the amendment to §5 line 312 and the dialog sentence now cover cookies sent to other servers on the address and overwritten by them.
3. Settled: the 400 days is now marked as a choice, `GET /device` re-sends the cookie so its lifetime slides, the 401 wording names a dropped pairing, and the stale device row is noted.
4. Settled: the field is now the string `credential: "cookie"`, read with the existing `field()` helper, and `"token"` is reserved for the fallback.
5. Settled: step 3 says what happens without `sb.host`.
6. Settled: the commit body lists the contract additions and has no trailer.

## Cuts

1. **Where this departs from the issue, item 8.** A recount of a number in the investigation notes is no instruction for the implementer, and "Actions and effects" already lists the ten literals. Cut the item.
2. **The record, first paragraph, from "If an owner answer flips a branch" to the end.** It describes the schema bump for a `Listen.web` field the plan does not build. Cut it, and cut the matching last sentence of Decision 4 ("the branch of a separate web switch is a `web: bool` on `Listen`…").
3. **Unbuilt branches written out in detail.** Departure 1 (the `excerpt` field, its event and fixtures), departure 3 (a `revoked.json` written through `RemoteFiles::save`) and decision 1 (option B's `make_identity` work) are designs for branches this commit does not build. One clause naming each branch is enough; the detail belongs in the owner's follow-up.
4. **Protocol step 3's identity check, the `sb.host` comparison and its "lost after a week idle" notes in step 3, "The record" and the §5 amendment.** The check cannot keep the token from a wrong host, because `Path=/api/v1` covers `/api/v1/info` and the cookie travels on that `/info` request. A different Switchboard at the address ignores a cookie not named for its own `server_id`, so step 4's 401 already reports the case. With the check cut, `sb.host` shrinks to a flag that only picks the 401 wording.
5. **Protocol step 2's comparison of the link with `/info`.** The host already refuses a `spki_sha256` that is not its own (`wrong_key`, `src/app.rs:3632-3639`), so this is a second mechanism doing the same job. The page should map `403 wrong_key` to the message "this is not the Mac whose code was shown" and drop the `GET /info` before `/pair`.
6. **Protocol step 7's second trigger, "or `connect-src` reports a violation".** A socket blocked by the CSP never sends a `hello` either, so the five-second `hello` timeout already covers it.
7. **The tests: the unit tests for `web::asset` and `respond_with`.** The page test in `tests/remote.rs` already checks the three paths, the 404s and every header over the wire. Keep the unit tests for `cookie_value` and `same_origin`.
8. **The tests: the `device` fixture and `client/Fixtures/device.json`.** The plan says no Swift model reads it yet, so nothing consumes it. Cut it.
9. **The tests: the remarks that existing tests are unchanged and still hold** (that the wrappers mean "no test changes", that `:426` "is unchanged and still holds", and that the pairing tests at `:477, :582, :635` "are unchanged"). The suite enforces all three. Cut the remarks.
10. **Order inside the commit, the closing "Then `cargo test` … `cargo fmt --all`".** The pre-commit hook runs these.
11. **Docs, the last two sentences (the attribution trailer and naming no client).** The first repeats "Commit message", and the second repeats CLAUDE.md. Keep the trailer rule in "Commit message" only.
12. **The new route, "Not added to `info.capabilities`…".** It is a decision not to do something nobody asked for. Cut it.
