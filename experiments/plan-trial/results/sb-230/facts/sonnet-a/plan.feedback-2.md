## Replies to round 1

1. Fragment claim: settled. The RFC sections are cited, the plan says they were not fetched, and the previewer half is marked Unresolved.
2. Hard-failing click-through: settled. The plan now says the page is unusable on that browser and rejects an `http:` listener. Facts line 9 is marked as an inference.
3. Reissue on every minting path: settled. `make_identity` takes `sans` and all four call sites are listed. The new test line covers it.
4. Schema version: settled. It says "1 today, so 2". The "plus one" wording is acceptable now that the 2 is written down and the loader behaviour is checked.
5. Spike: settled. `spikes/23-web-surface` is added with three checks (see cut 12 for the overlap with the unit tests).
6. Stale lines: settled. `device_name` is at `:3650` and the hover at `:113`.
7. Subprotocol plumbing: settled. `via_subprotocol: bool` is justified by the native-client handshake case.
8. JSON dropped from the QR: settled. The cost and the fallback are stated (see cut 3 on the fallback).

## Cuts

1. **Departure 2.** It restates what "The signals" and the `/other` test already say. Cut it to one clause there.
2. **Departures 5 and 6.** Departure 5's revoked-versus-removed behaviour is repeated under "Later visits". Keep it in one place. Departure 6 is two doc/hover edits already listed in "Docs" and "The UI". Drop it as a departure.
3. **Second mechanisms with no ask behind them.**
   - Departure 8's "Copy address" fallback.
   - Decision 2's cookie fallback paragraph (the Why-not-cookie reasoning stays).
   - The `GET /cert.der` fallback, in two places.
   - The "second listener is a second trust boundary" argument.
   Each is a branch the implementer does not build unless the owner asks. One line naming it is enough.
4. **Departure 3.** Drop the "if the owner answers yes, move `agent_excerpt` and add `excerpt`" paragraph. It is a plan for a case that is not being built. Keep the chosen behaviour and the corrected §4.3 sentence.
5. **"The record".**
   - The "Why" bullet restates Facts and `make_identity`.
   - The "what click-through buys / does not buy" prose duplicates the dialog sentence in "The UI".
   - The paragraph on what a build still on schema 1 does with a version-2 file is a case nothing forces. Keep "nothing reads `schema_version`; plain serde; no migration step".
6. **"The signals".**
   - Drop the sentence about the cookie route needing frame state.
   - Drop the `wss:` cost sentence.
   - Drop the `Connection: keep-alive` bullet ("as `respond`").
   - Drop the "Facts says standard practice, adopted as such" aside on the headers.
7. **Decision 2.** The line-number tour of `connection`/`respond_upgrade` (`:797`, `:519`, `:853`, `:506`) is code the implementer will read anyway. Keep "`via_subprotocol: bool`, set in `read_request`, read before the request moves, passed to `respond_upgrade`".
8. **"The page's pairing steps".** The "Later visits" paragraph re-lists views and endpoints already in the issue and the protocol doc. Keep the stored-key/401/backoff rules only.
9. **"The actions and effects".** It is a recap of "The record" and "The signals" (reissue step, `bind_remote` unchanged). Keep the `/me` route as one line and delete the rest.
10. **"The UI".** Drop the "devices table is unchanged" bullet. Cut the page paragraph's "no framework, no build step, no external resource" (the static test forces it).
11. **"The tests".**
    - Drop the "No JavaScript runs" preamble. The README/design status line already says it.
    - Drop "the `/other` and allowlist tests keep passing unchanged".
    - Collapse the "Not verifiable in this tree" paragraph into a pointer to the design-status bullet. It is the same list twice.
    - `me_is_read_scope…` needs no description.
12. **Spike versus unit tests.** Spike checks 1 and 2 duplicate the `rustls` verifier unit test and `a_subprotocol_token_is_checked_like_a_bearer`. Keep the spike for what is not tested later (check 3, the pool). Have the other two live only as the tests, with the spike README pointing at them. Or drop the unit-test copy of the certificate check.
13. **"Contract additions".** It repeats the body. Keep it as the commit-message text only. Drop the "No `model.rs`…" bullet and the schema-version restatement.
14. **"Docs".** The design bullet re-lists the gaps and unrun checks already stated above. Say "the gaps listed under Pool and rate, The record and Departure 3".
