All checks done. The `last_message: None` hits in `src/core/tests.rs` are inside hook-event literals, not session records, so the plan's correction of the facts file holds, and the eleven `RemoteReply` literal sites match exactly.

```
| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| v3-sonnet-b | 3 | 2 | 1 | 2 | note |
```

## Grounding
- `src/ports/events.rs:24` (`EventKind::Stopped { last_message, .. }`): found (line 24 is `last_message: Option<String>` inside `Stopped`)
- `src/core/events.rs:132` (core matches `Stopped { pending, .. }` and drops `last_message`): found (exactly that arm, setting `last_stop_at` and `pending_at_stop` only)
- `control/src/lib.rs:637-688` (`SessionView` has no excerpt field): found (struct spans 637-688; `card`, `waiting`, `waiting_reason`, no `last_message`)
- `src/app.rs:3568` (`remote_info`, unauthenticated, no scope): found (`fn remote_info(&self)` at 3568, `authenticated: false`)
- `src/app.rs:3166-3171` (`send` refused with code `scope`): found (`remote_command` returns 403 `"scope"` at 3166-3173)
- Extra: `src/app.rs:3127` `authenticate`, `remote.rs:95/444/519/567/623/770/962` and the eleven `RemoteReply` literals all check out.

## Departures caught
- WebSocket will not open from Safari: caught. Polling every 3 s is the floor, socket is the upgrade, acceptance is stated to hold without the socket. It frames this as "unresolved" rather than known, but builds the right thing.
- Certificate download is not an option: caught with a wrong fix. It drops the download but then reissues the certificate with IP and `.local` SANs, `serverAuth` and 825-day validity, adding identity fields, a schema bump, a core action and rebind logic, which is the address-and-expiry coupling the reference says the spec avoids.
- Each file is a connection: caught, partially. `Connection: close` on static replies and a separate static rate window, but three files rather than two, and it defers the "what is left for a second device" question to the owner's phone run.
- Device name is the page's to send: caught (`device_name` from `navigator.userAgent`, no host parsing, no `kind` field).
- Errata: excerpt not on the wire: caught, and it corrects the facts file's wrong claim with the right lines. Scope unknown after reload: caught (kept from the pair reply, corrected on a refused `send`). Token still in the pair body under a cookie: noted but kept as a feature, so the HttpOnly cookie gains little. Invite address versus loopback listen: missed.
- Found beyond the reference: "revoked" and "never paired" are the same 401 (true, `authenticate` at 3127-3148), so the page distinguishes them by a local marker; the non-API 404 is decided on the frame after taking an inbox slot; the facts file's "about 15" `RemoteReply` sites are 11.

## Decisions to overturn
- Certificate reissue over the same key with SANs, EKU and 825-day expiry, plus `certificate_names`/`certificate_expires` on `Identity`, an `IDENTITY_SCHEMA_VERSION` bump, `AppAction::CertificateReissued`, reissue inside `bind_remote`, and a "turn Local network off and on" instruction when the address moves. Reference decision 1: no reissue, no download; the click-through is the trust step and the identity binds nothing to an address or window.
- Token in `localStorage` with a bearer header on `fetch` and a `bearer.<token>` subprotocol on the socket as the default, cookie behind a `TRANSPORT` constant. Reference decision 2: the cookie, set on `/pair` and read into `bearer` by `read_request`; `localStorage` is the explicit reversal. The plan's `read_request` cookie path and `SameSite=Strict` match the reference, so the fix is flipping the default.
- Minor, not counted: a third static file `app.css` where the reference inlines CSS under a hash; `Path=/api/v1/` on the cookie is fine.

## Over-specification
- The whole certificate-reissue thread (record section, rehydration, the action, the builder, its unit and core tests, its spec and dialog text) is roughly a fifth of the plan and goes away with the decision.
- "Contract additions" restates the body; the running cross-references to `facts.md` Unresolved 1-9 and owner questions A-D, and the list of eleven `RemoteReply` line numbers, are process residue an implementer does not need. About a quarter in total.

## Hand-off
- Drop the certificate reissue and identity schema change entirely, and make the HttpOnly cookie the shipped transport with the token never in `localStorage`; the rest stands.

## Notes
- best: Caught the missing excerpt and the indistinguishable revoked state from the code, correcting the facts file, and built polling as the floor so the acceptance steps hold on Safari.
- worst: Spent a schema bump, a core action and rebind logic on a certificate reissue the identity model deliberately avoids, while leaving the token in `localStorage` by default.