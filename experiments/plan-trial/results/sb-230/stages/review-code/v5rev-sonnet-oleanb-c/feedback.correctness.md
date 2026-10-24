# Correctness review, round 1: stage/sb-230-implement-v5rev-opus-leanb (4b3a20c)

I read the host side (`src/adapters/remote.rs`, `src/adapters/web.rs`, `src/app.rs`, `src/core/remote.rs`, `src/ui/remote.rs`) and `web/app.js` against the plan and `facts.md`. `cargo test --locked --test remote` passes (52 tests). The host half matches the plan: the cookie name, the `Origin` rule for non-GET and upgrade requests, the omitted `token`, the page answered on the connection thread with `Connection: close`, and `invite_url`. No record or schema change was needed, and none was made. The three points below are in the page.

## 1. A cookie the browser did not keep makes the page say "removed on the host" (`web/app.js:237-243`, `:202-212`)

- `pair()` stores `{server_id, spki_sha256, server_name}` in `localStorage` as soon as `/pair` answers 200. It then calls `start()`, which reads `/me`.
- Plan Unresolved 3 allows that browsers may not store or send the `Secure; HttpOnly; SameSite=Strict` cookie from a click-through certificate on an IP-literal host.
- In that case `/me` is a 401 and `unauthorized()` finds a stored host. `/info` still returns the same `server_id` and key, so `inferUnauthorized` returns `removed`.
- The user sees "This device was removed on the host" right after a successful pairing, when the true state is "paired, but this browser did not keep the cookie".
- The host does have a `PairedDevice` for it, so Revoke on the Mac lists a device the page calls removed.
- `spikes/23-web-page/README.md` says the page "shows 'Not paired' after pairing" in this case. The code does not do that.
- This is the outcome the owner's phone test is meant to detect, and the page hides it.
- Failing state: Safari refuses the cookie, pairing succeeds, and the notice reads "removed".
- Suggested fix: write `localStorage` only after the first `/me` after pairing returns 200. If that `/me` is a 401, say "paired, but this browser did not keep the sign-in". Alternatively, correct the README and give the post-pair 401 its own sentence.

## 2. The page can stay on an error notice after the host comes back, or never leave a blank screen (`web/app.js:214-218`, `:290-321`, `:411-425`)

- `failed()` shows the "Can't reach the host" notice for any non-`HttpError`, such as a dropped network. It does not clear `state.me`, so polling continues.
- After the host is reachable again, `refreshList` → `drawList` calls `show('list')` only `if (!state.open)`.
- With a session open, the earlier `show('notice')` hides the session section and nothing re-shows it.
- A phone that sleeps or changes Wi-Fi while a session is open returns to a permanent "Can't reach the host" screen. Only Back or a reload clears it, and Back is itself hidden.
- Related: `failed()` returns null for an `HttpError` that is not a 401, such as a 429 or 503 from `/me` in `start()`. All sections are `hidden` in `index.html` initially, so the user sees a blank page with no message.
- Failing input: open a session, turn Wi-Fi off for one poll, turn it back on.
- Suggested fix: have a successful `refreshList` or `refreshSession` re-show the right section regardless of `state.open`. Show a notice for a non-401 `HttpError` at start.

## 3. The QR names loopback when the address is unknown, and the window says nothing (`src/core/remote.rs`, `invite_url`; `src/ui/remote.rs:~165-198`)

- With `listen.lan` on and `local_address()` returning `None`, `invite_url` falls back to `127.0.0.1`.
- The dialog then shows neither the "Mac only" notice, which is `!listen.lan` only, nor an address line, which needs a non-empty `address`.
- The QR is therefore a loopback URL that a phone cannot reach, with no explanation.
- Plan: "the URL uses it only when the port listens on the network, and 127.0.0.1 otherwise, with a line in the window". The line covers only one of the two loopback cases.
- Suggested fix: when `lan` is on and there is no address, say "no network address found" in place of the address line.

Nothing else found in the host path. Cookie matching is exact-name, a bearer takes precedence over a cookie, and a cookie-authenticated `GET` or upgrade without `Origin` is exempt or refused as the plan says.
