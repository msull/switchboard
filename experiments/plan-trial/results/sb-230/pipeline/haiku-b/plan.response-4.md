# Response to feedback 4

All edits are in `plan.md` in this folder; the earlier plan is untouched. I re-read the file and grepped for each new string after editing, and all seven are present.

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

11. **Read cap and exposure.** Resolved in round 3. No change this round.

12. **Refusal path for browser `read`, and whether the phone walk waits.** Accepted. The round-4 text covered `control` but not `read`, and it said the phone walk "covers read scope only" when the walk itself depends on both decisions. Changed:
    - *Open decisions*, "Browser `read`": two sub-points. If the owner refuses it, the plan is not built as written. The browser pieces (the static arm, the cookie, `client: "browser"`, `GET /api/v1/device`) come out, and a new plan is written for whatever trust step the owner accepts. The native URL QR and the parser change stay, since they do not depend on the decision. If the owner grants it, `control` is decided second, since `control` means nothing to a browser that cannot hold `read`.
    - *Open decisions*, "Browser `control`": it is moot if browser `read` is refused.
    - *Tests*, "Acceptance, by hand on a phone": the whole walk waits on the two decisions, because a phone pairs only through a browser grant. If `read` is refused, the walk does not run, and the phone acceptance is reported as not done. The "covers read scope only" wording is gone.
    - *Tests*, "Spike gate": the spike rows do not wait on the open decisions. They measure what the browser does, while the decisions are about what the owner accepts.
    - *Documents*: the §5 text now names both decisions, and says a refusal of `read` moves the browser pieces out of the plan.
