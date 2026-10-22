# Response to feedback round 1

All seven points are accepted. Edits are in `review/1/repo/plan.md` only.

1. **Stranger pool after pairing.** Changed: decision 4. The page sends
   every API request through one queue, `MAX_IN_FLIGHT = 2`. The first
   request (`/info` or `/me`) goes alone, and the socket opens only after
   the first authenticated reply. At most two connections are unauthenticated
   at once, and the three-retry rule covers every API call. Excerpt reads
   go through the same queue (departure 1 now says so, replacing "ten at
   a time"). The "raise to 6" fallback stays. Changed: The tests, new
   case 7 (five cookie connections at once: the fifth is dropped, four are
   served, then a sixth is served once the four are paired; a second case
   runs the queue's shape and drops none), and test 2 scans `app.js` for
   `MAX_IN_FLIGHT = 2` and a single `new WebSocket`. Documents: §4.1 names
   the queue.

2. **Cookie reaches every port on the IP.** Changed: decision 2. It now
   cites the For-the-owner line, states the leak (a second host or any
   HTTPS dev server or proxy logging headers receives the credential) next
   to the cookie's gains, and says the plan builds the cookie unless the
   owner picks the bearer branch. That branch is retitled "Alternative"
   and stays written out. Documents: `docs/remote-client.md` §5 gains the
   one-sentence warning. I kept the cookie as the default rather than
   switching: the `HttpOnly` property is the XSS defence for a page that
   can type into an agent. The owner's call is now explicit.

3. **Owner decisions without citations.** Changed: The record. Control
   scope cites its For-the-owner line (yes, picker as today) and writes
   the "No" branch (`/pair` with `credential: "cookie"` grants `read`,
   raised in the dialog; test 3 asserts it). The device name cites its
   line, and the host-side UA table is the named alternative. The tests'
   last bullet now names a default acceptance set (an iPhone on current
   iOS with Safari, and a Mac with Safari), marked for the owner to
   confirm, and says why Chrome and Firefox on iOS are out of it.

4. **Exception survival.** Changed: decision 1 and the not-yet-run list
   (The tests, last bullet). If the exception is lost between visits, the
   owner clicks through every time and the dialog says so. If it is lost
   within a visit, the SAN-reissue fallback applies.

5. **No LAN address.** Changed: The actions and effects and The UI.
   `pairing_url` takes `Option<SocketAddr>` and returns `Option<String>`.
   With no address the QR is the JSON as today, "Copy link" is not drawn
   and the old scan line shows. A `tests/ui.rs` case cannot seed it,
   because the address comes from the machine's real route. The choice is
   a pure function, so the `src/core/tests.rs` test covers it.

6. **`Cargo.toml:91`.** Changed: decision 5. The sentence is replaced by
   the real fact: no `[package] include` or `exclude`, `include_str!` reads
   the source tree, and the repo does not publish to crates.io.

7. **`Origin` on non-GET.** Changed: decision 2 and test 2. The plan now
   cites the Fetch "append a request Origin header" step. `Origin` is
   always sent for mode "websocket" or "cors" tainting. The page uses the
   default `fetch` mode and never sets `mode`, because under the page's
   `no-referrer` any other mode sends `Origin: null`. The host does not
   accept `null`. Test 2 asserts `app.js` contains no `mode:`.
