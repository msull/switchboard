| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| v4-sonnet-b | 3 | 2 | 2 | 2 | note |

## Grounding
- `src/core/model.rs:12` (`SCHEMA_VERSION = 16`): found (line 11 is the const, value 16; off by one line, same item)
- `src/core/model.rs:612` (`VIEWS_SCHEMA_VERSION = 7`): found (exactly at 612, value 7)
- `src/ports/remote.rs:17` (`IDENTITY_SCHEMA_VERSION = 1`): found (exactly at 17, value 1)
- `control/src/lib.rs:636-688` (`SessionView`, no excerpt field): found (struct spans 637-688; has `card`, `waiting`, `waiting_reason`, `last_stop_at_ms`, no `last_message`)
- `src/bin/switchboard-hook.rs:37` (`last_message` written from `last_assistant_message`): found (the mapping pair is at line 37)
- Also checked: `app.rs:3017/3023/3127-3153/3568/3754/2935/2957`, `adapters/remote.rs:444/625-640/770`, `core/remote.rs:105-117/166-175`, `ui/remote.rs:155/230`, `tests/remote.rs:245/2892/3226`, `tests/ui.rs:7424`, `BrowserView.swift:149-163`, spec line 207. All real and relevant.

## Departures caught
- Safari refuses `wss://` after a click-through: caught; polling every 5 s is the default, the socket an upgrade, with a "live updates unavailable" state.
- Certificate download is not an option with this certificate: half caught. The plan picks the click-through and defers download, but then reissues the certificate with an IP SAN, a `.local` DNS SAN, `serverAuth` and an 800-day validity, which is exactly the binding to an address and expiry that the reference says the spec avoids. It argues the reissue "cannot hurt", which misses the departure's point.
- Each file of the page is a connection: caught with a different fix. Three files (HTML, JS, CSS) rather than two, but `Connection: close` on static replies, a separate static rate window, and an explicit note that the iOS connection count needs a device. Says what happens to the pool for a second device only indirectly (slots are freed on send).
- Device name comes from the page's script: caught; `navigator.userAgent` fills `device_name`, host reads no `User-Agent`.
- Found beyond the reference: the card excerpt is not on the wire (and why `last_message` must not go on `SessionView`: masking needs the secret set, which is off the frame); nothing tells a device its scope after a reload (adds `GET /api/v1/device`); revoked vs never-paired is indistinguishable behind an HttpOnly cookie (non-secret marker in `localStorage`); `serve_remote` refuses stray paths only after a channel slot and a frame pass (connection thread answers them); the pair reply would carry the token in its body (`transport: "cookie"` omits it); `history.replaceState` does not clean what was recorded before the script ran; IPv6 brackets missing in the JSON address form.
- Missed: the invite carries a LAN address from `local_address()` while the host may be listening on loopback.

## Decisions to overturn
- The certificate reissue bundle: two new `Identity` fields, an `IDENTITY_SCHEMA_VERSION` bump, `AppAction::CertificateReissued`, `reissue_certificate`, `local_names` shelling to `scutil`, reissue-on-bind logic and its tests. The reference's decision 1 is no reissue and no download, the click-through alone; the spec's identity model deliberately binds nothing to an address or validity window, and the plan's own evidence says Safari still refuses `wss://` regardless. I would strip this before implementing. One reversal.
- Lesser disagreement, not counted: `app.css` as a third file where the reference wants at most two; `Connection: close` mostly neutralises the pool cost, so inlining the CSS is a small edit rather than a reversal.
- The rest matches the reference: cookie named by server id with `__Host-`, `SameSite=Strict` named as the CSRF defence plus an `Origin`-equals-`Host` check, bearer header untouched for native clients, one URL QR with `PairingInvite.parse` accepting both forms, static paths served on the connection thread as constants with the named headers, `watch` sent for the open session's `conversation` or `screen`, no `model.rs` or fixture change, `/device` as the one new read (justified by the scope gap the errata confirms).

## Over-specification
- The certificate reissue section, its record, action, adapter, `bind_remote` and test paragraphs: about 15 percent of the plan, and it should go entirely.
- "The page's protocol" restates spec §4.2, §4.4 and §4.5 (envelope, held revision, subscribe-then-snapshot, `ops/{op}` outcomes) rather than pointing at them: about 8 percent.
- The "Contract additions" closing list and the schema-version preamble repeat the body: about 5 percent.
- Test bullets are concrete but mostly necessary; the 61-GET rate-window test and the webpki-anchor fallback are over-detailed but short.
- Roughly a quarter to a third an implementer would not need.

## Hand-off
- Drop the certificate reissue entirely (keep `make_identity` and `identity.json` as they are) and inline the CSS so the page is two files; everything else can be built as written.

## Notes
- best: the departures section, which finds the excerpt, scope-after-reload, revoked-vs-never-paired and late-refusal gaps and builds polling-first with an HttpOnly cookie plus an Origin check, all grounded to exact lines.
- worst: reissuing the certificate with address SANs, a `.local` name and an 800-day expiry, which binds the identity to an address and a validity the spec avoids, costs a schema bump and a new action, and buys nothing the plan's own sources say Safari will honour.