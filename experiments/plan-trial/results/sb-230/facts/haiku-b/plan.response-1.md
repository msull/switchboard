# Response 1 to the feedback on plan.md (issue #230)

Edited: `review/1/repo/plan.md` only. The earlier plan and `facts.md` are untouched. Every edit below was re-read in the file before this response was written.

1. **Re-issue fallback (accepted).** Decision 1, "For the owner" bullet. The fallback now says the current code generates a new key (`src/adapters/remote.rs:101`), so a re-issue changes `spki_sha256` (`:112`). It names the new code: load the PEM from the Keychain item, `KeyPair::from_pem`, a DNS name, 825 days, and the range `:95-127`. The three gaps are written as costs: renewal inside 60 days of `not_after`, with a new browser exception on each re-issue; a `.local` rename, which breaks the name and the cookie origin; and the URL carrying a name. The `pairing_link` signature changed to take a host string, in The actions and effects, and in the contract additions.

2. **Cookie carrier (accepted).** Decision 2 has three changes. (a) The WebSocket question now cites WHATWG WebSockets §2.2 step 2, which sets credentials mode to `include`. I fetched it to check. The poll is the fallback, not the likely path. (b) The poll fallback is written as a change to the session view: the open session refetches every two seconds and the list every five, and `watch` and `changed` are no longer used by the browser. (c) A new gate bullet: if Safari does not store the cookie, the second carrier is `localStorage`, the "script never sees the token" claim is withdrawn, and owner call 2 covers it. The device run is gate step 0 under Order inside the commit.

3. **Trust-model trade (accepted).** The trade was not in "item 11", since the plan has no numbered items. It is in the "key in the URL" bullet under Where this departs. That bullet now names `docs/remote-client.md:309` and `docs/design.md:3111-3112` and points to owner call 8. Docs and status now amends both lines. Owner call 8 is new: accept the weaker browser trust model, or drop the browser path.

4. **Spike first (accepted).** Order inside the commit has a new step 0: `spikes/23-browser-pairing/`, run on a phone, with its README recording each result before any step is written. Spike 23 is free (the directory listing ends at 22). It is in the same commit, so the "one commit" rule still holds. Docs and status lists the spike directory, and the spike is named in the commit body.

5. **Tests that will fail (accepted).** The tests section now names each assertion and its new value. Revocation: `tests/remote.rs:674` (`devices.is_empty()`) becomes a check that the row stays with `revoked` set, and `:676` becomes `(401, "revoked")`. UI: `tests/ui.rs:7487-7492` (the JSON invite) becomes a URL parse. `:7500` becomes a check that the row stays; "No device is paired." still shows, because the list filters revoked rows.

6. **Poll is not "nothing else" (accepted).** Decision 2 owner bullet, rewritten as the change to the session view. The sentence "nothing else changes" is gone.

7. **No LAN address (accepted).** The UI pairing-window bullet, which now handles `local_address()` returning `None`. The window shows no QR and no copy button, says there is no network address, and still shows the code. The loopback case keeps the warning.

8. **Rebinding (accepted).** Decision 2, the Origin rule bullet, plus a new bullet "Rebinding is not stopped by the Origin rule." The argument is cookie scope: the cookie belongs to the host the URL carried, so a rebound name carries no cookie whatever SameSite does. It does not depend on the SameSite answer.

9. **GETs under Lax (accepted).** Decision 2, new bullet "GETs under the cookie". It cites RFC 6265bis §5.6.7.1 for cross-site top-level GETs, and says the CSRF argument covers POST only. It lists the two GETs with side effects, `DeviceSeen` at `src/app.rs:3150` and `ensure_editor` in the draft read at `:3361`, and says a later GET must be checked against that list. The plan's claim is limited to side effects, since the attacker cannot read the response.

10. **Commit message (accepted, with a note).** The brief asks for a one-line commit message, so the subject stays one line. The Commit message section now adds a body: the Contract additions list, copied as written. It names the new fields and the `RevokeDevice` behaviour change. The Contract additions entry for `RevokeDevice` now states the behaviour change with its line, `src/core/remote.rs:86-92`.

11. **Downgrade (accepted as an owner call).** Owner call 9 is new. It names the hazard (no `deny_unknown_fields` at `src/ports/remote.rs:95`, no version on `devices.json`) and the fallback: a versioned wrapper `{schema_version, devices}` that reads both forms and refuses to write a newer version, as `docs/design.md:125-128` requires. The No-bump decision stays as the default. The record section still states the hazard.

12. **Pairing message (accepted).** The pairing sequence now says "This address answers with a different key. The code was not sent." It adds that the check detects an address, not an impostor.

13. **CSP (accepted).** Decision 4 now says `connect-src` names both `'self'` (for `fetch`) and `wss:` (for the events socket), and gives the reason for each. The header and the text now match.

14. **Settled-line contradictions (accepted).** Decision 4 cites the facts' settled line, facts.md:7, and the tree at `src/app.rs:3028-3029`, and points to owner call 10. Decision 2's cookie is owner call 11, which cites `docs/remote-client.md:312`. Both are owner calls with a stated fallback: accept, or drop the browser path. Owner call 10 says the page is needed before pairing, so no authenticated route exists.

15. **Stale line citations (accepted).** The plan's own citations are now correct: `make_identity` at `:95-127`, generation at `:101`, the SPKI hash at `:112`. The SAN-less certificate is at `:102`. `facts.md:8` and `facts.md:10` still carry the old lines (`:97`, `:107-108`). I did not edit the facts file, since the instruction was to edit plan.md only, so that correction is for whoever owns the facts file.

16. **iPad mislabel (accepted).** Decision 6 now says the fallback in terms of consequence: the iPad in desktop-site mode is labelled Mac, the name is only a label, and nothing reads it for a decision.

Not changed: nothing was declined. The feedback's point 5 and point 13 needed no test or design change beyond the edits above.
