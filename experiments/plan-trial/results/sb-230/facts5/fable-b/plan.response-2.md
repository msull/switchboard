# Response to review round 2

Every cut was made in `review/1/repo/plan.md`; the earlier copy under
`plan/1` is untouched. Each is confirmed in the file.

## Where this departs, first bullet

Cut. The hook-event history (`events.rs:24`, `switchboard-hook.rs:37`,
what the core discards) is gone. The bullet keeps: no wire view carries
an excerpt and adding one is a wire crate change plus masking (201); the
list shows kind, card word and waiting reason; 207 is reworded; the
field is a known gap in design.md.

## The record

Cut. The `devices.json` shape and the three version numbers are gone;
the section ends "No serialized type changes, so no schema bump."

## The signals

Cut to the one line "No new signal."

## Decision 3, the `parse` bullet

Cut. The Package.swift direction and the bracket behaviour of
`Address.init?` are gone; the bullet keeps authority as text into
`addresses`, fragment split into `code`, `key`, `id`, JSON otherwise,
and "`parse` uses nothing from `SwitchboardTransport`."

## Decision 4, CSP

Cut. `img-src 'self'` is out of the policy and `Referrer-Policy:
no-referrer` is out of the header list; the third bullet is now
`X-Content-Type-Options: nosniff` alone.

## Decision 4, pool paragraph

Cut. The keep-alive and idle-timeout mechanics with their five cites
are gone. The paragraph keeps the count and order (three load
connections kept alive, `/waiting` moves one, then the socket as the
fourth stranger at worst), the unresolved retry line, and the sentence
that the poll covers a dropped socket.

## Decision 4, `RemoteReply`

Cut. The constructor alternative is gone; the literals change only by
`headers: Vec::new()`.

## Decision 4, `/info` capabilities

Cut. The "`web`" line is gone from Decision 4 and from the contract
additions list.

## The tests

- `a_native_pair_sets_no_cookie`: folded into
  `a_browser_pairs_from_the_invite_and_reads_through_its_cookie` as its
  last assertion.
- The `/app.js.map` and `/web/` additions to the allowlist test: cut.
- `a_page_load_fits_the_stranger_pool`: the paired-subscriber framing
  and the reference to the stalled-strangers test are gone; only the
  five-connection sequence remains.
- Fixtures: the Rust side now asserts the fixture equals
  `pairing_invite(..)` over the fixture identity; the Swift test is
  named as the parser's test.

## Docs and follow-ups

Cut. The follow-ups section opens with one line pointing at the
design.md known-gaps bullet for the excerpt, the socket on iOS and the
one address, and lists only the certificate reissue,
`Sec-WebSocket-Protocol` and tickets plus the conversation.
