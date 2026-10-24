# Correctness review, round 1 (head 4b65af5)

I read the host side (`src/adapters/remote.rs`, `src/app.rs`, `src/core/remote.rs`, `src/adapters/web.rs`), `web/app.js`, the Swift parse, and the new tests in `tests/remote.rs`. `cargo test --locked --test remote` passes (53). I did not run clippy or the other suites. No record or schema change, so there is no `SCHEMA_VERSION` point.

## Points

1. **A cookie the browser never keeps is reported as "removed on the host" (`web/app.js`, `pairFrom` and `unauthorized`).**
   - `pairFrom` stores `{server_id, spki_sha256}` as soon as `/pair` returns 200, then `start()` calls `/me`.
   - Plan Unresolved 3 is whether Safari or Chrome keeps a `Secure; HttpOnly` cookie set from a click-through certificate on an IP-literal host. The spike was "not run", so this path is likely.
   - If the browser drops the cookie, `/me` answers 401. `/info` still matches the stored id and key, so `infer401` returns `removed`. The page then says "This device was removed on the host. Open the pairing window…".
   - That is false. The device is paired, the code is consumed and the window is closed, so the user is told to repeat a step that cannot work.
   - Fix: after a successful `/pair`, treat a 401 on the first `/me` as a separate verdict, such as "paired, but this browser did not keep the credential". The plan's own fallback decision depends on the owner seeing that failure.

2. **An open `screen` session goes stale (`web/app.js`, `tick`, `readSession`).**
   - The plan says the 3 s poll keeps the content watch warm. The code does not do that. `tick` reads only `/waiting`, and a content key is read only inside `refresh`, which runs only when the list marker changed.
   - Plan §4.4 item 3 says a watched key lapses after about a minute without a read.
   - For a non-claude session (shell, command, service), the pane changes without any `SessionView` field changing, so the revision may not move.
   - Then the open screen is read once and never refreshed. The socket's `changed {screen}` is also lost once the watch lapses, and on Safari there is no socket at all.
   - Fix: while a session is open, have `tick` re-read the open session's content route (it also renews the watch). Then compare that reply's own revision.
   - I did not check whether pane output moves the global revision. If it does, this is moot for the polling path but not for the lapsed watch.

3. **Withdrawn.** I first read `a_rotated_key_keeps_the_server_id_and_changes_the_key_the_page_compares` (`tests/remote.rs:4010`) as a test that rotates nothing. It passes because the fake secret store holds no key, so the port makes a new one under the same `server_id`; that is a rotation. It is hard to read, but it is not a correctness defect.

4. **The page window counts methods it never serves (`src/adapters/remote.rs`, `Limits`).**
   - `Limits` matches any request whose path is a page path, but the page branch in `connection` answers only `GET`.
   - A `POST /app.js` spends a page-window slot and then reaches the frame for a JSON 404.
   - A stranger can burn the 120 a minute with `POST /` and 429 real page loads. This is minor, because the same stranger can already do so with `GET /`.
   - Fix: key the window on `GET` plus a page path, so the window and the branch agree.

## Checked, no finding

- `authenticate` takes the bearer first, then the cookie by exact name. A request with a bad bearer and a good cookie is 401.
- The `Origin` rule runs only for cookie-authenticated non-`GET` or upgrade requests. It runs after the token lookup, so an unknown cookie is 401, not 403.
- `/pair` omits `token` and sets the cookie only with `transport: "cookie"`. The native reply is unchanged.
- `invite_url` for IPv4, IPv6 and loopback matches the Swift parse. The key is hex, so `URLSearchParams` does not mangle it.
- The page branch returns before the frame and with `Connection: close`, so the pool slot is freed.
- The shapes `app.js` reads (`body.projects`, `body.sessions`, `body.text`, `epoch`/`revision`, `kind` "claude") match the wire.
