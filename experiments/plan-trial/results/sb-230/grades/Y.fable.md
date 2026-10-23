| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| Y | 3 | 3 | 1 | 2 | note |

## Grounding
- `src/core/model.rs:12` (`SCHEMA_VERSION = 16`): found, exactly that.
- `src/core/model.rs:612` (`VIEWS_SCHEMA_VERSION = 7`): found, exactly that.
- `src/ports/remote.rs:17` (`IDENTITY_SCHEMA_VERSION = 1`): found, exactly that.
- `src/adapters/remote.rs:101-110` (`make_identity` builds the cert with no SAN, EKU or basicConstraints): found; `CertificateParams::new(Vec::<String>::new())` plus a CN only, lines 102-110.
- `src/adapters/remote.rs:111` (the pin is over the key): found, off by one; line 112 is `sha256_hex(&key.subject_public_key_info())`, 111 is `let der`.
- Spot checks beyond five (`app.rs:3127-3156` authenticate, `:486-497` revoke closes sockets, `:1863-1886` content refusals, `core/events.rs:132-135`, `core/remote.rs:105-117` and `166-175`, `control/src/lib.rs:637`, `script.rs:730-733`, `tests/remote.rs:426/1587/3226`, `Info.swift:113-116`, `main.swift:12`, `FixturesTests.swift:157-165`, README `:78/:279/:318/:383`, spec `:207/:306/:309/:312`) all land. One misstatement: "`read_request` keeps the query only for paths under `/api/v1/` (`:496-504`)" is written in the present tense but lines 496-504 parse the query for every path; the test section makes clear it is a change the plan makes.

## Departures caught
- WebSocket will not open from Safari: caught; labelled Unresolved rather than asserted, with a polling-by-revision fallback and a "polling" state. The fix is socket-first with a 5 s wait for `hello` before any fetch, where the reference wants poll-first (see decisions).
- Certificate download is not an option: caught; click-through is the trust step, download deferred. But the plan then reissues the certificate anyway for a different reason (see decisions).
- Each page file is a connection: caught; `Connection: close` on static replies so a slot is held one round trip, pool left at 4, step 0 measures the load. Four files rather than the reference's two.
- Device name is the page's to send: caught; `navigator.userAgent` into `device_name`, host cap at 64 noted.
- Found that the reference lacked (errata): the excerpt is not on the wire (adds `SessionView.last_message` plus runtime `AppCore.last_messages`); nothing reports scope after a reload (adds `GET /api/v1/device`); the invite's address when the port is on loopback (falls back to `127.0.0.1`); the pair reply would still carry the token in its body (omitted when `cookie: true`). Also the `Debug` derive on `RemoteRequest` leaking the query, so the query is dropped off `/api/v1/` paths, and `Referrer-Policy: no-referrer` so subresource loads of `/?code=…` do not carry the code.

## Decisions to overturn
- Reissuing the certificate over the same key with `iPAddress` SANs and `serverAuth`, with `certificate_addresses` in `identity.json`, an `IDENTITY_SCHEMA_VERSION` bump, `AppAction::CertificateReissued`, and a once-a-second address check that rebinds and drops every native connection when the LAN address moves. The reference's decision 1 is "no certificate download, no reissue": the identity model binds nothing to an address, and the click-through is offered on the SAN-less certificate (Chrome's `ERR_CERT_COMMON_NAME_INVALID` and Safari's "not private" page both have a proceed link). The plan's premise that both vendors refuse the connection outright conflates trust failure with the interstitial, and it concedes the point is Unresolved while building the machinery anyway.
- Opening the socket and waiting up to 5 s for `hello` before the first fetch. On the device the page is for, where the reference says the socket will not open, every load shows nothing for 5 s. The reference's decision 3 is poll first, socket second: fetch at once, attach the socket when it arrives.
- Not reversed but noted: the cookie design matches the reference (HttpOnly, Secure, SameSite=Strict, `server_id` in the name, bearer fallback, Origin check on POST and upgrade). The "stop and report if the cookie is not kept" gate in step 0 blocks the commit on a manual phone test; I would build and let the acceptance run decide.

## Over-specification
- The reissue and rebind machinery, its core test, adapter tests, DER readback and docs are a quarter of the plan and would be cut with the decision above.
- The closing "Contract additions" list restates the body; the header's recital of three schema versions, "every existing literal takes `set_cookie: None`", the `icon.svg` file, the per-line spec edit list (`:306`, `:309`, `:312`) and the forum-thread leads in step 0 are filler. Roughly a third of the plan an implementer would not need.

## Hand-off
- Drop the certificate reissue and everything hanging off `certificate_addresses`; keep the click-through on today's certificate, and fetch the lists immediately with the socket attached in parallel rather than after `hello`.

## Notes
- best: the token path is exact and complete: cookie named with the server id, bearer wins, Origin rule on POST and upgrade, token out of the pair body, query stripped from the request and the `Referer`, and it caught every departure including four the reference missed.
- worst: a large identity-schema change, a new action and a rebind-on-address-move poll built to serve a SAN'd certificate the page does not need, on a premise the plan itself marks Unresolved.