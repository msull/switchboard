# Response to feedback 3

All edits are in `plan.md` in this folder; the earlier plan is untouched. I re-read the file and grepped for each new string after editing.

1. **Static arm returns, EOF checked.** Resolved in round 1. No change this round. Sections: *Actions and effects*, "Static files"; *Tests*, Integration bullet 1.

2. **Unpaired replies close, favicon stopped.** Resolved in round 1. No change this round. Sections: *Where this departs* (pool bullet); *Actions and effects*, "Static files"; *Tests*, Integration bullet 1.

3. **`Secret` for cookie and bearer.** Resolved in round 1. No change this round. Sections: *Actions and effects*, "The cookie"; *Tests*, Unit bullet.

4. **Browser pairings capped at `read`; `control` is an open decision.** Resolved in round 2. No change this round. Sections: *Actions and effects*, "Trust"; *Open decisions*.

5. **JSON branch and CHANGELOG.** Resolved in round 1. No change this round.

6. **Cookie `Origin` rule.** Resolved in round 1. No change this round.

7. **Certificate re-issue path.** Resolved in round 1. No change this round.

8. **Spike operator.** Resolved in round 1. No change this round.

9. **Line references.** Confirmed correct. No change.

10. **"Recorded owner decision" wording.** Resolved in round 2. No change this round.

11. **The read cap understates the exposure.** Accepted. I checked `docs/remote-client.md:304`: "these APIs can instruct coding agents and expose private transcripts". Read scope includes the conversation, last-response, screen, run and pinned-file views (§4.3). So a relay that gets past the browser's check does read transcripts at `read`; the cap limits what it can do, not what it can see. Changed:
    - *Actions and effects*, "Trust": the sentence now says a relay reads what a read-scope headset reads, with the transcripts named as the exposure. It also says read scope is a decision in its own right, separate from `control`.
    - *Open decisions*: a second item, **Browser `read`**, asks whether a browser may hold `read` scope at all, given that exposure. The cap at `read` in `remote_pair` stays until the owner decides.

    The feedback asked that the `control` decision not be treated as the only one that matters. The plan now lists both as open.
