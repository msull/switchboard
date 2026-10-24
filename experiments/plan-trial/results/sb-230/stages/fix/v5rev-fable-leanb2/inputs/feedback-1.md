# Correctness review, head fd5295b (round 1)

I read the host side (`src/adapters/remote.rs`, `web.rs`, `app.rs` authenticate/pair/me, `core/remote.rs`), `web/app.js`, the UI dialog change and the test list against the plan. No record type changes, so no `SCHEMA_VERSION` step is owed. I did not run the test suite or a browser. The host side matches the plan: cookie by exact per-server name, bearer preferred, the `Origin` rule applied to non-GET and upgrades, the body omits `token` under `transport: cookie`, and pages are answered on the connection thread with `Connection: close`. Points below, most severe first.

## 1. A reply being typed is wiped by any refresh (`web/app.js` `renderSession`, `loadSession`, `poll`)

State: a `control` device has a session open and is typing in the textarea, and the agent is working. Every host revision change (any session activity, not only this one) makes `poll` see a different `/waiting` revision. It calls `refetchAll`, which calls `loadSession`, which calls `renderSession`. `renderSession` runs `clear()` and builds a new textarea. The draft is restored only from `state.pending`, and that is set only inside `sendReply`. So the typed text disappears, with focus, caret and the phone keyboard state, every few seconds while the agent is active. Waiting on the agent and then replying is the main use of the page, and this makes typing a reply unreliable exactly then. The socket path (`changed` leads to `refetchAll`) does the same.

Failing input: open a claude session whose agent is mid-turn, type "yes, go ahead" slowly, and the box empties on the next revision tick.

Needed: keep the compose box across a refetch. Either update only the `<pre>` text and the mark when the same session is already rendered, or save `box.value` into state before `clear()` and restore it after. Neither the plan's Refresh section nor `app.test.js` covers this, because the logic is in the untested browser half.

## 2. The Mac-only notice is wrong in one reachable state (`src/ui/remote.rs`, `pairing`)

`on_lan = identity.listen.lan && address.is_some()`. With `listen.lan` on but `local_address()` returning `None`, the dialog shows `MAC_ONLY_NOTICE`, "allow devices on the local network…". That setting is already on, so the owner is told to enable something that is enabled. `invite_url` does put loopback in the QR in that state, so the URL is right and only the sentence is wrong. This is a small fix: split the two causes, or word the notice as "no network address found". No UI test covers the `lan` on, no address case (it needs `local_address` injected). The plan's test only covers `lan` off.

## 3. Spike README claims a browser retry that nothing shows (`spikes/23-web-page/README.md`, results)

The README says the fifth parallel connection is "dropped at accept… a browser retries a dropped connection, and a page load is three requests, so a single phone never meets the limit". The measurement is 4 of 5 served. Nothing shows a browser retries a connection that is closed with no bytes, and browsers open up to 6 parallel connections to one host. The README itself says the browser half was "not run". The test only asserts `served >= 4`. This is an unverified claim written as a result. If the retry does not happen, a page load on a phone could lose `/app.js` or `/app.css` and show a blank or unstyled page, with nothing on the page to say why. Please soften the claim to "not measured against a browser", and have the owner's phone steps list "page loads fully on first try" among the checks.

## 4. Minor

- `authenticate` returns the `origin` refusal with `authenticated = true`. A cookie-authenticated cross-origin POST therefore moves its connection into the paired pool (`connection`, `reply.authenticated && !paired`) on the strength of a refused request. The plan does not say whether it should. It is harmless, but a refused write should not buy a paired-pool slot, so pass `false` unless the other 403s also pass `true`.
- `Limits::limited` counts any method on a page path, so `POST /` or `POST /app.js` spends the page window and then gets 429, not the JSON 404 the plan promises for them. A test of "`POST /` still gets the JSON 404" passes only because it is not run 120 times in a minute.
