## Earlier points

1. Paired pool: settled. The two-in-flight queue, the 503 handling and the written fallback cover it.
2. Rate-limit order: settled. The rate check runs first and the static branch follows it.
3. CSP and inline style: settled. The `app.js` bans are in the tests.
4. Click-through promised as fact: settled. The dialog hint, the changelog and the `design.md` gap bullet no longer promise it.
5. Device name: settled. `new_device` drops control characters, with a core test. Leftover: line 9 still says "the host needs no new rule", which now contradicts the next sentence (see the first cut under "Where this departs").
6. Line references: settled. `refuse` is `:3024` and the `serve_remote` arm placement is described correctly.
7. `RemoteRequest` fields: settled.
8. `Info.swift` line: settled (`:114`, doc comment `:113`).

## Cuts

**Where this departs from the issue**
- Line 9: delete "So the page sends its own name ... and the host needs no new rule." The following sentences make the host rule real, so the sentence is wrong. Keep the cap citation and the control-character filter.
- Line 10 (`addresses` holds zero or one entry): delete. The only decision it supports is the single `host:port` in the URL, and Decision 3 already states that.
- Line 11 (code never stays in the address bar): delete. "Decision on the code in the URL" repeats it in full.
- Line 12 (CSP and the WebSocket): delete. The CSP bullet under "The signals" says it.
- Line 7: delete the whole "if the answer is yes, the implementer instead adds `last_message` ... fixtures ... Swift decoder" clause. The plan has chosen `card` and `waiting_reason`. A fallback written for a case the plan does not build is a second mechanism, and the facts already describe it (line 14).

**The record**
- Line 16: delete the schema-version list and "If a parallel ticket bumps one...". Only "No record changes, no schema bump" is needed. The test suite and `migrate` would force a bump anyway.
- Decision 2, last bullet (the cookie build list): delete. The decision is made, and the facts describe the cookie design.
- Decision 1, third bullet (the SAN and `/cert.der` fallback): cut to one sentence. The `design.md` bullet already names it.

**The signals**
- Line 40 (headers): keep the header list and drop the parenthetical reasons. The unit test lists the same headers.
- Line 42 (Decision 4): delete the "separate pool of 4" fallback and its wording. The `page` window plus `Connection: close` suffice, and the acceptance run is the stated check.
- Line 43 (paired pool): cut the "Two tabs are then at most six of the twelve..." and the "raise `MAX_PAIRED` and the 16-connection total" fallback sentences. The queue and the 503 rule stay.
- Line 47 (revoked device): delete. It repeats the "Live" bullet under "The UI" ("a close reads `/me` first").

**The actions and effects**
- Line 51: delete the parenthetical about IPv6 `Display` and the Crockford, hex and UUID remark. The IPv6 unit test forces the first, and the second is not needed to implement.
- Line 53: delete the sentence about `build.rs`, `include` and `scripts/bundle.sh`. It is a Settled line (facts 16). Also delete "a struct literal added later outside the crate...".
- Line 55: delete the sentences about the memberwise `init` and the three callers. The existing Swift tests force them.
- Decision 3, the "Copy as JSON" fallback: delete. The plan's own Docs section says `parse` still reads JSON.

**The UI**
- Line 60: delete the clause about the "existing dialog test still finds its button" and the `QR_SCALE` character counts. Keep "uses `127.0.0.1` with no LAN address".
- Line 62: delete the `module.exports` mechanism description. The Node test forces it.
- Line 77 (endpoint table "so the test can find them"): shorten to one clause.

**The tests**
- Line 82: delete the adapter-unit items that restate line 40 headers and the `HEAD`/405 behaviour, since they are listed twice (here and under "The port, end to end").
- Line 89 (the `me` fixture): delete. It says there is no fixture, so there is nothing for an implementer to do.
- Line 91: delete the "not in the pre-commit hook or CI" sentence. It is not needed to implement.
- Line 93 ("Not claimed"): shorten to the first two sentences. The enumerated Unresolved list is already in the `design.md` bullet.

**Docs, README, changelog**
- Line 97: collapse the §4.1, §4.3, §5 and §11 lists to "`docs/remote-client.md` §4.1, §4.3, §5 and §11 are updated for the page, `/me`, the subprotocol and the URL invite". The sections' content follows from the code changes.
- Line 99: delete the `client/README.md` line numbers and the `main.swift:12` reference. Name the files only.

**Commit**
- The contract-additions list repeats the sections above. Keep it, since CLAUDE.md asks for it, but drop the closing "No change to `model.rs`..." line. Line 16 already says it, and `new_device` is now a change to `src/core/remote.rs` that the line does not mention.
