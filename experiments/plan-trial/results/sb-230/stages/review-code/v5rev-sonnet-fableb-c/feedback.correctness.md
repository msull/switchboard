# Correctness review, round 1 (head abe87c6)

The Rust side matches the plan. I found no contract or SCHEMA_VERSION problem, since no serialized type changed. Cookie auth, the clearing 401, the `transport: cookie` pairing, the page routes and the rate window all do what the plan says. Three defects are in `web/app.js`. I read the code only; I did not run the page or the Swift tests.

## 1. A rotated key locks the page out of re-pairing (`web/app.js:109-112`)
`start()` checks `stored.server_id === info.server_id && stored.spki_sha256 !== info.spki_sha256`. If that matches, it shows "identity changed. Pair again." and returns, before it looks at the invite.
- State: the browser paired earlier, and the host then rotated its key. The server id is kept across a rotation (`make_identity` is called with `Some(server_id)`), so `stored` still matches by id and differs by key.
- Input: the owner opens the new QR or invite URL. `takeInvite()` has already cleared the fragment. The page shows the "Pair again" notice and posts nothing.
- Result: `localStorage` is only rewritten after a successful pair, so every later scan hits the same branch. The page tells the user to pair again and makes pairing impossible until they clear site data.
- Plan step 2 says to send nothing else when the key differs *without* a fresh invite. The code applies it to a fresh invite too.
- Fix: when an invite is present, compare `invite.key` with `info.spki_sha256` and pair. Show "identity changed" only when there is no invite.

## 2. A refresh requested during a refresh is dropped, and it can stay stale (`web/app.js:157-160`, `:332-334`, `:351`)
`refresh()` returns at once if `state.refreshing` is set. An event whose revision is above `state.rev.projects` calls `refresh()` and gets no retry.
- State: the socket is live, the poll timer skips because `state.live`, and a refresh is in flight.
- Input: a session starts waiting after the in-flight `GET projects` or `.../sessions` was answered, and its `changed` frame arrives before the refresh ends. That second `refresh()` is a no-op.
- Result: the list keeps the old waiting flag. Nothing retries until the next event, and the poll is suppressed while `live`. The "waiting first" view, the point of the feature, can be wrong for an unbounded time on a quiet host.
- Fix: set a `dirty` flag when `refresh()` is called while one is running, and run once more when it finishes.

## 3. A revoked or never-paired device is not shown as such if `/projects` or the poll answers 401 later (`web/app.js:162-165`)
`refresh()` shows "revoked" on a 401, and returns without stopping the interval or closing the socket.
- Result: `schedule()` keeps calling `refresh()` every 5 s once the socket is down. The next call replaces the notice.
- Result: a later non-200, non-401 reply, such as a 503 from a full pool, returns early without touching the notice.
- Effect: the notice survives only until the next `render()`, which a socket `onclose` or `onopen` triggers (`render()` at `:195` rebuilds `main`). A revoked device's notice is therefore replaced by the stale session list on the next socket event, because `render()` redraws `state.sessions`.
- Fix: on 401, set a `state.dead` flag, stop the timer and close the socket, and have `render()` return early on it.

## Not a defect
- The event revision comparison at `:332` is sound. The host has one global revision, so the `projects` reply's revision is comparable with an event's.
- Bearer-first auth, the clearing 401 and the unreachable no-cookie case in `authenticate` (`src/app.rs:3153-3178`) match the plan.
- `limited()` counts any method on `/`, `/app.css` and `/app.js` against the page window. A `POST /` is a 429 rather than a 404 after 120 hits. It is harmless.
