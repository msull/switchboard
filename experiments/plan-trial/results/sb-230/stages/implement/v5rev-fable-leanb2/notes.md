# Implement notes: sb-230 web control surface (v5rev-fable-leanb2)

Branch `stage/sb-230-implement-v5rev-fable-leanb2`, one commit as the plan
asked, on top of `7fe8949` (Release 0.4.1). Not pushed, no pull request.
`git status` is clean after the commit.

## The commit

`fd5295b` "Serve a web control surface from the remote port, paired from
a URL whose fragment carries the code", with the plan's contract-additions
list in the body. 24 files, +3104/-160.

Before committing: `cargo fmt --all`, `cargo clippy --locked --workspace
--all-targets -- -D warnings` (clean), `cargo test --locked --workspace`
(every binary green; `tests/remote.rs` 52 passed including the eight new
tests, `tests/ui.rs` 205, core 535). `swift test --filter
SwitchboardProtocolTests` in `client/` passes (4 tests). The Linux
cross-clippy for `dispatch` was not run: no `cfg(target_os)` arm and
nothing platform-only was touched.

## What was built, by the plan's order of work

1. **Spike 23** `spikes/23-web-page/`: a standalone crate (a throwaway
   HTTPS server over a P-256 CN-only certificate as `make_identity` makes
   it, a page under the app's exact CSP that sets `__Host-sb-<id>`
   HttpOnly, reads it back, prints `document.cookie`, and opens `wss:`),
   plus a README with the commands. **The browser half is "not run"**: the
   session could build and start the two servers but every browser launch
   and direct binary run needed an approval the non-interactive session
   cannot grant. The parallel-connection measurement was done against the
   harness instead and recorded: 4 of 5 parallel page requests served
   against the pool of 4; five sequential loads all succeed.
2. `RemoteReply::new`, `RemoteReply.headers`, `RemoteRequest.{cookie,
   origin, host}`; `respond` writes the reply's headers plus
   `Cache-Control: no-store` and `X-Content-Type-Options: nosniff` on
   every JSON reply. `remote::cookie_value` reads a cookie by exact name.
3. `authenticate` takes the bearer, else this host's cookie
   (`__Host-sb-<server_id>`), and refuses a cookie-only non-GET or
   upgrade without `Origin: https://<Host>` as 403 `origin`. `/pair`
   takes `transport: "cookie"`: `Set-Cookie` with `Max-Age=34560000;
   Path=/; Secure; HttpOnly; SameSite=Strict`, no `token` in the body.
   `GET /api/v1/me` under `read`.
4. `src/adapters/web.rs` (the table of three files, `CSP`, `HEADERS`,
   `page()`); page paths answered in `connection` before the frame with
   `Connection: close`; `Limits.page` at 120/minute (`PAGE_PER_MINUTE`).
5. `core::remote::invite_url` (re-exported from `core`); the dialog
   builds the QR and "Copy invite" from it, shows `MAC_ONLY_NOTICE` in
   place of the address while `listen.lan` is off, and adds the browser
   sentence beside the redaction one. Swift `PairingInvite.parse` takes
   the URL (fragment `code`, `key`, `id`; the authority as the one
   address) and the JSON.
6. `web/index.html`, `web/app.js`, `web/app.css`, `web/app.test.js`:
   boot from the fragment with `history.replaceState` first, `/info`
   comparison, pair by cookie, `/me`, the grouped list (waiting first),
   last response or screen, the reply box under `control` with a kept
   op, `/waiting` poll at 3 s with revision ordering, the events socket
   as an enhancement (20 s poll while connected, 5 s doubling to 60 s
   retry), the `/info` inference on every 401, `textContent` only. The
   routes live in one `ROUTES` table with `{id}` placeholders.
7. Docs: `remote-client.md` §0 dated paragraph, §4.1 pool and window,
   §4.3 rows for the page files, `/pair transport` and `/me`, the
   excerpt sentence corrected, §5 pre-pairing surface, the URL invite,
   the cookie paragraph, the headers; `design.md` a dated paragraph with
   known gaps at the end of "Remote client"; README Layout rows for
   `web/`, `src/adapters/web.rs`, the spike, the dialog and the test
   file, and the dev-aid sentence; CHANGELOG Unreleased.

## Tests added

- Core: `the_invite_is_a_url_whose_fragment_carries_the_code_and_the_key`.
- Adapter unit: cookie parsing; the reply and page writers' headers;
  `web::page` lookup.
- Host (`tests/remote.rs`): the three files with their headers and a
  closed connection, the JSON 404 for everything else outside the API
  (`/favicon.ico`, `POST /`, `/api`, `/index.html`, `HEAD /app.js`), a
  query not echoed; the page referencing only served paths with every
  `/api/v1/` string in `app.js` answering a paired device with something
  other than `not_found`, and the markup/script guards; pairing by cookie
  (attributes, no token in the body, the cookie alone reading `/me`,
  among other cookies, a native pairing unchanged); two hosts' cookies
  kept apart and the fixed name refused; the `Origin` rule on `send` and
  on the upgrade, a bearer exempt; `/me` following `SetDeviceScope`, a
  revoked cookie 401 against an unchanged `/info`; the rotated identity's
  `/info` assertion added to the existing missing-key test; the stalled
  strangers test extended with a page request failing at accept; page
  loads freeing their slot, five in parallel measured, the 121st page
  request 429 with `/info` and `/pair` untouched; `web/app.test.js`
  under `node --test` through a Rust test that skips when node is
  absent (node ran here and passed).
- UI: the "Copy invite" assertions rewritten for the URL and the
  Mac-only notice.
- Swift: the URL parses to the same invite as the JSON; a URL without
  `code` or without a fragment is an error; a trailing newline and an
  IPv6 address.

## What the plan got wrong or did not foresee

- **Reply literal count.** The plan (and facts) said about twenty
  `RemoteReply { .. }` literals in `app.rs`; there were nine (the grep
  counted function signatures returning `RemoteReply`). Converted all
  nine plus the one in `connection`.
- **`serve_remote` length.** Adding the `/me` arm pushed the function
  over clippy's 100-line limit; the catch-all refusal's message moved to
  a `NOT_ON_PORT` constant so the arm fits on one line. No behaviour
  change.
- **`subscribe_at` extension.** The plan said `subscribe_at` (`:1960`)
  "takes extra headers". It took a token; I added `subscribe_with(port,
  headers, path) -> Result<Subscriber, String>` beneath it and made
  `subscribe_at` a wrapper, since a refused upgrade needs to come back as
  an `Err` rather than a panic.
- **Swift IPv6 host.** This Foundation keeps the brackets in
  `URLComponents.host` for an IPv6 literal; the parser strips them before
  rebuilding `[host]:port`. The plan did not mention IPv6; `invite_url`
  brackets an IPv6 address as a URL needs.
- **The spike's browser half.** The plan allowed "not run" and that is
  what happened; Unresolved 1, 2 and 3 stand for the owner's phone and
  for the two Chrome commands in the spike README. The fallback (token in
  the page with a `Sec-WebSocket-Protocol` echo) was not built, as the
  plan said not to.
- **Pre-commit hook.** Whether `core.hooksPath` is set in this worktree
  could not be read (the config query needed approval); the four
  commands were run by hand before the commit instead.
- **Nothing scoped down.** Every item in the plan's order of work and
  test list is in the commit. The one-line excerpt on the wire, the
  trusted-certificate path and the Swift client reading `/me` are the
  plan's own follow-ups, left as listed under design.md's known gaps.
