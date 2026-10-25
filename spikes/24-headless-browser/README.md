# Spike 24: headless Chromium past a self-signed IP-literal host

Question (issue #233): driven by Playwright with HTTPS errors ignored, does
headless Chromium keep a `__Host-` `Secure; HttpOnly; SameSite=Strict` cookie
from a SAN-less self-signed certificate on `https://127.0.0.1:<port>`, and
does `wss://` to it connect? The browser test for the page depends on both.

Not built by the main crate. It drives spike 23's server, which sets that
cookie and answers a WebSocket upgrade.

## Commands

```sh
cd spikes/23-web-page && cargo build --release     # binary lands in the cargo target dir
cd ../24-headless-browser
npm install playwright && npx playwright install chromium   # ~/Library/Caches/ms-playwright
spike-web-page --port 43987 --id a &               # not 43187: the running app may own it
node probe.mjs https://127.0.0.1:43987/
```

## Output (playwright 1.64.1, Chrome Headless Shell 156.0.8078.4, node 26.11.1, macOS)

```
console Failed to load resource: the server responded with a status of 401 (X)
title: Spike 23
after pair : 200 then /me: 200 {"cookie":"__Host-sb-a"}
on reload  : 200 {"cookie":"__Host-sb-a"}
socket     : frame: hello; closed 1005
cookies    : [["__Host-sb-a",true,true,"Strict"]]
```

(The 401 is the first `/me` before pairing.)

## Recommendation

`newContext({ ignoreHTTPSErrors: true })` is enough: the cookie is stored and
sent, survives a reload, and `wss://` connects under `connect-src 'self'`.
The spike server does not check `Origin`; the app's host does, so the real
test still has to show the socket (a refused socket must not fail it).
Playwright's headless shell downloads into the user's cache, not the repo.
