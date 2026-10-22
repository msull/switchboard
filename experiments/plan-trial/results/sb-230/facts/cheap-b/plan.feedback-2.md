## Replies to round 1

1. Key confirmation: settled. The tap and the "automatic comparison protects" claims are gone, and the plan now says nothing authenticates the host on the first visit.
2. Fragment overstated: settled. Departure 1 now says the fragment keeps the code out of the request line, logs and caches, and does not stop the on-path attack.
3. `__Host-` and ports: settled. The plan cites RFC 6265 §8.5, states the cost, and puts it to the owner with the subprotocol as the fallback.
4. Origin fails open: settled. A missing `Origin` is 403, and a test covers it.
5. Camera and a bad certificate: settled. The plan adds a table row, and the URL appears as selectable text as the fallback.
6. `wss:` width: settled. The CSP is built per response from a validated `Host`.
7. Line references: settled. The ranges I checked match, and the plan's corrections (`remote_pair` 3606–3672, `authenticate` 3128–3129, `docs/design.md` 3143–3148) are in. I did not re-verify every number.
8. Static path and the limiter: the order is now stated, but the window it added is a cut (see the second list below).
9. Swift edits: settled. They are declared in the "Swift edits" section.

## Cuts

**Where this departs from the issue**
- "Checked and not departed from" (last paragraph): drop it. It restates a settled fact and two things the plan does not change. The implementer has nothing to act on.
- Item 7 (IPv6): drop it. The Pairing window bullet already says "IPv6 bracketed", and the `invite_url` unit test forces it.
- Items 6 and 8: fold into the UI and Port sections that repeat them.

**Record**
- Collapse the three "stays N" lines and the `devices.json` paragraph into one sentence: "No persisted record or schema changes; a browser pairing writes an ordinary `PairedDevice`."

**Signals**
- Drop the section. It says "No new signal" and re-lists spec rules (epoch refetch, revision ordering) that the page behaviour list in the UI section repeats.

**Core: nothing new**
- Drop it. The heading and one line say what the Commit section already says (contract additions: none).

**Port**
- **Rate window**: cut `STATIC_PER_MINUTE`, the `Limits::limited` change, the "static window" test and the §4.1 amendment. The plan itself says the window "does not remove" the flood exposure. The pool of four is the bound, `Connection: close` frees each slot at once, and the facts file's default for Q6 was no window. This is a second mechanism where the first suffices.
- **Pools**: cut the `MAX_UNAUTHENTICATED = 8` fallback and the "pool turnover" test's "fifth held open" assertion. The facts file lists browser parallelism as a measurement to take on a phone, not something to build for.

**Pairing: the trust step**
- **`GET /cert.der`**: keep one sentence in the owner decision. Cut it from the "Spec and design amendments" §4.3 row, from the `remote_page.rs` mention and from the pre-pairing paths. It is not built by default.
- **SAN reissue**: cut it to one line in the table ("if the phone refuses the click-through, reissue the certificate from the Keychain key with a SAN"). The paragraph describes a new function for a case the facts file lists as unresolved.
- **Subprotocol alternative**: it is written twice (the Reply section and the table). Keep it in the table only.
- **LAN switch**: cut the separate page-only switch with the `IDENTITY_SCHEMA_VERSION` step from the table. The owner said nothing in its favour, and it is a schema change for a case nobody asked for.

**The page**
- Step 1 repeats Decision 1 step 1 (`replaceState`), and step 2 repeats steps 1–3. Keep them in one place.
- "Never show a token, a code or a key beyond the short hash" and the closing paragraph ("no framework, no build step") restate the Files bullet. Cut both.

**Spec and design amendments**
- Drop the `facts.md` bullet. It is the planner's own working file, and the implementer does not need an edit to it.
- §11 "browser cases" and §13 "a row for…" say nothing an implementer can do. Fold both into one line, "document the browser path in §4.1, §4.3, §5, §11 and §13".

**Tests**
- Cut "Native pairing unchanged": the existing test is listed as passing unchanged, and running the suite forces that.
- Cut "Answered without the frame" as a separate test. Make it an assertion in the headers test.
- Cut "Other paths stay refused" down to one line. Two existing tests already cover it, and the plan itself says they are not changed.
- Cut "Revoked and rotated" to the 401 and the `/info` key. The page's message rule is manual, and the tree already tests revocation.

**Unresolved table**
- Drop the rows that carry the default as their only content and no real fallback: "Scope for the browser pairing" (Q9), "Device name from the user agent", "Multiple addresses", "What the page shows with no code and no token" (Q7), and "Revoked and rotated messages" (Q4, the plan already builds it).
- Merge "Storage eviction" and "History, autocomplete or tab restore" into one "run items" row. Their fallbacks are the same: re-pair from the Mac.
