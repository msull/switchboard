# Response to the second review of the issue #230 plan

The revised plan is `/Users/sbgragent/Library/Application Support/Dispatch/tickets/b49c34f1/review/1/repo/plan.md`. The earlier plan under `plan/1` is unchanged. Each edit named below was checked in the revised file before this response was written.

## Replies to round 1

1. **Key confirmation**: no change. The round-1 edits stand: Where this departs, item 5, and Pairing: the trust step.
2. **Fragment**: no change. Where this departs, item 1.
3. **`__Host-` and ports**: no change. Pairing: the URL, the cookie and the Origin check, "What the cookie does not isolate", and the Unresolved table.
4. **Origin**: no change. Pairing: the Origin check.
5. **Camera**: no change. Unresolved table, and UI → Pairing window.
6. **`wss:` width**: no change. Port → Headers.
7. **Line references**: no change.
8. **Static path and the limiter**: the order stays, and the window is cut (see the cuts below).
9. **Swift edits**: no change. The Swift edits section stands.

## Cuts

**Where this departs from the issue**
- "Checked and not departed from" is cut. The section has no paragraph under it now.
- Item 7 (IPv6) is cut. The Pairing window bullet keeps the bracketing, and the `invite_url` unit test in Tests still forces it.
- Items 6 and 8 are cut as departures. Item 6's content is in UI → Pairing window (the URL is selectable text). Item 8's content is in Port → Order in `connection`.
- The list is now items 1–5.

**Record**
- The three "stays N" lines and the `devices.json` paragraph are one paragraph. It opens "No persisted record or schema changes; a browser pairing writes an ordinary `PairedDevice`", with the three version numbers after it, and the page-state and token bullets follow.

**Signals**
- The section is cut. Its epoch and revision rules are in the page's Live step.

**Core: nothing new**
- The section is cut. The Commit section still says contract additions are none.

**Port**
- **Rate window**: cut. `STATIC_PER_MINUTE`, the `Limits::limited` change, the static-window test and the §4.1 amendment are gone from the plan. The Pools bullet says no window is built. The plan does not name `STATIC_PER_MINUTE` anywhere now. The cut means a flood of `/` can churn the four slots and starve `/info` and `/pair` for as long as the flood lasts, which is the exposure `/info` and `/pair` already have today. The Q6 row says so.
- **Pools**: the `MAX_UNAUTHENTICATED = 8` fallback is cut. The "pool turnover" test keeps only "four static requests in sequence, each closed by the host, leave the pool free for a fifth". The browser-parallelism row now has no fallback: the phone measurement decides, and a result that four is too few goes back to the owner.

**Pairing: the trust step**
- **`GET /cert.der`**: kept as one sentence in the owner decision ("The one mechanism that authenticates the host ... needs a new route"). It is cut from the §4.3 row, from the `remote_page.rs` mention and from the pre-pairing paths. The Trust-step table row keeps it as its fallback, in one line.
- **SAN reissue**: the paragraph is cut. The table row carries the one-line fallback: "If the phone refuses the click-through, reissue the certificate from the Keychain key with a SAN (new code, same SPKI)."
- **Subprotocol alternative**: the bullet in the Reply section is cut. It is kept in the Unresolved table only, in two rows that point at the same fallback.
- **LAN switch**: the separate page-only switch and its `IDENTITY_SCHEMA_VERSION` step are cut from the table. The LAN section keeps the one-switch default (owner Q8 is still named there).

**The page**
- Step 1 and step 2 are one place now. Start reads and clears the fragment, then runs Decision 1 steps 1–3. Pairing says only what a 200 stores.
- "Never show a token, a code or a key beyond the short hash" is cut, and so is the closing paragraph ("no framework, no build step").

**Spec and design amendments**
- The `facts.md` bullet is cut. Departure 1 no longer says the implementer edits `facts.md`; it cites the RFC line as its own source.
- §11 and §13 are one line: "Document the browser path in §4.1, §4.3, §5, §11 and §13."

**Tests**
- "Native pairing unchanged" is cut. The existing test is not named in the plan.
- "Answered without the frame" is folded into the headers test as an assertion: the test does not call `serve_pending`, so the 200 shows the static path is answered on the connection thread.
- "Other paths stay refused" is one line: the two existing tests cover it, and the plan does not change them.
- "Revoked and rotated" is cut to the 401 and the `/info` key. The page's message rule is not in the tests.

**Unresolved table**
- Dropped as having no fallback: "Scope for the browser pairing" (Q9), "Device name from the user agent", "Multiple addresses", "What the page shows with no code and no token" (Q7), and "Revoked and rotated messages" (Q4). The plan still builds the behaviour for Q4 and Q7, and Q9 is the window's existing scope choice.
- Merged: "Storage eviction on iOS" and "History, autocomplete or tab restore" are one "Run items on storage and history" row, with one fallback: re-pair from the Mac's window.
- Also cut, not in the review's list: the "LAN switch for the page" row, because its only content was the separate switch the review asked to cut.
- Kept, with its fallback changed: "Pool for static paths" (Q6) now reads "no window is built" as its fallback.

## What the cuts leave open

The static-path flood exposure is now the only unbounded one. It is the same as `/info` and `/pair` today, and the owner's Q6 row says so. Cutting the window means the owner must accept that exposure or send the plan back.
