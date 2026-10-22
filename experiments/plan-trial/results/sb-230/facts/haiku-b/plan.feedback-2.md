# Feedback 2 on plan.md (issue #230)

## Replies to round 1

1. Settled. Decision 1 (plan lines 56–59) now cites `:101`, `:112` and `:95-127`, and writes the three costs.
2. Settled. Decision 2 (lines 63–65) cites WHATWG §2.2 step 2, makes the poll the fallback, and adds the gate in step 0.
3. Settled. The trade is at line 11 and in owner call 8; lines 139–140 amend `remote-client.md:309` and `design.md:3111-3112`.
4. Settled. Order step 0 is `spikes/23-browser-pairing/`, run on a phone before any other step.
5. Settled. Lines 123 and 127 name each assertion and its new value.
6. Settled. The "nothing else changes" sentence is gone; line 64 states the poll as a change to the session view.
7. Settled. Lines 101–102 handle `local_address()` returning `None`.
8. Settled. Line 69 argues rebinding from cookie scope, not from SameSite.
9. Settled. Line 70 covers cross-site top-level GETs and lists the two side-effect GETs.
10. Settled. The commit body copies the Contract additions list (lines 178, 191).
11. Settled. Owner call 9 names the downgrade and the versioned-wrapper fallback.
12. Settled. Line 95 now says "This address answers with a different key."
13. Settled. Line 81 names both `'self'` and `wss:`, with a reason for each.
14. Settled. Lines 82 and 169–172 are the owner calls that cover the contradictions.
15. Settled for the plan. Its own citations are corrected; `facts.md:8` and `:10` still carry the old lines, and that is for the facts owner, outside plan.md.
16. Settled. Line 92 states the fallback as a consequence.

## Cuts

**Where this departs from the issue**
- Items 12, 13, 14, 15 and 17 each restate a decision made later (Docs §5; The record and App `authenticate`; Decision 6 and App `serve_remote`; UI page states; UI pairing window). Cut them here and keep the reasoning once, in the decision.

**The record**
- Line 24, "Identity and pairing window: unchanged": nothing to say. Cut.
- Line 25, "Cookie: no host record": restates Decision 2. Cut.

**The signals**
- The whole section says "None new". Cut the section. Keep its one fact, that a tombstone keeps the `RevokeDevice` id so the socket close still happens, in the Core bullet for `RevokeDevice`.

**Actions and effects, Core**
- `revoked_by_token` (line 40, and Contract line 185) is a second lookup where the first suffices. `device_by_token` can return the row with `revoked` set, and `authenticate` checks the flag and answers `401 revoked`. Cut the function. The core test then reads "`device_by_token` returns the revoked row with `revoked` set".

**Actions and effects, Adapter**
- Line 46, "About thirty construction sites … mechanical": the compiler forces every `RemoteReply` literal to change. Cut the count and the sentence.
- Line 45 and line 46 repeat the field lists in Contract additions (lines 182–183). The Contract list is needed for the commit body; cut the repeats here.

**App**
- Line 51, "The `/api/v1/` strip and the allowlist are unchanged": nothing to say. Cut.
- Line 48 puts the static answers on the connection thread. That is a second route for the same two paths, and the frame already carries `RemoteReply` headers for `Set-Cookie` (line 53). Make `GET /` and `GET /app.js` one `serve_remote` arm that answers in one frame pass, and cut the connection-thread path. This also removes owner call 4 and the pool question in owner call 5 from the build; step 0 measures the pool.

**Decision 1**
- The re-issue fallback's renewal at 60 days (line 57), the `.local` rename handling (line 58) and the dialog line for it are a second certificate mechanism built before step 0 says Safari needs it. Cut them, and keep one sentence: if step 0 shows no override, re-issue is new work with its own renewal design.

**Decision 2**
- Line 62 (subprotocol is out) and line 65 (the same reason again) say the same thing. Keep one.
- Line 64, the poll: a second transport built only if step 0 refuses the WebSocket. Cut it to one line in the gate: "if step 0 refuses the handshake, poll the session view."
- Line 65's `localStorage` second carrier: built only if step 0 shows no cookie. Keep it as one line in the gate, and drop the restated subprotocol reason.
- Line 66, the iOS `Max-Age` question: "Nothing is built for that." Nothing to say. Cut; the UI's "Not paired" state already covers it.
- Line 67, "Whether two pages on one IP-address host are same-site is unresolved": the choice of `Lax` does not depend on it. Cut the clause.
- Line 72, the 30-day cookie against token expiry: spec §5 says the token does not expire; nothing for the implementer. Cut.

**Decision 4**
- Line 78, the frame alternative: moot after the App cut above. Cut.
- Line 82, "This departs from the facts' settled line…": owner call 10 says it. Cut here.
- Line 85, the pool fallback: owner call 5 says it. Cut here.
- Line 86, "other paths … unchanged": the existing 404 test forces it. Cut.
- Line 87, "the page without a code … is a state": the UI's pairing states say it. Cut.

**Decision 5 and Decision 6**
- Line 90 (Decision 5) and line 93 (Decision 6) give fallbacks that owner calls 3 and 7 already give. Cut both lines here.

**The page's pairing sequence**
- The last sentence, "The check detects an address … (facts, Settled)", restates Where-departs item 11. Cut.

**UI**
- Line 112, "A changed key is not stored": restates Where-departs item 15, which is itself cut above. Cut.

**Tests**
- Line 122, `/pair` without `carrier`: `a_pairing_inside_the_window…` already checks it. Cut.
- Line 124, "unchanged" tests: nothing to implement. Cut the bullet.
- Line 125, "Fixtures … unchanged": nothing to implement, and the decode check runs regardless. Cut the bullet.
- Line 129, "Not tested in CI": restates Docs and the owner calls. Cut.
- Line 130, the commands: CLAUDE.md states them. Cut.

**Docs and status**
- Line 139, the amendment to §5: it restates owner calls 8 and 11 and the text of lines 309 and 312. Keep "amends §5 lines 309 and 312; owner calls 8 and 11", and cut the rest.
- Line 143, the spike entry: step 0 and the commit body already say it. Cut.

**Owner calls**
- Owner calls 1–7 repeat fallbacks the decisions already gave. After the cuts above, the owner list is the one place for them; the decisions keep the rationale only.
- Owner calls 8, 10 and 11 each ask "accept, or drop the browser path." Merge them into one call that names the three places the trade lands (trust, the static path before authentication, the cookie against spec §5).
