# Response 2 to the feedback on plan.md (issue #230)

Edited: `review/1/repo/plan.md` only. The earlier plan and `facts.md` are untouched. Every edit below was re-read in the file before this response was written; the cut sections were confirmed absent by search.

## Replies to round 1

Points 1 to 16 are confirmed as you describe them. No change is needed beyond what round 1 made. Point 15 stays with the facts owner, as before.

## Cuts

**Where this departs from the issue**
- Cut (done). The bullets for the non-loopback bind, revocation, scope, identity change and the loopback QR are removed. Their reasoning lives in Docs and status (§5), The record (tombstone), Decision 6 and App, UI page states, and UI pairing window. The pool bullet stays.

**The record**
- Cut (done). "Identity and pairing window: unchanged" and "Cookie: no host record" are removed.

**The signals**
- Cut (done). The section is removed. Its one fact, that a tombstone keeps the `RevokeDevice` id so the socket still closes, is now in the Core bullet for `RevokeDevice`.

**Actions and effects, Core**
- Cut (done). `revoked_by_token` is removed from the Core section, the Contract additions and the tests. `device_by_token` returns the revoked row, and `authenticate` checks the flag and answers `401 revoked` before `DeviceSeen`. The core test reads "`device_by_token` returns the revoked row with `revoked` set."

**Actions and effects, Adapter**
- Cut (done). The construction-site count and "the change is mechanical" are gone. The repeated field lists are gone from the Adapter section; it now points to Contract additions, and keeps only the cookie-not-logged rationale.

**App**
- Cut (done). "The `/api/v1/` strip and the allowlist are unchanged" is removed.
- Changed (done). The connection-thread path is cut. `GET /` and `GET /app.js` are one `serve_remote` arm, checked before the `/api/v1/` strip at `:3023`, which otherwise refuses every path outside the prefix. The arm answers in one frame pass, and the `RemoteReply` headers carry `Set-Cookie` and the page's headers. Owner call 4 (connection thread or frame) is removed. The pool question moves to step 0: if a cold load counts more than four, the fallback is `MAX_UNAUTHENTICATED` at 6. See App, Decision 4 and Order inside the commit.

**Decision 1**
- Cut (done). The renewal, the `.local` rename handling and the dialog line for it are no longer in Decision 1. Decision 1 keeps one sentence: if step 0 shows no override, re-issue is new work with its own renewal design (owner call 1), and the current code generates a new key, so the pins would break. The renewal and rename costs are listed once, in owner call 1. The `.local` host in `pairing_link` is also removed, so the URL carries only an IP literal and the re-issue mechanism is not built ahead of step 0.

**Decision 2**
- Cut (done). The subprotocol reason is kept once, at the start of Decision 2; the repeat in the gate is gone.
- Changed (done). The poll is a one-line gate in step 0: "If step 0 refuses the handshake, poll the session view." The session-view detail is not built unless step 0 needs it. Owner call 2 keeps the fallback.
- Changed (done). The `localStorage` second carrier is one gate line, with the subprotocol reason dropped.
- Cut (done). The iOS `Max-Age` bullet is removed. The UI's "Not paired" state covers it.
- Cut (done). The "same-site is unresolved" clause is removed from the `Lax` bullet. The argument now states only what `Lax` does and that the Origin rule does not depend on the site definition.
- Cut (done). The 30-day token-expiry bullet is removed.

**Decision 4**
- Cut (done). The frame alternative is moot, since the frame is now the chosen path.
- Cut (done). The "departs from the settled line" sentence is removed; owner call 6 names it.
- Cut (done). The pool fallback is removed from Decision 4 and moved to step 0 (see App above). Owner call 5 is removed.
- Cut (done). "Other paths unchanged" is removed; the existing 404 test forces it.
- Cut (done). "The page without a code is a state" is removed; the UI's pairing states say it.

**Decision 5 and Decision 6**
- Cut (done). The Decision 5 fallback line is removed; owner call 3 now carries the mechanics (the page reads `location.hash` before `replaceState`, and the Rust builder and Swift parser change to match). The Decision 6 owner line is removed; owner call 5 now carries "the page sends, shows the reply box, and reports `403 scope` inline."

**The page's pairing sequence**
- Cut (done). The sentence "The check detects an address … (facts, Settled)" is removed. The mismatch message stays, and the Where-departs key bullet carries the reason.

**UI**
- Cut (done). "A changed key is not stored" is removed.

**Tests**
- Cut (done). The `/pair` without `carrier` bullet is removed; `a_pairing_inside_the_window…` already checks it.
- Cut (done). The "unchanged" tests bullet is removed.
- Cut (done). The "Fixtures … unchanged" bullet is removed.
- Cut (done). "Not tested in CI" is removed; the phone run is still in Order inside the commit, and owner call 6 is unaffected.
- Cut (done). The commands line is removed; CLAUDE.md states them.

**Docs and status**
- Changed (done). §5 now reads "amends lines 306, 309 and 312; owner call 6." The rest of the §5 text is cut. I kept line 306 as well as 309 and 312, because the pre-authentication rule in §5 is the text the static-path change contradicts; without it the spec amendment is incomplete.
- Cut (done). The spike entry under Docs is removed; step 0 and the commit body say it.
- Changed (done). §4.1 now says static paths are served by the frame, not the connection thread.

**Owner calls**
- Changed (done). Owner calls 1 to 7 now hold the fallbacks. Decision 1 carries one sentence and points to owner call 1; Decision 5 and Decision 6 point to owner calls 3 and 5. The list is renumbered to seven entries.
- Changed (done). Owner calls 8, 10 and 11 are merged into one call, now owner call 6, which names the three places the trade lands: the browser trust model, the static paths before authentication, and the cookie against spec §5. All references are renumbered to match, including the Where-departs key bullet (owner call 6), the §5 docs line (owner call 6) and the Decision 2 gate (owner call 2).
