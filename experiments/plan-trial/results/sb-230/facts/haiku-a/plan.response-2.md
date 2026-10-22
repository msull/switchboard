# Response 2 to feedback on plan.md (issue #230)

Edits are in `review/1/repo/plan.md`. The earlier plan is untouched. I re-read the edited file and grepped for each cut and kept item before writing this.

## Replies to round 1

Items 1, 3 to 11, 13 to 18 are accepted as settled. No edit.

- **Item 2 (facts.md:45).** Correct, and it is still wrong. The facts file is outside this round's edit scope, so the plan does not change. The plan no longer relies on that line.
- **Item 12 (spec line 207).** The feedback is right and my round-1 answer was wrong. I grepped `docs/remote-client.md` for `last_message` and found it on line 207 ("that is `last_message` from the `Stop` hook on the record … and moves with `changed {session}`"). I had read only the start of that line. Section "Docs and status", `docs/remote-client.md` bullet, now amends line 207's sentence and line 201 together. The `session.last_response` projection on line 207 is unchanged, because the plan ships that read.

## Cuts

**Where this departs from the issue.** Bullets 9, 10, 12, 13, 14 and 15 are cut. Bullets 7, 8 and 11 remain.

**Record.** The CLAUDE.md and `load_identity` reasoning is cut to "`IDENTITY_SCHEMA_VERSION` stays 1." "Nothing else changes on disk" is cut.

**Signals.** Cut to "None new. The page uses the events socket as `docs/remote-client.md` §4.4 defines it."

**Actions and effects.**
- Pool: the "was never the pairing bound" sentence is cut. The paragraph keeps the number 8, the reason (six per host plus two for `/pair`), and the test. The owner's note is kept inline (see the owner items).
- Static routes: cut to the response list and the one clause about `Connection: close`.
- Reply writer: the "several `Set-Cookie` lines" sentence is cut.
- Request: the manual `Debug` impl and its unit test are cut, and so is the `RemoteRequest` `Debug` line in the unit tests and Steps. The rule that the raw `Cookie` header is never kept stays.
- Pairing: the "SPKI check and code check unchanged" sentence and the fallback sentence are cut.
- Authentication: the Origin rationale paragraph is cut to the rule and the refused forms. The "If the phone run shows another form" sentence is cut.
- Startup certificate: the native-verification parenthetical and the clock sentence are cut.
- Trust decision 1: the "Reasons" and "What this buys" paragraph is cut to the decision and the pre-pairing list. "Why no SAN" is cut to "No SAN, by decision." The hard-fail bullet is kept, since it answers round-1 point 6 and the feedback did not ask to cut it.
- Trust decision 2: the fallback is cut to one sentence, "If Spike 23 shows the cookie is not kept, the plan is reopened." The fallback wire line, the subprotocol line and the commit text that matched them are removed.
- Client-side logic: the sentences on CLAUDE.md's rule and on the projection fallback are cut. The revision rule and the §4.4 citation are kept. A one-line owner note stays, since item 7 had no other home once the owner list was cut.

**UI.**
- "The address-bar code" bullet is cut, and `history.replaceState` stays in Startup. The residual is kept in one sentence under Known costs. The feedback says Known costs and the Gate already hold it, but neither did, so the residual would otherwise be lost.
- "No code and no token": the paragraph after the list is cut to the owner note.
- "Device name": the known-limit clause is cut to "A desktop-mode iPad reads as a Mac."
- "Pairing states and copy" is cut.

**Tests.**
- `the_unauthenticated_pool_refuses_the_ninth_connection` is cut.
- "Not automated" keeps its first sentence only.
- Checks: the macOS sentence is cut; the Linux arm command stays.
- `a_read_only_instance_does_not_reissue_the_certificate` stays.

**Steps.** Steps 1 and 2 are cut to "Fix every struct literal the compiler reports", with the field names and the `new_device` parameter kept. Step 3 is "Built after step 0 is recorded."

**Docs and status.**
- `docs/remote-client.md` §4.1 keeps the bound amendment only.
- §4.3 keeps line 201 and line 207 amendments (see item 12 above).
- "§11: the new test names" and "§11, lines 479 and 502" are cut.
- `docs/design.md`: the conditional is cut.

**Owner items.** The list is cut to the five items with no inline home: device name, test tool, reissue, restart survival, and the phone run. Two deviations:
- Item 11 (unauthenticated cap) is not in the feedback's keep list, and it had no inline note. It moved into "Pool" as an **Owner** note, so the owner still has a place to answer it.
- Item 13 (the acceptance run) is now item 5, "The phone run, Spike 23 (Gate)."

**Acceptance.** The paragraph is cut to the Gate. The Gate did not hold two of the issue's acceptance items, so they moved into it in one clause: a session that starts waiting moves to the top, and a revoked device sees "This browser's access to the Mac has ended" on its next request. The send and the events socket were already in the Gate.

**Known costs.** The `localStorage` sentence is cut. The residual sentence described above is added here.

**Contract additions and commit message.** The "Internal, not contract" line is cut. The attribution paragraph is cut. The commit message is now the subject line plus "The commit body is a copy of this Contract additions list." The body is not written out twice.
