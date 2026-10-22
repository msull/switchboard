| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| O | 3 | 2 | 0 | 2 | no |

## Grounding
- `read_request` (`src/adapters/remote.rs:444`): found (the function signature is on that exact line; it splits the target into path and query as the plan says)
- `docs/remote-client.md` §4.3, the `session.last_response` paragraph saying the excerpt "is `last_message` from the `Stop` hook on the record": found (line 207 ends with exactly that sentence)
- `src/ports/events.rs:24` (`last_message` carried by the hook event): found (`Stopped { last_message: Option<String>, .. }` at line 23-24)
- `src/adapters/hooks.rs:71` (`last_message` on the hook line): found (field at line 71 of the hook-line struct)
- `SessionView` (`control/src/lib.rs:637`) with no excerpt field: found (struct at 637 has `card`, `last_stop_at_ms`, `quiet_secs`, no `last_message`)
- Spot-checked beyond five: `cards.rs:558`, `content.rs:42/827`, `app.rs:2781/2810/2836/2866/3023/3159/3650/486`, `remote.rs:55/95/117/183-197/530/567/621/631/770/982/1012/962`, `ports/remote.rs:17/25`, `ui/remote.rs:113/141/280`, `tests/remote.rs:1406/1423/1587/1829/2892`, `core/tests.rs:11769`, `tests/ui.rs:7436/7474`, `main.swift:107`, `Info.swift:114`, `BrowserView.swift:106/883`, `design.md:3585`, `model.rs:12/612`, spikes 20-22, `time`/`rcgen`/`qrcode` in the lock. All correct. The one thing cited that is not in the tree is `facts.md`, which the plan leans on throughout.

## Departures caught
- WebKit refuses `wss://` to a click-through certificate (reference 1): missed. The plan names "whether `fetch` and `wss://` keep working after a click-through" as unresolved, ships the socket alone, and says that if a browser trusts the page but not `wss://` "the page is unusable on that browser, and the plan accepts that". No polling fallback. That is the device the page is for.
- Certificate download is not an option with this certificate (reference 2): caught with a wrong fix. The plan correctly finds no SAN, no `serverAuth`, year-4096 expiry, and picks click-through, but then reissues the certificate with an IP SAN, EKU and 800-day validity, which the reference says ties the identity to an address and an expiry the spec avoids.
- Each page file is a keep-alive connection against the pool of 4 (reference 3): caught, partly. The plan flags the 30 s idle slot, plans a spike and `Connection: close` as the fix, but serves three files rather than two and does not say what a phone's load leaves for a second device.
- The device name is the page's `device_name`, not host-side parsing (reference 4): caught. Page fills it from `navigator.userAgent`, editable before sending, host keeps no header.
- Found beyond the reference: the card excerpt is not on the wire and the §4.3 sentence is wrong (verified, matches the errata; handled by showing `card`/`waiting_reason` plus a lazy `last-response` first line); a page has no way to learn its scope after a reload (new `/api/v1/me`); the invite carries a LAN address while the port is loopback-only (shows `127.0.0.1` and a message). Also the query-string code reaches the host's request line before `replaceState`, so the plan moves the values to the fragment; the reference does not require it but it is sound and both forms stay accepted.

## Decisions to overturn
- Certificate reissue: new `Identity.sans` and `certificate_issued`, `IDENTITY_SCHEMA_VERSION` bump, `reissue_certificate` on launch, `make_identity` gaining a `sans` parameter at four call sites, an `Identity::covers` check and a "restart Switchboard" line. The reference: click-through is the trust step, no reissue, the identity binds to nothing but the key.
- Token in `localStorage` carried over `Sec-WebSocket-Protocol`: the reference sets an `HttpOnly; Secure; SameSite=Strict` cookie named with the server id, filled into `bearer` by `read_request`, and names `localStorage` as the call to reverse. The plan's CSRF argument about ports is real, but the plan still leaves the token readable by any script on the origin.
- Socket-only live updates: the reference polls first and uses the socket when it opens, because Safari will not open it. The plan builds nothing for that case.
- Minor, not counted: three files instead of two, a new `PAGE_PER_MINUTE` window, a `spikes/23-web-surface` directory. The adapter-side route for the constant files matches the reference.

## Over-specification
- The certificate-reissue block, the `Identity` fields, the reissue unit tests against `WebPkiServerVerifier`, and the "address changed since launch" UI are all work the reference would not do: roughly a sixth of the plan.
- The repeated "Facts (Settled/Unresolved, ...)" citations and "Fallback, built only if the owner asks" lines in every section point an implementer at a document that is not in the tree and at work that is not to be done: roughly another tenth.
- The spike directory and the contract-additions list that restates the body: a few percent. In all about a quarter to a third cuttable; the test list and the page's pairing steps are earned.

## Hand-off
- no. Three reversals, and two of them (socket-only updates, certificate reissue) change what gets built on the device the issue is for and the identity model the spec guards.

## Notes
- best: Every one of some forty line references is exact, and it found three real departures the reference missed, with sensible handling of the missing excerpt.
- worst: It writes down that Safari may refuse `wss://` after the click-through and then accepts an unusable page rather than polling, while spending a schema bump and a reissue on a certificate the reference leaves alone.