# Response to plan.feedback-2.md

Round-1 points 1–8 were marked settled; nothing in them changed except where a cut below touches them. I rewrote `plan.md` in this directory and grepped the result: the cut text is gone and the kept text is present. The file went from 536 to 422 lines.

1. **Departure 2.** Cut as a departure. "The signals", under "Static paths", now carries one clause: the frame's `/api/v1/` refusal at `src/app.rs:3023` is untouched and `/other` keeps its JSON 404. The remaining departures are renumbered 1–5.
2. **Departures 5 and 6.** Both are gone from the departures list. The revoked-versus-removed reasoning now lives only in the "Later visits" paragraph of "Decision 2", with a clause saying why the issue's plain "revoked" cannot be told from removal. The hover and §5 wording is one sentence in "The UI" and the §5 edit is already in "Docs".
3. **Second mechanisms.**
   - The departure-5 "Copy address" button is now a one-line fallback.
   - The cookie fallback paragraph is now one line at the end of "Why not the cookie".
   - `GET /cert.der` is one line in "The record" (Decision 1). The second mention is gone.
   - The "second listener" argument is cut. The `http:` rejection keeps its one reason: code and token in the clear, against §5.
4. **Departure 3's excerpt paragraph.** Cut to one line naming the fallback. The chosen behaviour and the corrected §4.3 sentence stay.
5. **"The record".**
   - The "Why" bullet is folded into one sentence at the head of "What is built", so the Settled Apple row is still cited.
   - The "buys / does not buy" prose is cut. The dialog sentence in "The UI" now carries it, including that a page from a different host is not protected by the key check.
   - The schema-1-build paragraph is cut. "Nothing reads `schema_version`; plain serde; no migration step" stays.
6. **"The signals".** Cut the cookie-route sentence, the `wss:` cost sentence, the `Connection: keep-alive` bullet and the "standard practice, adopted as such" aside. Pool-and-rate stays.
7. **Decision 2 line tour.** Reduced to `via_subprotocol: bool`, set in `read_request`, read by `connection` before the request moves, passed to `respond_upgrade`.
8. **"Later visits".** Reduced to the stored-key, 401, network-error, backoff and reply-box rules. The view and endpoint list is gone from it. One short list of views stays in the page paragraph under "The UI".
9. **"The actions and effects".** Reduced to the `/me` route. The reissue step and `bind_remote` are gone, and the `invite_url` description moved into the first bullet of "The UI". The page paragraph's host-name and scope sentence stays there.
10. **"The UI".** Dropped the "devices table is unchanged" bullet and "no framework, no build step, no external resource". The inline-script/style ban stays.
11. **"The tests".**
    - Dropped the "No JavaScript runs" preamble. One sentence at the end of the section says the owner's phone run is the only check of the JS and points to the design-status bullet.
    - Dropped the "`/other` and allowlist tests keep passing" line.
    - Collapsed "Not verifiable in this tree" into that pointer. The list now exists once, in "Docs".
    - `me_is_read_scope…` is a bare name.
    - I added one assertion to `a_subprotocol_token_is_checked_like_a_bearer`: a client offering nothing gets a 101 with no subprotocol line. It covers round-1 point 7 now that the spike no longer does.
12. **Spike versus tests.** "The spike" now holds only the pool check. The certificate check lives as the unit test, and the 101 echo as the subprotocol test. The README points at them. The unit test is kept, since the spike copy is what went.
13. **"Contract additions".** Kept as commit-message text only. Dropped the "No `model.rs`…" bullet and the "1 today, so 2" restatement. The schema-version bump is still named in the `Identity` line, because the implementer copies that list. The detail stays in "The record".
14. **"Docs".** The design bullet now says "the gaps listed under Pool and rate, The record and departure 2". The device-only checks are listed once there, as "Not run".
