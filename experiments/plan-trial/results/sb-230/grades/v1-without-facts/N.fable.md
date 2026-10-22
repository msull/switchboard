| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| N | 3 | 2 | 0 | 1 | no |

## Grounding
- `src/core/model.rs:12` (`SCHEMA_VERSION` is 16): found (line 12 is `pub const SCHEMA_VERSION: u32 = 16;`)
- `src/ports/remote.rs:17` (`IDENTITY_SCHEMA_VERSION` is 1): found (line 17, exactly as stated)
- `src/ports/remote.rs:39-40` (`listen` added under version 1 with `#[serde(default)]`): found (lines 39-40 are `#[serde(default)] pub listen: Listen`)
- `src/core/remote.rs:64` (window closes on first pairing): found (the `Paired` arm at 64-70 sets `self.pairing = None` with the single-use comment)
- `src/ui/cards.rs:558-574` (`agent_excerpt` derives the excerpt UI-side): found (function spans 558-574 and does what the plan says)
- Spot checks beyond five: `app.rs:3142-3148` 401 refusal, `remote.rs:835-848` pool move, `:573`/`:785` idle timeout, `:701-705` accept-loop drop, `app.rs:2920`/`2936`/`2951-2956`, `action.rs:1257`, `ui/remote.rs:157-172`, `core/remote.rs:166`, `tests/ui.rs:7474`, `client/README.md:180-190`, the rcgen serial derivation at `certificate.rs:458-468`, and `time 0.3.55` in the lockfile all check out. Unusually well grounded.

## Departures caught
- WebSocket will not open from Safari: caught, hedged as "Unresolved" rather than stated, but the build does not depend on it: socket first, 5 s REST polling when the socket fails before `hello`, spike records the device result. Reference orders poll first, socket second; functionally the same page.
- Certificate download is not an option with this certificate: caught with a wrong fix. The plan sees the SAN/EKU/validity problem but answers it by issuing a second served leaf over the same key with the routed IP as a SAN and an 800-day expiry, kept in `identity.json` and reissued on address change. The reference says no reissue, and the plan's own fallback (a CA:TRUE certificate for download) is the option the reference rules out.
- Each file is a connection: caught. Counts three files plus `/info` as four stranger slots, adds a `page` rate window, moves cookie-bearing static loads to the paired pool, and names the 4 to 8 fallback. Ships three files where the reference says at most two, and only the spike decides whether a second device is locked out.
- Device name is the page's to send: caught (`navigator.userAgent` in the pair body, host unchanged).
- Found by the plan and in the errata: the excerpt is not on the wire (shows `waiting_reason`, `card`, first line of `notes`); the scope after a reload (new `GET /api/v1/device`); the pair reply carrying the token in its body (omitted when `cookie: true`); the cookie not being port-scoped (name carries the port).
- Found by the plan, not in the reference: a revoked and an unknown token share one 401, so "revoked" versus "identity changed" is decided page-side from stored `{server_id, spki_sha256}`; `screen` refuses a stopped session; the fragment rather than the query keeps the code out of the request target and `Referer`.
- Missed: the invite names a LAN address while the host may listen on loopback only.

## Decisions to overturn
- The served leaf: a second certificate with the IP as SAN, serverAuth EKU, 800-day validity, a new `Identity.served` field, `ServedCertificateMade`, a rebind on `OpenPairing` when the address moved, an injected `address_source`, a direct `time` dependency. Reference decision 1: click-through is the trust step, no reissue; the identity binds nothing to an address or a validity window (§5, §9). Safari still refuses `wss://` regardless.
- A browser pairs `read` whatever the window grants and is raised by hand. The issue says scope is chosen in the pairing window as today, and the acceptance path "send a reply from the phone" now needs an extra manual step on the Mac. Reference decision 6 keeps the scope from the pair reply.
- Static files answered on the frame, with a cookie-aware `authenticated` flag for public bytes. Reference decision 5: constants on the connection thread; a frame-side route for a constant file is the call to reverse.
- The `X-Switchboard-Page: 1` header required on every cookie request and on `/pair`, with a new `cross_origin` refusal. Reference decision 2: `SameSite=Strict` plus an `Origin` check is enough; a CSRF token is over-specification. The plan's own reasoning admits a same-origin GET carries no `Origin` and is accepted anyway, so the header buys nothing a browser-sent `Origin` on POSTs and upgrades does not. The plan's point that same-site ignores ports is correct, and the `Origin` check alone answers it.
- Not reversed: the fragment in place of the query. The issue said query, but the fragment is strictly better for the same page code and the CLI parse handles it.

## Over-specification
- The served-leaf machinery across The record, The signals, The actions, its three test groups and its docs lines is roughly a quarter of the plan and all of it goes with the reversal above.
- Every owner question carries an "if the owner says otherwise, the fallback is" paragraph, and `facts:NN` citations point at a file the implementer does not have. Together about another tenth.
- The CORS/Origin mode reasoning for `fetch`, the rcgen serial detail, and the iPad user-agent heuristic are more than an implementer needs. Roughly 35 to 40 percent could go.

## Hand-off
- No. The served leaf contradicts the spec's identity model and the read-only cap breaks the issue's acceptance flow. With those two stripped and the static files moved to the connection thread, the remainder (cookie named by port, fragment URL, `/device`, excerpt departure, polling fallback, test list, docs list) is a sound plan.

## Notes
- best: Exceptional grounding and real departures the reference missed: the shared 401, the stopped-session screen, the fragment keeping the code off the wire.
- worst: Answers "a phone may not accept this certificate" by building a reissued, address-bound, expiring leaf that §5 and §9 deliberately avoid, and then caps browsers at `read` to compensate.