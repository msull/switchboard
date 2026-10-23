Earlier points:

1. Click-through as a stop condition: closed. Decision 1 and the spike paragraph now stop the build first on it, then on the cookie.
2. Fragment instead of query: closed. Decision 3, `invite_url` and `app.js` use `#code=...` and `location.hash`.
3. Spec line 312: closed. A "Where this departs" bullet quotes it, and "Docs and order" amends it in place.
4. Rebind trigger: closed. A rebind happens only on an address change seen at `OpenPairing`, and the cost is stated.

Cuts:

- **Where this departs.** The "one-line excerpt" bullet restates `control/src/lib.rs:637-690` and ends "no new field". Decision 4 and the UI section already say what the page shows, so drop the bullet. Drop the "Compare the key in the URL" bullet too: decision 1's dialog sentence and "Errors in the page's words" both carry it. The "code in the query" bullet repeats decision 3's reason, so keep one of them.
- **Decision 1.** The vendor-text recap (the "Visit this website" / DNS-name / one-vendor sentence) is evidence for the planner, not instructions for the implementer. The spike's stop condition is the instruction. Cut the recap.
- **Decision 2.** "Cookies are not scoped by port" and the "dialog's sentence covers it" line add nothing the implementer builds. Cut them. "A subprotocol or a first-message token is not built" repeats the follow-ups section.
- **Decision 3.** "The code, key and id are Crockford, hex and a UUID: nothing to percent-encode" is a test the core test already forces. "The issue's `?` form would have worked since `read_request` splits the query" is history. Both can go.
- **Decision 5 and Decision 6.** These two are one-liners that say nothing the sections below don't. "Tests in the last section" and "no field is added" can be dropped. The `PairedDevice` unchanged bullet in "The record" already covers 6.
- **The record.** The schema paragraph lists three constants and then "had one been needed ...". CLAUDE.md already makes the rule. Keep one line: "no record changes, no schema bump".
- **The record, Identity bullet.** It restates the Certificate bullet's "in memory only". Merge.
- **The record, pools.** "Unresolved: how many connections a browser opens ..." and the separate 10 s idle for the page pool are machinery for an unresolved case. The page pool (6) plus the 120 per minute rate window is enough to protect `/info` and `/pair`. A 10 s idle is a second knob. Build the pool and the rate, and leave idle at 30 s.
- **The signals, socket enhancement.** This is an enhancement built in front of the default. The facts say Safari's `wss` over a clicked-through certificate does not work. Chrome and Firefox are unverified or secondhand. The poll is "the whole feature". The `hello` slow-down to 30 s, the debounce, the backoff, the `watch`/`unwatch` frames, and the second code path that applies them are roughly half the page's JS for a path the facts call unproven. Cut it from this commit and list it as a follow-up once spike 23 shows a socket opening. That also removes the cookie-on-upgrade `Origin` test and the upgrade branch of the `Authenticate` bullet, and "wss in `connect-src 'self'`" from decision 4.
- **Authenticate bullet.** With the socket cut, the `Origin` check on non-`GET` remains. The "stops a cross-site WebSocket hijack" sentence goes.
- **Request bullet.** "The 8 KiB head is checked with a real browser's request in the tests" repeats the adapter test.
- **Reply bullet.** "30 literal sites (`grep ...`)" and "The reason table gains nothing" are mechanical. The compiler forces the sites.
- **`/pair` bullet.** The 64-character `device_name` cut is a new rule nobody asked for. Drop it, or move it to the test list.
- **The UI.** The page-file list (`index.html`, `app.css`) restates the decisions. The `<meta name="referrer">` duplicates the header in decision 4. Keep one.
- **The tests.** The "page references only served paths" integration test is machinery for a typo the first manual run finds. The `Copy invite` dispatches-nothing test and "the existing tests pass untouched" are forced by the suite anyway. The Swift test list repeats the contract section. The acceptance paragraph repeats the follow-ups.
- **Docs and order.** The "Order" sentence is a restatement of the plan's own sections.
- **Commit message and Contract additions.** The contract list repeats the bullets above it. Keep it only as the commit body, and drop it as a section.
