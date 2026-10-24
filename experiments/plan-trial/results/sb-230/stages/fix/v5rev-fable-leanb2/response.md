# Response to round 1 (head fd5295b)

One commit, `c45051c`, `--squash=fd5295b`: the fix adds `core::remote::reachable_address`, so the body carries the line for the folded commit's contract list. `cargo fmt --all`, `cargo clippy --locked --workspace --all-targets -- -D warnings` and `cargo test --locked --workspace` pass; the page's node tests ran under the suite (not skipped). No `cfg(target_os)` arm was touched, so the Linux clippy of the dispatch crate was not owed.

## Correctness review

### 1. A reply being typed is wiped by any refresh — fixed

`renderSession` in `web/app.js` now keeps a `state.view` for the session on screen (its title, mark, `<pre>` and footer nodes). When a refetch renders the same session, it sets the title, the mark and the text in place and leaves the footer alone, so the textarea keeps its draft, caret, focus and the phone's keyboard; `clear()` drops the view, so leaving the session or a notice starts fresh. The footer is rebuilt only when its kind changes (`footerKind`: nothing while loading, read-only, refusal, compose), and a provisional render after the first draw (a content error) keeps the footer it has, since a failed content read says nothing about whether the device may reply. `footerKind` is a pure export covered in `app.test.js`; the DOM half is still what the plan left to the owner's phone, as nothing drives a browser.

### 2. The Mac-only notice is wrong in one reachable state — fixed

The decision is now one core function, `reachable_address(identity, address)` (`listen.lan` on and an address found), which `invite_url` and the dialog both call; the UI's `on_lan` local is gone (also feedback-2 point 6). The dialog shows the address when it is `Some`, `MAC_ONLY_NOTICE` when `listen.lan` is off, and a new `NO_ADDRESS_NOTICE` ("No local-network address found: the QR code names this Mac's loopback address…") when `listen.lan` is on and no address was found. The core test covers all three outcomes of `reachable_address`. The `lan` on, no address case is not a UI test: `local_address()` is read directly by the dialog and there is no injection point for it, and adding one is a port change beyond this round; the sentence chosen from that result is the only thing left in the UI.

### 3. Spike README claims a browser retry that nothing shows — fixed

The paragraph now says "Not measured against a browser", that a browser opens up to six parallel connections, that a retry of a connection closed with no bytes is not shown, and what a missed `/app.js` or `/app.css` would look like. A new "What the owner's phone checks" section lists "the page loads fully on the first try" first, then the certificate exception, the cookie across a reload, two instances, and the live move to the top. `docs/design.md` points at that list (see point 8 below).

### 4. Minor

- **`origin` refusal with `authenticated = true`** — fixed. It passes `false`: a refused origin says the request did not come from the device's own page, so the connection is not the device's and does not move into the paired pool. The `scope` 403s keep `true`, since there the device itself asked. A comment at the check says so.
- **`Limits::limited` counts any method on a page path** — fixed. It takes the method and spends the page window only on a `GET` of a page path; any other method goes to the frame for its 404. `page_loads_free_their_slot_and_have_their_own_window` now sends `POST /` after the window is spent and asserts the JSON 404.

## Wording and tidiness

### 1. `by_cookie` is named before it is true — fixed

`token` is `request.bearer.clone().or_else(cookie lookup)`, with no cloning arm, and `by_cookie` is computed after the token has been found, where "no bearer" does mean "the cookie carried it"; the comment there says so.

### 2. `NOT_ON_PORT` and the `events` arm — the arm stays, the const stays, with a reason

The `events` move stays, as you asked. The const is not reverted: with the literal inline, `serve_remote` is 101 lines and `clippy::too_many_lines` fails the build (I tried it), and the const is what made the `me` arm fit. Its doc comment now says exactly that, so neither the commit message nor the next reader has to work it out.

### 3. `respond` and `respond_page` hand-build the same head — fixed

One `write_response(w, status, content_type, connection, headers, body)` writes the status line, the content headers, `Connection`, the standing `Cache-Control`/`X-Content-Type-Options` pair (now a `STANDING_HEADERS` const), the caller's headers and the body in one write. `respond` and `respond_page` are thin calls to it. The existing unit test of both writers still passes.

### 4. Rate constant docs — fixed

`PAIR_PER_MINUTE`'s comment covers `/pair` and `/info` only. `PAGE_PER_MINUTE` has its own: `GET`s of the page's files, a load is three, and public because the window test in `tests/remote.rs` spends it.

### 5. `Page`'s fields undocumented — fixed

One-line docs on `path`, `content_type` and `body`.

### 6. `MAC_ONLY_NOTICE` pub without a reason; `on_lan` repeats the core — fixed

The constant's doc says it is public so the dialog's UI test can find the label by it; `BROWSER_NOTICE` and the new `NO_ADDRESS_NOTICE` stay private. `on_lan` is replaced by the core's `reachable_address` (correctness point 2).

### 7. `COOKIE_MAX_AGE_SECS` comment — fixed

It says "Chrome's cap on a cookie's age (Safari has rules of its own)".

### 8. design.md paragraph; remote-client.md line 207 — fixed; confirmed

The one bullet is three: what is built, what rests on the owner's phone (now pointing at the spike README's checks and naming the parallel-connection question), and the web page's known gaps. The excerpt correction is in `docs/remote-client.md`: the `session.last_response` entry (line 215 in this tree, line 207 at the plan's base) says the excerpt is not on the port yet, how the Mac computes it, and what putting it on the wire would take.
