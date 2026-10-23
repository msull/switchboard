# Plan feedback, round 2

## Earlier points

1. Settled. The certificate now has the server-use extension and an 800-day life, renewed over the same key. Basic Constraints is left as a device test, and the plan promises only polling on iOS. The rcgen defaults the response took on trust match rcgen 0.14.10 `src/certificate.rs:94-96`.
2. Settled. The cookie's reach is now worded as "other HTTPS services on the IP", in the plan, the dialog, the §5 amendment and For the owner 6.

## Cuts

### Where this departs from the issue

- Cut the bullet "Shortening the certificate's life is a spec decision" (lines 40-47). It narrates an earlier draft. What it settles is already stated, with its sources, in the EKU and validity bullet under "The record" (lines 103-115).

### The record

- Cut lines 121-127, the Basic Constraints bullet, or else cut its twin at lines 322-326. Both give the same open question and the same "untested on iOS" wording. For the owner 1 and 3 say it a third time.

### The signals

- Cut the first paragraph (lines 145-151) down to "The page follows §4.4 as the headset does". It restates the spec's event list and its revision rule.

### Host changes

- Drop the counts: "There are 3 `RemoteRequest {` literals" and "There are 30 `RemoteReply {` sites ... converted mechanically". The compiler lists every site once the fields are added. Build order step 1 repeats "the 30-site conversion"; drop it there too.

### The tests

- `write_fixtures`: cut `device.json` and `pair_cookie.json`. No Swift code reads `/device` or pairs by cookie, so the fixtures would have no consumer on the other side. The plan already makes their Swift check conditional. Keep `invite_url.txt`, which `PairingInvite.parse` reads.
- `tests/ui.rs`: cut the sentence saying the test at line 7485 now parses a URL. That test fails as soon as the payload changes, so the suite forces the edit. Keep the new test that the QR is hidden when the port is bound to this Mac only.

### For the owner

- Cut item 3. Its question (Basic Constraints `CA:TRUE` if iOS needs it) is already the second half of item 1.
