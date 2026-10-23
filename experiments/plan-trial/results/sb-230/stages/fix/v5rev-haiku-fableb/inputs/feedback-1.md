# Correctness review, round 1: dc1c58a

Read: plan, facts, the diff of the host (adapter, `app.rs`, core), `web/app.js`, the pool and rate tests. Host side matches the plan: cookie auth order, per-host cookie name, clearing only on this host's own cookie, `transport: "cookie"` omitting `token`, static replies before `serve_remote`, no record change (so no schema bump is needed). I ran nothing; points below are from reading.

## 1. The socket is retried on every poll, not every 30 s (web/app.js, `refreshList` / `tick`)

`refreshList` calls `connectSocket()` unconditionally (after `/waiting`). `connectSocket` returns early only if `state.socket` is set. After a failed or closed socket `onclose` nulls `state.socket`, and while `!state.live`, `tick` calls `refreshList()` every 5 s. So every 5 s poll opens a new `wss://` attempt. The `ticks % 6` retry in `tick` is dead in practice, because `refreshList` has already reconnected.

State that shows it: iOS Safari where `wss://` fails after the certificate bypass (the unresolved case the plan designs around). The socket closes at once, `live` stays false, and each 5 s poll opens a fresh handshake. Each is a connection in the four-slot stranger pool until its reply, and a dropped one hits the pool drop. That can starve the page's own `fetch` connections. The plan's step 5 says retry every 30 s. Fix: open the socket from `refreshList` only on the first call, or from `tick` alone. `refreshList` should not own reconnection.

## 2. `hello` triggers a second full list fetch (web/app.js, `onEvent`)

`hello` calls `refreshList()`, and the socket was opened from inside `refreshList`, which is already fetching the collections. Every connect fetches `waiting`, `projects` and every project's sessions twice, concurrently. Both write `state.sessions`, so the last reply wins, not the newest revision. Subscribe-then-snapshot already makes the first fetch safe, so the `hello` refetch is redundant. It is not wrong, but the two fetches race (`listRevision` can be set from an older reply). It also feeds point 1: a reconnect storm becomes a refetch storm.

## 3. "did not keep its pairing" can fire for a later, unrelated 401 (web/app.js, `failed`)

`justPaired` is cleared only after `/waiting` returns OK. If the first `/waiting` fails with a non-401 (host busy, network), `justPaired` stays true. A later revocation 401 then shows "This browser did not keep its pairing" instead of "revoked". Clear the flag on any answered `/waiting`, or on the first 401 decision.

## 4. Stale QR can show with no address (src/ui/remote.rs ~165-172)

With no `local_address`, `invite` is `None`, so `draft.qr` is never rebuilt. The draw at line 172 shows whatever `draft.qr` holds without checking it belongs to `window.code`. Open pairing (QR drawn for code A), close it, lose the network, open pairing again (code B): the dialog shows A's QR, a dead code, beside B and the "No network address" sentence. Clear `draft.qr` when `invite` is `None`, or check the stored code at the draw.

## Not reproduced / fine

- `authenticate`: bearer unknown plus this host's cookie valid gives 200. Another host's cookie name carrying this host's token gives 401 with no `Set-Cookie`. Both match the plan.
- `a_page_load_fits_the_stranger_pool` holds three strangers, moves the fourth, then upgrades a fifth, as planned.
