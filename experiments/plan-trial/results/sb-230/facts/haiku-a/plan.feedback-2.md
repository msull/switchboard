# Feedback 2 on plan.md (issue #230)

## Replies to round 1

1. Settled: the page no longer says "revoked" on a 401; three sentences keyed on the stored key.
2. Settled in the plan. The facts line at `facts.md:45` still says the states look the same; it is wrong and should be corrected in the facts file.
3. Settled: the pool is 8, the page load is bounded by Chrome's six per host, and `/pair` is bounded by its rate window and failure counter (`src/adapters/remote.rs:583`, `src/core/action.rs:973`). Safari and Firefox remain unmeasured, and Spike 23 covers them.
4. Settled: `respond` builds the head from two defaults that a caller's header replaces.
5. Settled: the fallback is written in full, with the reply, the subprotocol parse and the contract lines. Its cut is in the list below.
6. Settled: a hard-fail branch and the reason for no SAN are written.
7. Settled: `spikes/23-browser-session` is step 0, before the cookie branch.
8. Settled: the citation is CLAUDE.md, and the no-bump reasoning is stated.
9. Settled: the commit message body lists the additions.
10. Settled: the literal sites are listed and the counts are 29 and 1, which I confirmed by grep.
11. Settled: the reissue is gated on `read_only()`, with a test.
12. Not settled. The reply says `docs/remote-client.md:207` does not contain the `last_message` / `changed {session}` sentence. It does: line 207 reads "The compact card's one-line excerpt is not part of it: that is `last_message` from the `Stop` hook on the record, … and moves with `changed {session}`." The §4.3 amendment must cover line 207's sentence as well as line 201. Line 201 is right.
13. Settled: the page reconcile cites the envelope revision and §4.4.
14. Settled: the address-bar code bullet and its residual are written.
15. Settled: the Origin check compares the parsed scheme and authority, and `Origin: null` is refused.
16. Settled: the per-start device cost is stated in Known costs.
17. Settled: `read_request` extracts the token and keeps no raw `Cookie`. The manual `Debug` is cut below.
18. Settled: the citations are corrected.

## Cuts

**Where this departs from the issue**
- Bullet 9 (page vs revoked) repeats the UI section; cut it there.
- Bullet 10 (cookie on `/pair`) restates Actions and effects; cut.
- Bullet 12 (CLAUDE.md procedure) and bullet 13 (spike) are process narration; cut, since Steps 0 and 3 carry the work.
- Bullet 15 (pool 4 to 8) duplicates "Pool"; cut.
- Bullet 14 (browser questions) repeats Trust decision 1 and Startup certificate; cut.
- The "Confirmed as the issue says" paragraph is a verification log; cut.

**Record**
- The CLAUDE.md and `load_identity` reasoning in the Identity paragraph is rationale; keep "stays 1, `#[serde(default)]`" and cut the rest.
- "Nothing else changes on disk" restates the Record; cut.

**Signals**
- The list of event kinds restates `docs/remote-client.md` §4.4; cut to "None new; the page uses the events socket as §4.4 defines it."

**Actions and effects**
- "Pool": the paragraph explaining why the four-slot pool "was never the pairing bound" is history; keep the number, the reason for 8 (six per host plus two), and the test.
- "Static routes": "Static answers never reach the frame, so the 4 ms budget is untouched" and the logging sentence restate the Browser protocol rules; cut to the first clause of the response list.
- "Reply writer": "`respond` writes the list in order, so several `Set-Cookie` lines are possible" is machinery for a case the plan never uses; cut.
- "Request": the manual `Debug` impl and its unit test are machinery for a case the tree rules out: nothing formats `RemoteRequest`, and `refuse` logs method and path only (`src/app.rs:3762-3766`); cut both. Keep the rule that the raw Cookie header is never kept.
- "Pairing": "The SPKI check and the code check are unchanged" restates the code; cut.
- "Authentication": the Origin rationale paragraph repeats Browser protocol; cut to the rule and the list of refused forms. The "if the phone run shows another form" sentence repeats the gate; cut.
- "Startup certificate": the parenthetical on native verification and the clock sentence are rationale; cut.
- Trust decision 1: the "Reasons" and "What this buys / does not buy" paragraph is rationale for the owner; cut to the decision and the pre-pairing list. "Why no SAN" is rationale; cut to "no SAN, by decision."
- Trust decision 2 fallback: this is a second authentication transport (bearer in `fetch`, bearer in the subprotocol, a second pairing reply shape, a second `read_request` parse, a second contract line, a second commit line). It doubles the auth paths before the spike has run. Cut the fallback to one sentence: "If Spike 23 shows the cookie is not kept, the plan is reopened." Remove the fallback wire line in Contract additions, the subprotocol line, and the commit body's matching text.
- Client-side logic: the sentences on CLAUDE.md's rule and on the projection fallback restate the owner item; cut both. Keep the revision rule and the §4.4 citation.

**UI**
- "The address-bar code" bullet: the residual analysis repeats Known costs and the Gate; keep the `replaceState` call and cut the bullet.
- "No code and no token": the paragraph after the list restates the sentences; cut to the owner note.
- "Device name": the known-limit clause repeats Owner items; cut to one clause.
- "Pairing states and copy" section says nothing beyond the UI section; cut the section.

**Tests**
- `the_unauthenticated_pool_refuses_the_ninth_connection`: an existing test that the plan says needs no assertion; cut.
- "Not automated" paragraph: keep the first sentence; cut the rest, which the Gate restates.
- Checks: the sentence that the adapter code has no macOS path is a restatement; cut. Keep the Linux arm command.
- The `a_read_only_instance_does_not_reissue` test and the other tests stay.

**Steps**
- Step 1 and Step 2 list every literal site and count. The compiler lists every literal site when a field is added, so the list is forced by the test suite; cut to "fix every struct literal the compiler reports." Keep the Identity and PairedDevice field names and the `new_device` parameter.
- Step 3: "Built after step 0's result is recorded, and the branch built is the one the spike chose" becomes "after step 0 is recorded"; the fallback clause goes with the cut above.

**Docs and status**
- `docs/remote-client.md`: the §4.1 bullets duplicate "Pool"; keep the bound amendment and cut the restatement. Keep §4.3's line 201 and line 207 amendments. Cut "§11: the new test names" (the names are in Tests) and "§11, lines 479 and 502: no change" (a no-op).
- `docs/design.md`: the conditional "if the trust-boundary paragraph lists the pre-pairing paths" is machinery for a case the facts rule out. I searched `docs/design.md` lines 219–300 for pairing and found only the secrets paragraph's remote-port sentence (`docs/design.md:251`), so cut the conditional.

**Owner items**
- Items 1–5, 7 and 12 duplicate the owner bullets inline; the list is an index with no new content. Cut the list to items 6, 8, 9, 10 and 13, which have no inline home, or cut the list entirely if the inline bullets are kept.
- The Acceptance paragraph duplicates the Gate; cut it to "the owner's phone run, Spike 23 (Gate)."

**Known costs**
- The `localStorage` sentence repeats the UI section; cut.

**Contract additions and commit message**
- The commit message body and the Contract additions list say the same thing twice. Keep Contract additions and say the commit body copies it.
- The "Internal, not contract" line lists names the Record and Steps already name; cut.
- The attribution paragraph restates CLAUDE.md; cut.
