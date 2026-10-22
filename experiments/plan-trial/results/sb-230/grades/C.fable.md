| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| C | 3 | 2 | 1 | 2 | note |

## Grounding
- `remote_pair` (`src/app.rs:3660`): found (line 3660 is the `json!({token, scope, server_id, device_id})` reply, the one body carrying `scope`, as the plan says; the fn itself starts at 3605)
- envelope (`src/app.rs:3730`): found (`fn envelope` at exactly 3730, with `epoch`, `revision`, `body` only)
- `SessionView` (`control/src/lib.rs:637`): found (struct at 637; fields are id, name, kind, card, waiting, waiting_reason, trust_question, last_stop_at_ms, pending, etc.; no excerpt or last-message field, so the plan's claim holds)
- `connection` (`src/adapters/remote.rs:770`): found (`fn connection(tcp, shared, paired)` at 770)
- `serve_remote`'s rule at `src/app.rs:3023`: found (the `strip_prefix("/api/v1/")` line, 404 `not_found` otherwise)
- (Also checked: `:583` rate consts, `:360` `RemoteRequest`, `:444` `read_request`, `:623` `Limits`, `app.rs:486` revoke-closes-socket, `:3568` `remote_info`, `core/remote.rs:166` `new_device`, `Info.swift:114` `parse`, `tests/remote.rs:464/657/1829`, `tests/ui.rs:7424`, spec line 316 "answers with the device's scope". All exact.)

## Departures caught
- Safari refuses `wss://` to a click-through certificate: **missed**. The page opens the socket, waits for `hello`, then fetches; reconnect-with-backoff is the only fallback. On the device the issue is for, the list never updates and a session that starts waiting does not move; the acceptance line fails. No polling anywhere.
- Certificate download is not an option: **caught** (no SAN, iOS refuses it as a profile, rotation undoes every install; click-through is the trust step, said in the dialog).
- Each page file is a connection holding an unauthenticated slot: **caught with its own fix**. `Connection: close` on the three static replies so no idle slot is held, plus a separate `PAGE_PER_MINUTE` window. Reasonable and testable, though three files rather than two means one more parallel connection, and the plan never says what the four slots leave for a second device.
- Device name is the page's to send (host drops every header but two): **caught with a different fix**. The plan noticed the host keeps no `User-Agent` and adds it to `RemoteRequest`, then parses it host-side in `web::device_name`. The reference has the page fill `device_name` from `navigator.userAgent` with no host parsing.
- Found beyond the reference: no reply after `/pair` carries the device's scope, though spec §5 line 316 says the host answers with it; the plan adds `device {id, name, scope}` to an authenticated `/info`. Verified: `envelope` carries only epoch/revision/body. Real and useful.
- Found beyond the reference: the "one-line excerpt the card shows" is not on the wire; the plan declines a new read model and shows card word, waiting reason and last-stop time. Verified against `SessionView`.
- Found beyond the reference: static files answered on the connection thread, not in `serve_remote`'s frame route, which matches the reference's own preference.

## Decisions to overturn
- **Socket-only live updates.** Reverse to poll first, socket when it opens (reference decision 3). As planned, the page is static on iPhone, iPad and Apple-silicon Safari.
- **Cookie named `switchboard_token` with no server id.** Cookies ignore ports; two instances on one Mac overwrite each other's token. Reference names the cookie by server id.
- Leaning but not counted: host-side `User-Agent` parsing adds a header field, a module function and tests where one line of page script does the job. I would send it from the page, but it is not wrong.
- Not reversed, though the reference differs: three files instead of two. `Connection: close` makes the third connection cheap, so I let it stand.

## Over-specification
- "The signals" says no new signal, then lists the whole §4.4 vocabulary the page consumes; "The UI" is a pointer to earlier sections; "Contract additions" restates the adapter section field by field. The unpaired page's typed-code entry (a code field shown while `pairing_open`) is scope the issue did not ask for. The `/favicon.ico` note, `Max-Age`, `Referrer-Policy` and the `MAX_HEAD` remark are details the implementer would settle in passing. Roughly a fifth of the plan.

## Hand-off
- Safari will not open `wss://` to a certificate the user only clicked through: make the page poll the collections and the open session's content on a short interval and treat the socket as an upgrade, and name the cookie by server id.

## Notes
- best: every one of a dozen-plus line references is exact, and it found two real departures the reference missed (scope never re-reported after `/pair`, no excerpt on the wire) without inventing a read model.
- worst: it never questions that the events socket opens in a browser, so the page it specifies is static on the phone the issue is written for.