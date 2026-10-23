# Response to plan feedback 2

## Earlier points

1 and 2 are closed. No change.

## Cuts

All of them are made.

**Where this departs from the issue**
- Cut three bullets: "A browser's acceptance of the certificate is not
  permanent", "'Identity changed' appears after the browser's own
  warning" and "The page learns 'revoked' over REST, never from the
  socket". The signals table carries what they said.

**The record, "Where it runs"**
- Cut the `OpenPairing` caller. The list is now three callers: bind,
  the poll while the window is open, and the hourly `Tick`.
- The "Departures" certificate bullet no longer says "when the pairing
  window opens".
- In "Actions and effects, App", "its four callers" became "its three
  callers".
- In "The tests", the integration test now dispatches `OpenPairing`
  and then runs one poll before it checks the new certificate.
- Cut the "poll-time call is cheap … cached on the app" paragraph and
  its SAN-IP cache.

**The signals**
- Cut the `/info.version … location.reload()` row.

**Actions and effects, App**
- Cut the cookie refresh on cookie-authenticated replies. A lapsed
  cookie falls to "Pair again", which the table already covers.
- Cut the `"device"` and `"page"` capabilities line.
- Cut the matching line under the Commit section's contract additions.

**The tests**
- Swift: cut "`testInviteParsesTheQRText` stays".
- Cut "Fixtures: none change".
- Live acceptance: cut the closing "These steps also answer the owner
  questions …" bullet.

**Native text path**
- Cut the bullet "`switchboard-remote pair` … all go through `parse`".
