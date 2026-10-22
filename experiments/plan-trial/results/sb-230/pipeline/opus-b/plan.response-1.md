# Response to feedback 1

1. **The pool test cannot pass as written.** Agreed. The accept loop
   (`src/adapters/remote.rs:701-707`) drops a fifth connection before TLS.
   Changed in **Tests → `tests/remote.rs` → Browser pairing end to end**:
   - Hold three unauthenticated connections open, with TLS done and no
     request sent, inside the 5 s handshake deadline.
   - Load `/` with the cookie on a fourth connection and keep it open.
   - Show that a fifth connection is then accepted and answers `/info`.
   - The bullet says why a fifth connection opened before the cookie load
     never reaches `serve_remote`.

2. **The plan should not assert that a load fits in four slots.** Agreed.
   I rewrote the *Pool* bullet in **Decisions → 4. Static paths**. It now
   says the plan does not assert that a load fits, and names the causes:
   parallel subresource connections, preconnect, the WebSocket's own
   connection, the 30 s `IDLE_TIMEOUT`, and the silent drop at accept. It
   then adds three server and page measures:
   - **`Connection: close` while unauthenticated.** An unauthenticated
     static reply and the `/pair {cookie:true}` reply carry
     `Connection: close`, and the connection thread closes after writing
     it. Such a connection holds its slot for one request, not 30 s.
   - **Cookie loads use the paired pool.** A load that carries the cookie
     moves to the paired pool.
   - **One retry.** `app.js` retries a failed `fetch` once, after 1 s.

   It also adds a fourth item. The spike records the peak count of open
   connections for a cold load, a reload and a second tab. §4.1 says a
   cold load can meet the pool while another client pairs, and that a
   retry or reload is the remedy.

   Other sections changed to match:
   - **The adapter and the app:** `RemoteReply.close`, and the connection
     loop returns after a reply that sets it.
   - **Tests:** an unauthenticated static reply closes its connection.
     With three slots held, a static load and then a second connection's
     `/info` both answer.
   - **Docs (§4.1)** and **Contract additions** list the new
     `Connection: close` header and the `close` field.

3. **A stale cookie on a static request should not go through
   `authenticate`.** Agreed.
   - **The adapter and the app → `serve_remote`:** a static request now
     looks the token up with `core.device_by_token(&remote::token_sha256(..))`
     directly. A miss writes no `refuse` log line, sends no `DeviceSeen`
     and no clearing `Set-Cookie`, and the file is served as to anyone.
   - **Decisions → 2:** the *Revoked* bullet now limits the clearing
     `Max-Age=0` to `/api/v1/` requests.
   - **Tests:** a revoked cookie on `GET /` answers 200 with no
     `Set-Cookie`. The same cookie on `GET /api/v1/device` answers 401
     with `Max-Age=0`.

4. **`trust_question` is a field on `SessionView`, not a `send` reply.**
   Agreed.
   - **The page → 5. Views → Reply box:** the page reads `trust_question`
     from the session view (`control/src/lib.rs:662`) before sending.
     While the flag is true, the page hides the reply box and says
     "Waiting on the folder trust question. Answer it on the Mac." It
     cites `docs/remote-client.md:279`, and the box comes back on the
     next `changed {session}`.
   - **Tests:** the page-references test also checks that `web::APP_JS`
     reads `trust_question`. The wire side of the field is already
     asserted at `tests/remote.rs:1296`.
   - I did not add core or UI tests. The plan has no core transition and
     no egui widget for this; the decision lives in `app.js`, which has
     no test runner in this repo. The Rust-side checks above are the ones
     the repo can run.

5. **The `RemoteReply` count is wrong, and three `respond` calls were
   missed.** Agreed. Changed in **The adapter and the app → `RemoteReply`**:
   - The literal count is now 29: 28 in `src/app.rs` and the 504
     `no_answer` at `src/adapters/remote.rs:829`.
   - `respond` now takes the content type and the extra headers.
   - The three replies written without a `RemoteReply` are named: the 429
     `rate_limited` (`remote.rs:789`), the 503 `busy` (819) and the 503
     `full` (839). They pass `application/json` and so get the
     every-reply headers.
   - `respond_upgrade` (519) is stated as unchanged.

6. **The `hostname -s` reference points at the wrong file.** Agreed.
   Changed in **Decisions → 3 → Address**. The text now names
   `default_server_name()` at `src/adapters/remote.rs:304-312` as the
   function that runs `hostname -s`. `src/adapters/bonjour.rs:41` is kept
   only as the line that publishes that name as `<name>.local.`.
