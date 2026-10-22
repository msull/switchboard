| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| K | 3 | 2 | 1 | 2 | no |

## Grounding
- `control/src/lib.rs:637` (`SessionView`): found (struct at 637; carries `card`, `waiting`, `waiting_reason`, `kind`, no excerpt or `last_message` field, so the departure claim holds)
- `src/ui/cards.rs:558` (`agent_excerpt`): found (fn at 558, computes the first line of the last turn locally)
- `client/Sources/SwitchboardUI/BrowserView.swift` (headset fetches `last-response` per row): found (`lastResponses` map filled per ref at lines 149-163)
- `src/adapters/remote.rs:95-127` (`make_identity`, `CertificateParams::new(Vec::<String>::new())` at 102): found (exact; no SAN, no EKU, doc comment says "never expires")
- `src/adapters/bonjour.rs:41` (`<hostname -s>.local.` with empty address list): found (line 41 builds the host from `default_server_name()`, address `""`, then `enable_addr_auto()`)
- Beyond five: nearly every later line (`app.rs` 2891/2928/2936/3017/3127/3606/3636/3650/3754, `remote.rs` 247/268/304/360/623/684, docs 309/312/318, Info.swift 92-117, main.swift 13/107, tests/remote.rs 245/299/327/426/658, tests/ui.rs 7424/7486) checks out. One slip: "the remote group that `src/app.rs:1248` routes" is actually `src/core/action.rs:1249`; `app.rs:1248` is `diff_pass()`.

## Departures caught
- Safari refuses `wss://` over a click-through certificate; page must poll: **missed**. The plan hangs list, session view and reconnect entirely on the events socket, with a spike item "the subprotocol reaches the host from iOS Safari" and no polling fallback. On the device the issue is for, the page would be static.
- Certificate download is not an option: **caught**, but with a fix the reference rejects. The plan rules out download (iOS profile install), then reissues the certificate with SAN/EKU/820 days, daily renewal on `Tick`, a rebind, a new `AppAction::CertificateRenewed`, and a Keychain key read. The reference says no reissue; the spec deliberately binds the identity to no expiry.
- Each page file is a connection: **caught with a different fix**. Four files, each answered on the connection thread with `Connection: close`, a page rate window, and a spike that counts a cold load with a drop-the-icon fallback. It does not say what the pool of 4 leaves for a second device, which the reference asks for.
- Device name is the page's to send: **caught** (`navigator.userAgent` into `device_name`, citing `app.rs:3650-3655`).
- Found by the plan, not in the reference: the card's one-line excerpt is not on `SessionView` (verified; the wire has `card` but no `last_message`), so rows fetch `last-response`. Also: a browser's current scope is not readable, hence `GET /api/v1/device`; cookies ignore ports; browsers probe `/favicon.ico` and must not reach the frame; the Bonjour record publishes no address of its own.
- Plan gap on its own excerpt fix: `changed {conversation}` only arrives for keys in the watch set, so list rows refetched on `changed` go quiet a minute after their last read unless watched; the plan does not say so.

## Decisions to overturn
- **Reissue and renew the certificate.** Conflicts with the reference's "no certificate download, no reissue": it adds an expiry and a `.local` SAN the spec avoids, a daily renewal path, a port rebind per renewal, and one expected browser warning per renewal, which undercuts the dialog's own "a warning at any other time means something else is answering". Safari still refuses `wss://` afterwards, so it does not buy the socket either.
- **Split token: `HttpOnly` cookie half plus a `localStorage` key half sent as `X-Switchboard-Key` or an `sbk.` subprotocol.** The reference's call is the cookie alone with `__Host-`/`SameSite=Strict`, and names `localStorage` for the token as the thing to reverse. The split adds a header, a subprotocol, a new error code, and a Safari seven-day storage wipe that forces re-pairing, to defend against another HTTPS service on the owner's own Mac replaying the cookie.
- Not counted but noted: `<LocalHostName>.local` via `scutil` as the invite host where the reference uses the IP from `addresses`; defensible for iOS and macOS, but it is coupled to the SAN reissue above.

## Over-specification
- The schema-version table under "The record" (no bump happens), the Bonjour aside and its known-gap entry (pre-existing, unrelated), the `sudo scutil --set HostName` spike step, the cookie rationale repeated three times (departures, decision 2, docs §5 rewrite), the "Commit" section restating the contract list, and the full rehydration sequence. Roughly a quarter of the plan.

## Hand-off
- no: three things need changing before an implementer starts (poll-first live updates, drop the reissue/renewal machinery, collapse the token to the cookie), which is more than one line.

## Notes
- best: precise, verified anchors throughout and two real departures the reference lacks (the excerpt is not on the wire; `/favicon.ico` probes must stay off the frame).
- worst: builds the live page on a WebSocket Safari will not open over a click-through certificate, then adds certificate-renewal machinery that does not fix that.