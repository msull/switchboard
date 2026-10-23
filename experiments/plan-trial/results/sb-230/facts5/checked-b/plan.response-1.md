# Response to plan feedback, round 1

Both points were right. The plan changed for each. The edits are in `plan.md` under review/1.

## 1. The certificate download rests on nothing, and the deferral is wrongly called a spec change

Accepted in full.

- **"Where this departs from the issue".**
  - The click-through bullet now calls the download an option that is untested on iOS. Polling is the only iOS path the plan promises.
  - A new bullet withdraws the claim that a shorter lifetime is a spec decision. It cites `docs/remote-client.md:308` ("a new expiry") and says the year 4096 lives only in the `make_identity` doc comment. It also says both native verifiers ignore EKU and validity.
- **"The record".**
  - `Identity` gains `not_after: Option<SystemTime>` (`serde(default)`) beside `addresses`. `refresh_certificate` can then tell when a certificate is near expiry without parsing DER.
  - The migration sentence now says an old file reissues once, which also replaces the year-4096 certificate.
  - The schema-version wording is unchanged: the value in `ports/remote.rs` at implementation time, plus one.
- **Certificate bullets in "The record".**
  - `make_identity` and `reissue_certificate` set EKU `serverAuth` and an 800-day validity. Apple's page says issued-after-2019 certificates need both and that connections which violate them "will fail". It does not say user-trusted certificates are exempt, so the default meets the rule at no cost to the native clients.
  - `refresh_certificate` also reissues with under 90 days left.
  - The doc comment on `make_identity` is rewritten.
- **Basic Constraints.** The plan does not set it. A new bullet gives the DTS finding and the opposing risk, which is that browsers may refuse a `CA:TRUE` certificate used as a TLS end entity. It names the question as a device test under For the owner 1, with a matching follow-up (For the owner 3).
- **Wording.**
  - The Decision 1 paragraph no longer says the download "makes the iOS path work". It says it is the one route known to work for `wss://` on Safari, that iOS trust is unsettled, and that polling is the promised path.
  - The download bullet, the page's link text ("may let live updates work; not tested on iOS") and the dialog line (the stronger check is for a device that can install the certificate; iOS is untested) were changed to match.
- **Tests and docs.** The unit tests assert the serverAuth OID bytes, `not_after` within 825 days, reissue under 90 days left, and a one-time reissue for an old file. The `docs/remote-client.md` §5 bullet lists EKU, validity and same-key reissue.
- **Contract additions.** `Identity.not_after` is added.

I did not re-fetch the Apple page or the DTS thread. I rely on the feedback's quotations. The rcgen default claim (no EKU, `NoCa`) is also taken from the feedback and is not re-checked in the vendored source.

## 2. The cookie's cost misreads RFC 6265

Accepted. The decision stands and the wording was wrong.

- **"Credentials".** The cost paragraph now says "other HTTPS services on the same IP". It adds that `Secure` cookies are sent only over a secure channel (RFC 6265 §4.1.2.5, §5.4), so a plain-HTTP dev server never receives `sb_session`.
- **"The Mac's dialog".** The radio-choice line now says "other HTTPS services on this IP".
- **Docs bullet.** The §5 amendment says "for other HTTPS services on the IP".
- **For the owner 6.** It now says "other HTTPS services on the IP".

The facts file (line 42) still names "the dev servers Dispatch starts". It is not part of the plan, so I left it. The plan no longer repeats that reading.
