# Feedback on plan.md, round 1 (premises)

Checked against the worktree at 380e4d4. These match the tree: the
schema versions (`src/core/model.rs:12` = 16, `:612` = 7,
`src/ports/remote.rs:17` = 1); every `src/adapters/remote.rs` line the
plan cites (`:360`, `:389`, `:444`, `:519`, `:530`, `:567`, `:580`,
`:583`–`:584`, `:631`, `:829`); the `src/app.rs` route table and
`:3127`, `:3568`, `:3606`, `:3650`, `:3754`; the `tests/remote.rs`
anchors (`:245`, `:426`, `:682`, `:1422`, `:1587`, `:1960`, `:3226`);
and `tests/ui.rs:7424`, `:7474`–`:7492`. The plan's premise that revoke
closes a device's sockets holds (`src/app.rs:486`–`495`), and so do the
single-use code and the three-try window (`src/core/remote.rs:65`–`69`,
`src/core/action.rs:972`–`973`). The plan injects no keystrokes, writes
nothing into a project's dot-directories, and its contract changes are
additive. No serialized type changes, so needing no schema bump is
right.

1. **Most of the paired page's first load lands in the 4-slot stranger
   pool, and the plan gives no fallback for it.** The plan only limits
   connections before pairing (decision 4: "at most three connections
   are open at once"). The tree treats every new connection as a
   stranger. The accept loop counts it against `MAX_UNAUTHENTICATED`
   and drops it once 4 are open (`src/adapters/remote.rs:702`–`711`).
   A connection moves to the paired pool only after the frame answers
   one of its requests as authenticated (`:840`–`:855`). A paired
   browser opening the list (page step 4) sends all of these at once:
   - the socket,
   - `projects`,
   - `waiting`,
   - one `sessions` call per project (there is no global listing,
     `src/app.rs:3053`),
   - up to ten `last-response` reads (departure 1).

   A browser runs up to ~6 HTTP/1.1 connections per host in parallel.
   That limit is the Unresolved line on parallel connections, and here
   it applies after pairing too. Any connection past the fourth in
   flight is closed before it is read, and `fetch` sees a network
   error. The fallback ("raise to 6") is written only for the page
   load. The plan must say how the page limits its own requests. For
   example: one queue with at most two requests in flight plus the
   socket; the socket opened after the first authenticated reply; the
   three-retry rule applied to every API call, not just `/info` and
   `/pair`. It must also say how this is tested, for example a
   `tests/remote.rs` case that opens five cookie connections at once and
   asserts how many are dropped, so the limit is fixed in a test. The
   "ten at a time" for `last-response` should drop to whatever that
   queue allows.

2. **Cookies are shared across ports, and the plan only draws one of
   the two consequences.** Settled (RFC 6265 §8.5): the cookie jar is
   not separated by port. The plan answers the overwrite problem by
   putting the server id in the name. The other consequence is that
   every HTTPS server the browser reaches on that IP, on any port,
   receives `__Host-sb-<id>=<token>` in its `Cookie` header. That
   includes a second Switchboard host, and any HTTPS dev server or
   proxy on the Mac that logs request headers. That is a paired
   device's credential in someone else's log. The subprotocol-plus-
   `localStorage` branch does not have this problem, because storage is
   per origin and the origin includes the port.

   The choice between them is a For-the-owner line ("Cookie or
   `Sec-WebSocket-Protocol` for the token?"). Decision 2 does not cite
   it; it cites only the Unresolved storage line. The plan should:
   - cite that For-the-owner line;
   - name this trade where decision 2 is made;
   - add a sentence on it to `docs/remote-client.md` §5;
   - keep the bearer branch as the written alternative, so the owner
     decides on the whole picture.

3. **Three owner decisions are taken without citing the facts line they
   answer, and without the other branch.**
   - Control scope for a browser in the first cut (For the owner, "Should
     a browser get control scope…"): the plan keeps the scope picker as
     it is today, which is the "Yes" branch. It does not say so, and it
     does not write the "No" branch (browser pairings stay read-only
     until raised in the dialog).
   - The device name (For the owner, "typed on the page… or from the
     user agent"): the plan picks "typed, defaulted from the UA" and
     cites only the iPad Settled line. It should cite the owner line
     and say the UA-table branch is the alternative.
   - The acceptance devices: the facts line says "The plan names them".
     The plan says "on the devices the owner names" (Tests, last
     bullet). Name a default set, for example an iPhone with current
     iOS Safari and one Mac with Safari, and mark it for the owner to
     confirm.

4. **The plan never mentions the Unresolved line on whether a
   click-through exception survives.** That line asks whether the
   exception survives a reload, a host restart, a reissued certificate
   and private mode. Decision 1 makes the click-through the trust step,
   and decision 2's 400-day cookie only helps if the page can load
   again later, so both rest on that line. Decision 1's fallback covers
   only "no click-through". Say what happens if the exception is lost
   each session. Either the owner clicks through on every visit, which
   is accepted and said in the dialog, or the same SAN-reissue fallback
   applies. Add this line to the not-yet-run list in the status
   section.

5. **`pairing_url` has no branch for a host with no LAN address.**
   `remote::local_address()` returns `None` when there is no route, and
   it drops loopback (`src/adapters/remote.rs:962`–`970`). The dialog
   already handles that case with an empty `addresses` list
   (`src/ui/remote.rs:156`–`163`). The plan's
   `pairing_url(addr: SocketAddr, …)` needs an address, and The UI
   section does not say what the QR encodes when there is none. Choose
   one: show the JSON QR as today, or show no QR with a line saying the
   Mac has no network address, and hide "Copy link". Add a
   `tests/ui.rs` case for it, or say why it cannot be seeded there.

6. **A cited premise is wrong: `Cargo.toml:91` is
   `[workspace] exclude`.** That key decides which crates belong to the
   workspace, not which files go into the package, so it does not show
   that "`web/` ships in the crate". `include_str!` reads from the
   source tree whatever packaging does, and the repo does not publish
   to crates.io, so the conclusion is harmless. Either drop the
   sentence or cite the right thing: there is no `[package] include` or
   `exclude` in `Cargo.toml`.

7. **I settled the premise that a browser always sends `Origin` on a
   non-GET request. It holds only for one fetch mode, and the plan's
   own header would break the other.** Source: the Fetch standard,
   "append a request `Origin` header"
   (https://fetch.spec.whatwg.org/#append-a-request-origin-header).
   - `Origin` is always added when the response tainting is "cors" or
     the mode is "websocket". So the WebSocket handshake always carries
     it.
   - For any other non-GET request whose mode is not "cors", the
     referrer policy applies, and under "no-referrer" the value is
     `null`.
   - The plan sends `Referrer-Policy: no-referrer` on the page (decision
     4).
   - A same-origin `fetch()` with the default mode ("cors") carries the
     real origin, so the rule works. A `fetch(…, {mode: "same-origin"})`
     POST would send `Origin: null`, and every send would be refused 403
     `origin`.

   The plan should state the rule: the page calls `fetch` with the
   default mode and never sets `mode`, citing the Fetch step above.
   Test 2 (the static scan of `app.js`) should assert that `app.js`
   contains no `mode:`. Do not loosen the host check to accept `null`.
