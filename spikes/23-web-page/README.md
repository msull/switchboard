# Spike 23: the page on the remote port

The questions issue #230's plan leaves to a browser, which no test in
the tree can drive: whether Chrome and Safari on the Mac store and
send a `Secure; HttpOnly; SameSite=Strict` cookie set by a server with
a self-signed P-256 certificate that has no subject alternative name
(the one `make_identity` makes today) on an IP-literal host once the
owner has clicked past the warning; whether two servers on two ports
of one address keep their cookies apart when each names its cookie
for its own server id; whether a WebSocket to the same origin is
admitted under `connect-src 'self'` and connects over that
certificate; and how many connections a page load opens against the
port's pool of four unauthenticated slots.

Standalone crate, not built by the app. Builds into its own
`target/`, as spike 22 does.

## What it does

`src/main.rs` is a throwaway HTTPS server: a fresh key and SAN-less
certificate per run, a page under the port's exact
Content-Security-Policy, `POST /pair` setting
`__Host-sb-<id>=token-for-<id>; Max-Age=34560000; Path=/; Secure;
HttpOnly; SameSite=Strict`, `GET /me` answering 200 when that cookie
(by its exact name) came back and 401 otherwise, and `GET /events`
answering a WebSocket upgrade with one text frame and a close. The
page shows three lines: the cookie after Pair, the cookie on a reload
(the `SameSite=Strict` case: a reload is same-site, the first open
from a camera app is not), and the socket's outcome.

## Commands

```sh
cd spikes/23-web-page
cargo run --release -- --port 43187 --id a      # one instance
cargo run --release -- --port 43188 --id b      # a second, in another shell
```

Then, in Chrome and in Safari on the Mac:

1. Open `https://127.0.0.1:43187/`, proceed past the certificate
   warning (Chrome: Advanced, then Proceed; Safari: Show Details, then
   visit this website). Does the page load at all? This is Unresolved 1
   of the issue's facts.
2. Click Pair, then Read /me: 200 says the cookie was stored and sent.
3. Reload: the "on reload" line is the `SameSite=Strict` same-site case.
4. Open `https://127.0.0.1:43188/` and do the same: its id is `b`, and
   its cookie is `__Host-sb-b`. Return to the first tab and Read /me
   again: 200 says the second pairing did not overwrite the first.
5. Click "Open the socket": "frame: hello; closed 1005" says `wss:` to
   this certificate is admitted by the policy and connects; "error;
   closed 1006" says the browser refused the socket (Safari is expected
   to, per the facts' "Certificate trust"), in which case the page's
   polling is the path and nothing else changes.

The server prints nothing of a request. A phone on the same network
opens the same URL with the Mac's address in place of `127.0.0.1`.

## Results

### The browser steps on the Mac: not run

Steps 1 to 5 need a browser driven by hand past a certificate warning,
and the stages that built this ran without a display session they may
type into (the repo forbids keystroke injection). The page and the
host are built to the plan's defaults whatever the answers are: the
cookie is the page's only credential, and a browser that does not keep
it gets "Paired, but this browser did not keep the sign-in" on its
first `/me`, with the fallback (the token held by the page and echoed
as a `Sec-WebSocket-Protocol` value) left as the owner's call; and the
page meets its acceptance steps by polling, so a refusal of `wss://`
costs only the socket's speed.

The same five steps can be run against the app itself rather than
this harness, two instances on two ports of one address:

```sh
SWITCHBOARD_DATA_DIR=/tmp/sb-a SWITCHBOARD_REMOTE_PORT=43190 \
  SWITCHBOARD_TMUX_SOCKET=switchboard-test-web-a cargo run --locked &
SWITCHBOARD_DATA_DIR=/tmp/sb-b SWITCHBOARD_REMOTE_PORT=43191 \
  SWITCHBOARD_TMUX_SOCKET=switchboard-test-web-b cargo run --locked &
```

In each, open Remote devices, allow devices on the local network, open
a pairing and open the invite URL ("Copy invite") in Chrome and in
Safari. Expected: the page lists the sessions; Web Inspector's storage
pane shows one `__Host-sb-<id>` cookie per instance; reloading either
tab still lists; the Network pane shows `/api/v1/events` upgraded
(101) or failed with 1006.

### On the owner's devices (2026-10-23): steps 1 and 2 pass

The owner paired an iPad (Safari), an iPhone (Safari) and an Android
tablet (Chrome) against the app's own listener on this branch's build
and its siblings from the same plan, each after its browser's
certificate warning (Safari: Show Details, visit this website; Chrome:
Advanced, proceed). On all three the page loaded, the cookie was
stored and sent back on the page's requests, the page listed the
sessions and, under control scope, delivered a reply to a session.
Reloads kept the sign-in. So the certificate click-through covers the
page's `fetch` on current iOS, iPadOS and Android, and the
`Secure; HttpOnly; SameSite=Strict` cookie is kept on an IP-literal
origin the owner clicked through: the question every plan for #230
left unresolved. Not observed: whether the socket opened (the page
polls either way and showed no difference), and the two-host step.

### Five parallel connections against the pool: run

A temporary test beside `tests/remote.rs`'s helpers opened five TLS
connections at once, released by a barrier, each asking for
`/app.js`, five rounds:

```
round 0: 200 200 200 reset 200   (~1.0 ms each, the reset at 0.5 ms)
round 1: 200 200 200 200 reset   (~3.5 ms)
round 2: 200 200 200 200 reset   (~2.4 ms)
round 3: 200 200 200 200 reset   (~0.6 ms)
round 4: 200 200 200 200 reset   (~0.7 ms)
```

Every round, four were answered and the fifth was closed at accept
(`ConnectionReset`): the pool counts a connection from accept until its
first authenticated reply, and a page reply does not authenticate. With
`Connection: close` each slot frees as soon as its file is written, so
sequential loads never meet the limit (`a_page_load_frees_its_slot_at_once`
in `tests/remote.rs`), and the pages have a window of their own, 120 a
minute per listener (`page_requests_have_their_own_rate_window`).

What this means for the page: a browser opens one connection for `/`,
then fetches `app.css` and `app.js`, typically in parallel on new
connections, three at most. The page itself must not fan out: a
`Promise.all` over every project's sessions would have the browser open
up to six connections, each unauthenticated until its first reply, and
the fifth would be refused. `web/app.js` therefore reads the projects'
sessions one at a time on its kept connection, which moves to the
paired pool on its first authenticated reply. How many connections
Safari and Chrome open per load, and whether either asks for
`/favicon.ico` despite the `data:` icon, is still unmeasured.

## Recommendation

Build as the plan says, which this branch does: the cookie transport,
the pages on the connection thread with `Connection: close`, polling as
the base and the socket as the enhancement. The owner's three devices
settle the cookie question in the page's favour; the Mac browser steps
above remain the way to watch the socket and the two-host case.
