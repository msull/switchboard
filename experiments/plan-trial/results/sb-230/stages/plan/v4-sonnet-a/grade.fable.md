Every anchor I opened is real, including the later ones (about thirty line references, all within a line or two). Scoring now.

```
| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| v4-sonnet-a | 3 | 3 | 2 | 2 | note |
```

## Grounding
- `src/ports/events.rs:24`: found (`last_message: Option<String>` inside `EventKind::Stopped`)
- `src/adapters/hooks.rs:71`: found (`last_message: Option<String>` on the hook line struct)
- `src/core/events.rs:132` (`EventKind::Stopped { pending, .. }`): found (exactly that arm; the core drops `last_message`)
- `src/core/model.rs` "no match for `last_message`": found (grep confirms none; the facts file's "exists on the session record" claim is wrong and the plan corrected it)
- `src/app.rs:1093` (`refresh_captions`): found (doc comment at 1093, `fn refresh_captions` at 1096)
- Also checked: `control/src/lib.rs:636` (`SessionView` at 637), `app.rs:3568/3127/3639/3754/2782/1704/3023`, `remote.rs:360/444/530/567/623/829`, `ports/remote.rs:17`, `ui/remote.rs:141/160/187/201/230`, `tests/remote.rs:245/1406/1422/2892/3226`, `tests/ui.rs:7424`, `Info.swift:109`, `main.swift:107`, `script.rs:730`, `design.md:3585`. All real.

## Departures caught
- Safari refuses `wss://` after a click-through: caught. Polling is the base mode at 5 s, the socket an upgrade, `watch` sent for the open session, "live updates unavailable" in the header.
- Certificate download is not viable with this certificate: caught (deferred to a follow-up as resting on unresolved questions), but with a wrong fix bolted on: a launch-time reissue with SANs, `serverAuth` and 825 days (see decisions).
- Each page file is a connection: caught. `Connection: close` on static replies frees the slot after one request; a dedicated rate window keeps `/pair` and `/info` from starving. It ships three files where the reference wants two; with `close` I would not reverse that.
- Device name is the page's to send: caught. `navigator.userAgent` fills `device_name`; no host-side header parsing, and it declined the facts file's `user_agent` field.
- Beyond the reference: the excerpt is not on the wire (reads `last-response` for waiting sessions, follow-up for a `SessionView` field); no endpoint returns scope after reload (adds `GET /api/v1/device`); the pair reply would carry the token in its body (the `transport: "cookie"` field omits it); "revoked" is indistinguishable from "never valid" at the host; §5's relay sentence does not hold for a browser; the invite names a LAN address while the host may listen on loopback (dialog sentence plus `127.0.0.1` fallback); the `/api/v1/` refusal happens on the frame, so static must be answered on the thread.

## Decisions to overturn
- **Certificate reissue at launch** (new `Identity.certificate_names` and `certificate_not_after`, `IDENTITY_SCHEMA_VERSION` bump, reissue logic in `load_identity`, dialog line, two test groups). The reference says no reissue: the identity model deliberately binds nothing to an address or validity window, Safari still refuses `wss://` by Cockpit's report, and the native clients pin the SPKI so they gain nothing. The plan itself admits the IP SAN is not followed on a running rebind, so every IP change means a rewrite of `identity.json` and a fresh browser warning. I agree with the reference.
- Everything else matches the reference: click-through as the trust step, `HttpOnly; Secure; SameSite=Strict; Path=/` cookie named with the server id, bearer kept for native clients, `Origin` check rather than a CSRF token, one URL QR with `parse` accepting both forms, static paths on the connection thread with CSP, `no-store`, `nosniff`, no new dependency.

## Over-specification
- The whole reissue thread (record section, `certificate_params`/`certificate_due` tests, the legacy-renewal integration test, the dialog line, the follow-up for rebinds) is roughly a tenth of the plan and should go.
- The "Signals" section restates §4.2 and §4.4 protocol mechanics the implementer can read in the spec; the `RemoteReply` literal-site list ("the compiler lists them") is restated code; the facts-file cross-references ("Unresolved 5", "For the owner A") clutter nearly every decision; the third rate window for static files is machinery `Connection: close` already makes marginal.
- Roughly a quarter of the plan is cuttable.

## Hand-off
- Drop the certificate reissue and the two `Identity` fields entirely; the click-through is the trust step and the current certificate stays.

## Notes
- best: Found and corrected the facts file's wrong `last_message` claim, then found the scope-after-reload gap, the token in the pair body, and the browser/relay hole in §5 that the reference also lists.
- worst: A launch-time certificate reissue with SAN, EKU and 825-day expiry, plus an identity schema bump, which the spec's identity model avoids and which no evidence shows helps Safari.