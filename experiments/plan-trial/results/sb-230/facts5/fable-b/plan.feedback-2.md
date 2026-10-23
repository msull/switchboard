# Review round 2: replies and cuts

## Replies to round 1

1. `PairingInvite.parse` and `Address`: closed; the authority is taken as text and `parse` touches nothing in Transport, which matches `client/Package.swift:18-19`.
2. Stranger-pool count: closed; `/waiting` first, socket after, and the five-connection test now proves the sequence the design counts on.
3. Excerpt departure: closed; the redaction claim is gone and the 207 rewording agrees with 201.
4. Line references: closed; 307-308, 311 and 464 now match the tree.

## Cuts, by section

**Where this departs, first bullet.** The hook-event history (`ports/events.rs:24`, `switchboard-hook.rs:37`, what the core discards) is review record, not implementer need. Keep: no wire view carries an excerpt; the list shows kind, card word and waiting reason; 207 is reworded; the field is a follow-up.

**The record.** The `devices.json` shape and the three version numbers restate the tree. One sentence carries it: no serialized type changes, so no bump.

**The signals.** The section says nothing Decision 4 and page step 5 do not; drop it.

**Decision 3, the `parse` bullet.** The Package.swift dependency direction and what `Address.init?` does with brackets explain a rejected design, not the chosen one. Keep: authority as text into `addresses`, fragment split into `code`, `key`, `id`, JSON otherwise, nothing from Transport.

**Decision 4, CSP.** `img-src 'self'` guards images the page has none of, and `Referrer-Policy: no-referrer` guards links the page has none of (a fragment is never in a Referer anyway). Drop both; `default-src 'none'` already refuses what they would allow.

**Decision 4, pool paragraph.** The keep-alive and idle-timeout mechanics with their five line cites are the tree restated. Keep the design count and the order (three load connections, `/waiting` moves one, then the socket) and the sentence that the poll covers a dropped socket.

**Decision 4, `RemoteReply`.** Two mechanisms are offered for the 30 literals: `headers: Vec::new()` or a `json` constructor. Name one. The default `Content-Type` when the list is empty is already decided, so the constructor is the surplus.

**Decision 4, `/info` capabilities gains `"web"`.** Nothing reads it: the page is same-origin and knows the host serves it, and the native clients have no use for it. Drop it and its contract line.

**The tests.**
- `a_native_pair_sets_no_cookie`: one assertion on the existing `pair` helper's reply; fold it into `a_browser_pairs_from_the_invite_and_reads_through_its_cookie` rather than a test of its own.
- `the_allowlist_refuses_everything_else_with_no_side_effect` gaining `GET /app.js.map` and `GET /web/`: the test already proves arbitrary paths are refused, and the new page test covers `/index.html` and `/app.js/`. Drop the two additions.
- `a_page_load_fits_the_stranger_pool`: "with a paired subscriber open … the first subscriber keeps receiving" is what `stalled_strangers_fill_their_own_pool_and_never_a_paired_clients` (3226) already proves. Keep only the five-connection sequence.
- Fixtures: "parses it with `pairing_invite`'s inverse in the test" writes a second URL parser in Rust beside the Swift one. The Rust side need only assert the fixture equals `pairing_invite(..)` for the fixture identity; the Swift test is the parser's test.

**Docs.** The design.md known-gaps bullet lists the excerpt, the socket on iOS and the one address; the follow-ups section at the end lists the same three. One list is enough: keep the design.md bullet, which is the binding record, and let the plan's follow-ups section name only what design.md does not (the certificate reissue, `Sec-WebSocket-Protocol`, tickets and the conversation).
