| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| AE | 3 | 2 | 1 | 2 | note |

## Grounding
- `src/ui/cards.rs:558-574`: found (`agent_excerpt`, the card's one-line excerpt computed UI-side from the transcript).
- `src/ui/cards.rs:176-180`: found (caption picks `agent_excerpt` for agents, captions or host title otherwise).
- `control/src/lib.rs:637-678`: found (`SessionView` with `card`, `waiting`, `waiting_reason`, no excerpt field).
- `docs/remote-client.md` §4.4 names `last_message`: found with a slip (the sentence is line 207, in §4.3 Endpoints, not §4.4; the field is indeed absent from the tree).
- `docs/remote-client.md:316`: found (exact sentence "the host answers with the device's scope"). `src/app.rs:3568-3599` `remote_info` also exact; further spot checks (`app.rs:2962-2966`, `:3127`, `:3636`, `remote.rs:102`, `:573`, `:631`, `:701-706`, `ui/remote.rs:33`, `:157`, `ports/remote.rs:17`, `model.rs:12`, `:612`, `tests/remote.rs:682`, `main.swift:110`, `BrowserView.swift:106, 883`) all land.

## Departures caught
- Safari refuses `wss://` after a click-through: caught (facts 8); page polls every 5 s as the floor, socket as an upgrade, `watch` sent for the open content key.
- Certificate download is not an option with this certificate: caught with a wrong fix. The plan rightly does not build the download, but then reissues the certificate with IP SANs, adds `Identity.addresses`, a schema bump, a new action and a reissue-at-bind path, which is the address binding the reference says the spec avoids.
- Each page file is a connection: caught with a partial fix. It sees the accept loop counts before the path and answers static replies with `Connection: close`, but ships four files and only says a fifth connection is dropped, not what a second device gets.
- Device name is the page's to send: caught (JS fills `device_name`, no host parsing, iPad via `maxTouchPoints`).
- Found beyond the reference: the excerpt is not on the wire; nothing tells a cookie-authenticated page its scope (adds `GET /me`); the invite shows a LAN address while the host listens on loopback; the pair reply would carry the token in its body (omitted when `cookie: true`); revoked and never-paired are indistinguishable to a browser (new `revoked` code).

## Decisions to overturn
- Certificate reissue with IP SANs plus `identity.addresses`, `IDENTITY_SCHEMA_VERSION` bump, `AppAction::CertificateReissued` and `make_certificate`: reverse. Reference decision 1 is no reissue; Chrome still offers Proceed without a SAN, so this buys little and ties the identity to addresses.
- Cookie named `__Host-switchboard`: reverse. Reference decision 2 names the cookie with the server id because cookies ignore ports and a second instance on the same Mac (the README's own dev-aid setup) would overwrite it; the plan's own facts 5 states the port problem and then ignores it.
- Softer: the mandatory `X-Switchboard-Page` header and `page_header` code could be the `Origin` check the reference accepts, and the page could be two files rather than four. Not counted as reversals.

## Over-specification
- The reissue machinery (record field, action, adapter function, bind logic, core and adapter tests, known-gap text) is work the reference cuts, roughly 15%. The "Contract additions" section restates the decisions and actions sections, and "The signals" partly restates §4.4's event kinds. About a fifth to a quarter of the plan is cuttable.

## Hand-off
- Drop the certificate reissue and `identity.addresses` entirely, and name the cookie with the server id rather than a fixed `__Host-switchboard`.

## Notes
- best: Every departure in the reference's errata (excerpt, scope after reload, loopback address, token in the pair body) plus the Safari socket limit, with a poll-first page and the unresolved cookie question ordered as the first thing to test.
- worst: Reissuing the certificate with IP SANs, which drags in an identity field, a schema bump, a new action and tests for a binding the spec deliberately avoids.