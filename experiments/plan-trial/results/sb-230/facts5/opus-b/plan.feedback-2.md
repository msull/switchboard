## Earlier points

1. Closed. `refresh_certificate` runs on open, on the poll and on `Tick`, and the QR prints `certified_address`. The swap is preferred over a rebind because a rebind drops `Shared` and gives kept connections a 503 (`src/app.rs:2936-2940` sets `self.remote = None`). That is evidence I did not have, and it holds.
2. Closed. "The implementer's first test" names the cookie mechanic, makes the probe the first step, and states the fallback for a "no" as an owner follow-up.

## Cuts

**Where this departs from the issue**
- Cut three bullets. They restate the signals table and set no work: "A browser's acceptance of the certificate is not permanent", "'Identity changed' appears after the browser's own warning", and "The page learns 'revoked' over REST, never from the socket". Rows 2, 3 and 7 of the table already carry them.

**The record, "Where it runs"**
- Cut the `OpenPairing` caller. The QR prints `certified_address`, which is always an address the served certificate carries. The once-a-second poll while the window is open already follows a move within a second, and it is the first call after opening.
- Cut the "poll-time call is cheap … cached on the app" paragraph and its SAN-IP cache. One UDP `connect` a second while a five-minute window is open needs no cache. Re-reading the slot's DER through `certificate_facts` is enough.

**The signals**
- Cut the `/info.version differs … location.reload()` row. Nothing in the plan re-reads `/info` after load, so the row never fires. `Cache-Control: no-store` on the page files already makes the next reload current.

**Actions and effects, App**
- Cut the cookie refresh on every cookie-authenticated `GET device` reply. It is an enhancement placed ahead of its default: when the cookie lapses after 400 days, the page shows "Pair again", which the plan already handles.
- Cut `"device"` and `"page"` in `remote_info` capabilities, and the matching line in the Commit section. No reader exists: the page ships in the same binary as the host, and no native client is planned to check either word.

**The tests**
- Swift: cut "`testInviteParsesTheQRText` stays". There is nothing to do.
- Cut "Fixtures: none change". The section has nothing to say.
- Live acceptance: cut the closing "These steps also answer the owner questions … covered by the default". The first-test section now owns those questions.

**Native text path**
- Cut the bullet "`switchboard-remote pair` …, the headset's paste and `SWITCHBOARD_INVITE` all go through `parse`, so none of them changes". It restates the tree, and the Swift test forces it anyway.
