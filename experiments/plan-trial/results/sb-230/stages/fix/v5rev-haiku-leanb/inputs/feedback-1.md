# Correctness review, round 1 (head f693952)

`cargo test --locked --test remote` passes (55). I did not run the full workspace suite or clippy. The Rust side matches the plan: cookie name per `server_id`, bearer-wins, `Origin` rule on non-GET and upgrade, `/me`, page answered on the connection thread with `Connection: close`, `Limits.page`, no record or schema change. Findings are few.

## 1. `web/app.js` `openSession`: crash when the user leaves while it loads (should fix)

`openSession` sets `state.session`, awaits `/sessions/{id}`, then writes `state.session.view = ...` and calls `renderSession()`. Both read `state.session` after an await. Nothing guards against it having changed meanwhile.

Failing sequence: tap session A, tap "Back to the list" before the reply arrives. `backToList` sets `state.session = null`. `state.session.view = ...` then throws a `TypeError`, and the same happens at `renderSession()` after `loadContent`.

Second sequence: tap A, Back, tap B quickly. A's continuation writes A's view and content into B's `state.session` (id B). B's page then shows A's name and text until the next refresh. The same applies to `loadContent`, which uses `current` for the fetch but is applied in a view that has moved on.

Fix: capture `const mine = state.session` and return after each await if `state.session !== mine`.

## 2. `web/app.js` `loadList` sets `state.list.stamp` from `/waiting` read before the project and session reads (minor)

If a change lands between the three reads, the stamp is older than the data. The next poll sees a newer stamp and refetches, so the list heals. Not a bug in the end state, but the plan's "a reply older than the one applied is dropped" is applied only to session content (`isNewer` in `loadContent`), not to the list. `refreshAll` can also run concurrently (socket `changed` plus poll), and the slower, older one can overwrite the newer one. Self-heals within 3 s. No action needed unless you want the plan's rule honoured for the list.

## 3. Test gap: `web/app.js` routes against the host (should add or confirm)

The plan asked for a test that every `/api/v1/...` string in `ROUTES` answers something other than `not_found` for a paired device. I did not find one by name in `tests/remote.rs`. If it exists under another name, withdraw this point. The routes I checked by hand all exist in `serve_remote`: `/me`, `/waiting`, `/projects`, `/projects/{id}/sessions`, `/sessions/{id}`, `last-response`, `screen`, `send`.

## Checked and fine

- `truncated` is an array of field names on the wire. The page treats non-empty as truthy, which works because the key is omitted when empty.
- The token is URL-safe base64, so it is cookie-safe.
- A 401 on the socket upgrade: `retrySocket` is scheduled before `stopped` is set, but `openSocket` re-checks `stopped` when the timer fires, so it does not reconnect.
- `authenticate` takes the bearer first. A bad bearer with a good cookie gets 401, which is acceptable.
