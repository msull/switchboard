# Correctness review, round 1, head ecffe04

Scope: one commit since 7fe8949. `cargo test --locked --test remote` passes (51). The host side matches the plan: cookie name per server id, the `Origin` rule on writes and upgrades, `/me`, `Connection: close` pages answered on the connection thread, the `Limits.page` window, and no schema change. Findings are all in the page and the invite, plus one test gap.

## 1. A transient failure leaves the page stuck on the error screen (`web/app.js:250-253`, `:495-511`)

`read()` answers a network failure (status 0) by calling `notice(...)`, which hides the list and the session and shows the "Can't reach the host" screen. Polling keeps running, because only a 401 calls `stopRefresh()`. When the host comes back, `poll` sees a changed revision and calls `refreshAll`. That redraws the hidden list, and nothing ever calls `show('list')` or `show('session')` again.
- Failing state: paired phone on the list, Wi-Fi drops for one 3 s poll or the phone sleeps and wakes, then the Mac is reachable again. The page stays on "Can't reach the host" until a manual reload, even though every later poll succeeds.
- Fix: on the first successful read after a notice, restore the previous view. Alternatively, show the unreachable text as a banner and leave the section alone.

## 2. Pairing succeeds on the host but a refused cookie reports "Removed on the host" (`web/app.js:308-317`)

After `/pair` answers 200 the host has consumed the code, closed the window and stored a `PairedDevice`. The page then saves `{server_id, spki_sha256}` and calls `/me`. This plan's open question 3 is whether a browser stores or sends the cookie from a click-through certificate. If it does not, `/me` is 401. `unauthorized()` runs `inferUnauthorized(stored, info)`, and the stored and live identity match, so it answers `removed`: "This device was removed on the Mac."
- Wrong text for this state: the device was never removed. It is paired and listed in the dialog, and the user is told to scan again.
- The retry also needs a new pairing window, because the code was single use.
- This is the exact failure the spike exists to detect. The page should tell "I just paired and the cookie did not come back" apart from "removed", for example with a flag set in `pair()` before the `/me` call. A leftover `PairedDevice` with no cookie is also worth a line in the notice.

## 3. A malformed fragment throws before `replaceState`, so the code stays in the address bar (`web/app.js:44-45`, `:259-262`)

`decodeURIComponent` throws `URIError` on input such as `%zz`, and `parseFragment` is not guarded. In `boot()` the throw happens before `history.replaceState`. `boot()` is an unawaited async function, so the page stays on "Loading…" and the fragment (with the code) remains in the URL.
- Only a hand-edited URL triggers it. The QR contents are hex and digits, so it is low severity.
- A partial fragment, such as `#code=…` with no key, returns null. The page then falls through to `/me` and also leaves the fragment in place, with no hint that the link was incomplete.

## 4. Loopback URL with no warning when the listener is on the LAN but no address was found (`src/ui/remote.rs`, the `match address` block)

`invite_url` gets `address = identity.listen.lan.then(local_address).flatten()`. If `listen.lan` is true and `local_address()` returns `None`, for example with no network up, the URL names `127.0.0.1` and the QR is drawn. The `None if identity.listen.lan => {}` arm shows nothing, so neither the Mac-only notice nor an address line appears. The plan only asked for the notice when the port is Mac-only, but here the QR points at an address a phone cannot reach and the dialog says nothing. Show a "no network address found" line in that arm.

## 5. Weak or missing tests against the plan

- `page_loads_free_their_slot_and_have_a_window_of_their_own` loads 120 pages one after another. A client that reads the reply and drops the socket frees the pool slot whether or not the host sends `Connection: close`, so the test would pass if `close` were removed. The plan's assertion (four stalled strangers still make a fifth page request fail at accept) was not added. `stalled_strangers_fill_their_own_pool…` is unchanged apart from the rest of the file.
- There is no test that a `GET` page request carrying an `Upgrade: websocket` header is still answered as a page, or that the page answer does not consume a paired slot. Low value.

## Withdrawn on checking

- A page request on an already-authenticated keep-alive connection: it returns `Ok(())`, which closes the connection. The paired-pool decrement is the same code path as any other close.
- Cookie parsing with `=` in the value: `split_once('=')` splits at the first `=`, so the name is exact and the token keeps any padding.
- The `Origin` comparison on IPv6 hosts: browsers send the same `[addr]:port` form in `Origin` and `Host`, so the comparison holds.
