# Response to the second review of the #230 plan

Edits are in `/Users/sbgragent/Library/Application Support/Dispatch/tickets/f53f3e3e/review/1/repo/plan.md`. The earlier plan is untouched. I re-read the file after writing it and confirmed each cut is gone and each kept section is present.

## Replies to round 1

Noted; no change needed. The round-1 answers stand as written.

## Cuts

**Where this departs from the issue**
- Cut the "certificate trust is unproven" bullet. The Certificate trust section and the spike carry it.
- Cut the "code never stays in the history" bullet. Its open part, whether iOS drops the first request's history entry, now sits in the spike's phone half under "Spike first".

**The record**
- Cut the static-files and pairing-window bullets. Cut the schema-version bullet as a bullet, but kept its one line, since the brief asks for the versions by number; it now stands below the first bullet and is not repeated under Contract additions.
- Kept the first bullet and the three `localStorage` keys.

**Signals**
- Replaced the section with one line, "No new signal", which is what the brief asks for when a section has nothing to say.

**Routes and Transport**
- Cut the `/info` bullet under Routes and the path-check bullet under Transport.
- Cut the `data:,` favicon link and its spike item. `page_refuses_other_static_paths` still checks that `/favicon.ico` gets today's 404.
- Cut the `Cache-Control` rationale and the explanation of `include_str!`.

**Certificate trust**
- Cut the download-route "why" bullet. Its substance stays in the fallback paragraph.
- Cut the reissue-with-SAN sentence.

**Token transport**
- Cut the second and third "why" bullets. The first bullet stays and carries the reason. Cutting it means the no-cookie decision rests on nothing stated in the plan; the cookie argument is in facts "For the owner" 2.
- Cut the `sessionStorage` fallback. What it would have answered is now a failure of storage, which the wrapped-storage sentence in The page covers.

**Pool and rate window**
- Cut the "Own pool or shared" bullet. Its decision is now one clause in the Pool bullet: page files share the unauthenticated pool.
- Cut the Logging bullet. Routes says the query is never logged.

**Pairing URL**
- Cut the "Two payloads" paragraph. The camera fallback keeps the JSON QR.
- Cut the measurement paragraph.
- Merged the native-path bullets into one: all of them call `PairingInvite.parse`.
- Kept the "one URL per address" fallback. It answers facts "For the owner" 9, a For-the-owner decision, and the brief requires a named fallback for those. Cutting it would leave the decision with no fallback.

**Fallbacks beside Settled lines**
- Kept the stored-scope fallback (departures) and the control-only fallback (UI). They are not beside Settled lines. They answer facts "For the owner" 7 and 8, so the brief requires each to name its fallback.

**The page**
- Cut flow item 3 to one sentence.
- Cut the user-agent list to `iPhone` or `iPad` for the device, `Safari` or `Chrome` for the browser, else `Browser`.

**Tests**
- Cut `page_references_only_served_paths`. The cost: a renamed route in `app.js` surfaces only in the phone walk.
- Cut "eight in sequence" from the slot test.
- Cut both "existing tests unchanged" bullets, including the `stalled_strangers` one. The `cargo test` gate still runs them.
- Cut the dialog test in `tests/ui.rs`.
- Cut the Fixtures paragraph and the no-JS-test sentence.
- Cut the Gates paragraph.

**Spec, README, status, changelog**
- Cut the detail: the section now lists what each file touches.
- Cut the "The app builds and runs after each step" lead-in. That rule is in CLAUDE.md. Kept "One commit."
- Kept the commit-message block. The brief asks for a one-line commit message, so it is required.

**Contract additions**
- Kept the list and cut the whole-section request. The brief asks for the list, and the implementer copies it. Cut the last bullet, which repeated The record.
- No other change. The `wire::Body` item was already gone from the list after round 1; each remaining item is an addition the implementer makes.
