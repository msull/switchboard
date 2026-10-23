# Response to plan feedback, round 2

Earlier points 1 and 2 are settled, so nothing changed for them. Every cut was made in review/1 `plan.md`.

## Cuts

- **Where this departs from the issue.** Cut the bullet "Shortening the certificate's life is a spec decision". The EKU and validity bullet under "The record" carries the same claim and sources.
- **The record, Basic Constraints.** Kept the bullet under "The record". Cut its twin in "The certificate for download", which repeated the open question and the "untested on iOS" wording. That bullet is now one sentence about the link's wording on the page. I cut the twin and not the original, because the original is where the decision is made.
- **The signals.** The first paragraph is now "No new event. The page follows §4.4 as the headset does."
- **Host changes.** Dropped both counts ("3 `RemoteRequest {` literals", "30 `RemoteReply {` sites"). The `RemoteReply` bullet now says every construction site is converted. Build order step 1 no longer says "the 30-site conversion".
- **The tests, fixtures.** `write_fixtures` now adds only `invite_url.txt`. `device.json` and `pair_cookie.json` are cut from the tests and from Contract additions, since no Swift code reads them. The conditional `FixturesTests` clause went with them. A Swift test still checks that `PairingInvite.parse` reads `invite_url.txt` and `pin.json` to the same fields.
- **The tests, `tests/ui.rs`.** Cut the sentence about the test at line 7485. Kept the test that the QR is hidden and the hint shown when `lan` is false.
- **For the owner.** Cut item 3 (Basic Constraints `CA:TRUE`), which is the second half of item 1. Items 4 to 7 became 3 to 6.
