The earlier points are settled and need no answer. All twelve cuts were made.

1. **Departure 8 (the recount).** Cut: "Where this departs from the issue", item 8 is gone. "Actions and effects" still lists the literals.

2. **Schema-bump paragraph and the `Listen.web` sentence.** Cut: "The record" now ends with the versions as they are today and "`model.rs` is not touched". Decision 4's last sentence now reads only that the separate web switch is not built.

3. **Unbuilt branches.** Cut to a clause each: departure 1 ("the other branch is an additive `SessionView` field and is not built"), departure 3 ("a durable record of revoked token hashes … is not built"), decision 1 ("Option B (a reissued certificate, facts:55) stays unbuilt").

4. **Identity check in step 3 and the `sb.host` comparison.** Cut, and the reasoning holds. Step 3 now goes straight to step 4, and says a different Switchboard ignores a cookie named for another `server_id`, so step 4's 401 reports it. `sb.host` is now a `sb.paired` flag that only picks the 401 wording ("The record", step 4). The "lost after a week idle" notes are gone from step 3, "The record" and the §5 amendment in "Docs". The "identity-changed" banner is removed from "The UI".

5. **Step 2's `/info` comparison.** Cut. The page posts `/pair` with the link's key and maps `403 wrong_key` to "this is not the Mac whose code was shown". Consequence:
   - Departure 4 is rewritten as "The page does not compare the key itself". It cites `src/app.rs:3632-3639` and now says the wrong genuine host has seen the code and counted a failure against its own window.
   - Decision 1's "what option A buys" and the dialog sentence now say the key makes a wrong Switchboard refuse, where before they said it stops the page sending the code.
   - The pairing test covers a wrong key giving `wrong_key`.
   - After the `/device` call, step 4 fetches `/info` once for the host's name.

6. **Step 7's second trigger.** Cut. The step now says a socket the CSP blocks never sends `hello`, so the five-second timeout covers it.

7. **Unit tests for `web::asset` and `respond_with`.** Cut from "The tests". `cookie_value` and `same_origin` stay.

8. **`device` fixture.** Cut from "The tests" and from step 6 of the order.

9. **"Unchanged and still holds" remarks.** Cut: the wrappers parenthetical, the `:426` sentence and the `:477, :582, :635` parenthetical.

10. **Closing `cargo` line.** Cut from "Order inside the commit".

11. **Docs closing sentences.** Cut from "Docs". The trailer rule stays in "Commit message" only.

12. **`info.capabilities` remark.** Cut from "The signals", new route.
