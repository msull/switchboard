| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| X | 3 | 3 | 1 | 2 | note |

## Grounding
- `src/adapters/remote.rs:102-110` (`CertificateParams::new(Vec::<String>::new())`, no SAN): found (line 102 is exactly that call inside `make_identity`, CN pushed at 104-107, `self_signed` at 108-110; the doc comment at 90-92 confirms the year-4096 expiry).
- `control/src/lib.rs:637-688` (`SessionView` with `card`, `waiting`, `waiting_reason`, no excerpt): found (struct spans 637-688; fields match, no `last_message`).
- `docs/remote-client.md:207` (spec describes a `last_message` field that exists nowhere): found (line 207 says the excerpt "is `last_message` from the `Stop` hook on the record").
- `src/ui/cards.rs:558` (`agent_excerpt` computed UI-side from the transcript): found (`fn agent_excerpt(c: &Conversation)` at 558).
- `src/adapters/remote.rs:567`, `699-706` (pool of 4, fifth stranger dropped at accept): found (`MAX_UNAUTHENTICATED: usize = 4` at 567, `MAX_PAIRED` 12 at 568; the drop at accept is 699-706).
- Also checked: `src/ports/remote.rs:26-43` (`Identity`, `IDENTITY_SCHEMA_VERSION` 1 at 17), `src/app.rs:2936-2985` (`bind_remote`, `tls_config` at 2957), `src/ui/remote.rs:157-166` (payload, `format!("{ip}:{port}")` with no IPv6 brackets), `src/core/remote.rs:105` (`update_identity`), `src/app.rs:3580` (`capabilities`). All as cited.

## Departures caught
- Safari will not open `wss://` through a click-through: caught. The plan files it as *Unresolved* rather than settled, but builds the page poll-first (`/api/v1/waiting` revision every five seconds, socket as the enhancement), which is what the reference asks.
- "Offer the certificate for download" is not an option: caught with a wrong fix. It rejects the download (a profile install under Settings, screens deep) and keeps the click-through, but then reissues the certificate with SAN, EKU and 825 days, which the reference says is not to be done. See decisions.
- Each file is a connection: caught. It names six connections per browser, raises the pools to 8 and 16, and serves the files on the connection thread before `try_send`. It does not say what the pool leaves for a second device beyond a follow-up, and it ships four files where the reference wants two.
- Device name is the page's to send: caught explicitly (host reads no `User-Agent`; the page posts `device_name` from `navigator.userAgent`, cut at 64 by `new_device`).
- Errata departures: the excerpt not on the wire, caught and the spec line corrected; scope unknown after a reload, caught (scope kept in `localStorage`, a 403 on `send` hides the box); the token in the pair body even with a cookie, caught (`credential: "cookie"` omits `token` from the body). The loopback-bind-with-LAN-invite mismatch is not addressed.
- Found beyond the reference: a browser fails the connection when it offered a `Sec-WebSocket-Protocol` that is not echoed (one more reason for the cookie); the dialog's address format drops IPv6 brackets; browsers request `/favicon.ico` and `/apple-touch-icon*.png` unasked, so the 404 must be cheap; 405 for a non-GET on a static path.

## Decisions to overturn
- The certificate reissue: new `Identity` fields, a `CertificateReissued` action, a reissue-at-bind policy keyed on `local_address()` and a 30-day expiry margin, plus the tests and docs around it. The reference says click-through only, no reissue, because it binds the certificate to an address and an expiry the spec's identity model avoids. I partly agree with the plan that the SPKI pin makes it safe for native clients, but the payoff rests on the plan's own *Unresolved* line about Safari, and the policy undercuts the dialog sentence it writes: a reissue on address change or expiry shows the very "second warning" the dialog tells the owner means an impostor. Reverse.
- The cookie named `switchboard_token` with no server id. Cookies ignore ports, so two instances on one Mac (the dev-aid test instance beside the real one) overwrite each other's token. The reference names the cookie with the server id. Reverse, one word.
- Not counted but noted: four files (`index.html`, `app.js`, `app.css`, `icon.svg`) against the reference's two; inlining the CSS under a hash and dropping the icon would halve the connections a page load holds.

## Over-specification
- The reissue machinery, if reversed, is roughly a fifth of the plan: the `Identity` fields, the schema bump, the action and effect paragraph, `make_certificate`/`reissue_certificate`, the bind-time policy prose, the adapter and core tests for it, the host-section line and the README `identity.json` line.
- The "Contract additions" section restates what the body already specified. The trust paragraph under decision 1 argues the same point three times. The DNS-SAN follow-up is a paragraph on something not being built. Together with the above, about a quarter of the plan could go without an implementer missing anything.

## Hand-off
- Drop the certificate reissue and everything that hangs off it (keep click-through plus the poll-first page as written), and name the cookie with the server id; the rest is implementable as it stands.

## Notes
- best: Every file and line it cites is real, and it found departures the issue missed (the excerpt is not on the wire, a non-echoed subprotocol fails the socket, the pair body would still carry the token), with the cookie design and the page states fully worked out.
- worst: It builds a certificate reissue policy, new identity fields and a new action on an admittedly unresolved claim about Safari, and that policy produces the re-warning its own dialog text tells the owner to treat as an impostor.