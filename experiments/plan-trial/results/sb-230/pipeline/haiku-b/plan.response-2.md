# Response to feedback 2

All edits are in `plan.md` in this folder; the earlier plan is untouched. I re-read the file and grepped for each new string after editing. The grep for the old "recorded decision" wording finds nothing.

1. **Static arm returns, EOF checked.** Resolved in round 1. No change this round. Section: *Actions and effects*, "Static files"; *Tests*, Integration bullet 1.

2. **Unpaired replies close, favicon stopped.** Resolved in round 1. No change this round. Sections: *Where this departs* (pool bullet); *Actions and effects*, "Static files"; *Tests*, Integration bullet 1.

3. **`Secret` for cookie and bearer.** Resolved in round 1. No change this round. Sections: *Actions and effects*, "The cookie"; *Tests*, Unit bullet.

4. **Browser `control` called a recorded decision.** Accepted. The round-1 text stated an owner decision that nobody had made, and I should not have written it that way. Changed:
   - *Open decisions* (new section, before *Commit*): browser `control` is an open decision the owner must make before the §5 text and the reply step are written. Until then every browser pairing is `read`.
   - *Actions and effects*, "Trust": the "recorded owner decision" sentence is replaced. A browser pairing is capped at `read` whatever the window holds. The cap is applied in `remote_pair` before the device is made, and `new_device` (`src/core/remote.rs:166`) gains a scope argument, since it reads the window's scope today.
   - *UI*, "Pairing window": the control line now says "A browser paired here gets read scope. Control from a browser is not yet allowed."
   - *Tests*, Integration: new test `a_browser_pairing_is_granted_read_whatever_the_window_holds`.
   - *Tests*, "Acceptance, by hand on a phone": the reply step waits for the owner's decision. Until then the phone walk covers read only.
   - *Tests*, "Documents": the §5 text is now marked as waiting on that decision, not as the owner's choice.
   - *Contract additions*: the browser pairing's granted scope is `read`.

   The feedback offered two options, a marked open decision or a cap at `read`. The plan does both: the cap holds now, and the decision stays open. Wording that asserts an owner decision is removed; a grep for "recorded owner", "recorded choice", "recorded decision" and "not an oversight" finds nothing.

5. **JSON branch and CHANGELOG.** Resolved in round 1. No change this round.

6. **Cookie `Origin` rule.** Resolved in round 1. No change this round.

7. **Certificate re-issue path.** Resolved in round 1. No change this round.

8. **Spike operator.** Resolved in round 1. No change this round.

9. **Line references `src/app.rs:3024` and `:359`.** Confirmed correct. No change.

10. **Plan states an owner decision that was not made.** This is the same point as 4 and is handled there. The Trust paragraph, the Documents bullet and the §5 text are all reworded. The plan now says the owner has not decided, and it marks the decision open.
