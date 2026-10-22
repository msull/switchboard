## Replies to round 1

1. `wire::Body::Device`: settled. The reply is built with `json!` (Routes, Actions and effects), and the plan cites `Body` at `src/app.rs:42`.
2. Connection loop: settled. The page arm returns `Ok(())` and the test reads to end of stream (Pool, Tests).
3. `Host` in the CSP: settled. A character check gives 400 before any header is built, and `page_host_is_validated` covers it.
4. Rotation signal: settled. The plan now cites the key-missing, no-identity and read-only branches and `RevokeDevice`.
5. Camera fallback: settled (new paragraph in Pairing URL). The other two were already covered.
6. Subprotocol and failed open: settled. The plan has the token-alphabet cite and says every failed open runs the `/info` check.
7. Checks against CLAUDE.md and design.md: settled. The line refs I re-checked match, and the 169 correction and the step reorder are right.

## Cuts

**Where this departs from the issue**
- Cut the "certificate trust is unproven" bullet. Certificate trust and the spike say it all.
- Cut the "code never stays in the history" bullet. Flow step 1 and the spike already carry it.

**The record**
- Cut the second to fifth bullets: the schema versions (restated again under Contract additions), "static files are not records" and "the pairing window stays in memory". Keep the first bullet and the three `localStorage` keys.

**Signals**
- Cut the whole section. It lists the events the page uses and says no new signal; the spec §4.4 already says this and the page flow names the events.

**Routes and Transport**
- Cut the last Routes bullet (`/info` unchanged) and the last Transport bullet ("the page's path check and the spec's allowlist"). Both restate other lines.
- Cut the `data:,` favicon link and its spike item. A stray `/favicon.ico` already gets today's 404 and `page_refuses_other_static_paths` expects it.
- Cut the `Cache-Control` rationale sentence. Cut the explanation of what `include_str!` is, since the build forces the files to exist.

**Certificate trust**
- Cut the third "why" bullet (download route), which the fallback paragraph repeats.
- Cut the reissue-with-SAN sentence. It is a later change, so it is not part of this plan.

**Token transport**
- Cut the second and third "why" bullets. The cookie argument restates facts "For the owner" 2, and the third is a restatement of the Transport bullet.
- Cut the `sessionStorage` fallback. No Unresolved line makes storage a risk, and the wrapped-storage line covers failure.

**Pool and rate window**
- Cut the "Own pool or shared" bullet and its separate-pool fallback. The measured fallback (a larger unauthenticated pool) is enough. Cut the "Logging" bullet; Routes already says the query is never logged.

**Pairing URL**
- Cut the "Two payloads" paragraph. The camera fallback already keeps the JSON QR, and it is the only case that needs it.
- Cut the "one URL per address" fallback and the measurement paragraph (the QR size is a number nobody acts on).
- Cut the native-path bullets that say "neither changes" (`switchboard-remote`, headset paste, `SWITCHBOARD_INVITE`). Say once that they all call `parse`.

**Fallbacks beside Settled lines**
- Cut the stored-scope fallback in departures and the control-only fallback under UI. Neither rests on an Unresolved line.

**The page**
- Cut flow item 3 (the "not signed in" wording) down to one sentence.
- Cut the user-agent list to Safari and Chrome on iPhone and iPad, else `Browser`. `Android`, `Windows`, `Firefox` and `Edge` are machinery for cases the issue does not name.

**Tests**
- Cut `page_references_only_served_paths`. It string-scans JavaScript for a case the phone walk and the other route tests already cover.
- Cut "the same with eight in sequence" from the slot test, and the two "existing tests are unchanged" bullets.
- Cut the dialog test in `tests/ui.rs`. It asserts the existing device list.
- Cut the Fixtures paragraph and the no-JS-test sentence. Cut the Gates paragraph, which restates CLAUDE.md.

**Spec, README, status, changelog**
- Cut the detail of the `remote-client.md` edits to a list of the sections touched. Cut the "Order of work" lead-in "The app builds and runs after each step" (CLAUDE.md) and the commit-message block.

**Contract additions**
- Cut the whole section. Each item repeats a line in Routes, Transport or Pairing URL, and the schema-version line repeats The record.
