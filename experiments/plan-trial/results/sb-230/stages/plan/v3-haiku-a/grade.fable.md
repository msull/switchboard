| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| v3-haiku-a | 3 | 3 | 1 | 2 | note |

## Grounding
- `docs/remote-client.md:207`: found (the `last_response` bullet, ending with "that is `last_message` from the `Stop` hook on the record, a field of the session's metadata")
- `src/ports/events.rs:23-24`: found (`EventKind::Stopped { last_message: Option<String>, pending }`; `last_message` lives on the hook event, and grep confirms no such field in `src/core/model.rs`, so the plan's correction of the facts file is right)
- `src/core/events.rs:132`: found (the `Stopped` arm binds only `pending` into `pending_at_stop`; `last_message` is dropped)
- `src/ui/cards.rs:177-181`: found (`conversation.and_then(agent_excerpt)` computes the agent card's caption from the parsed transcript)
- `src/app.rs:3568-3599`: found (`remote_info`, ends in `self.envelope(200, &body, false)`, so unauthenticated and with no device scope)

Spot checks beyond the five also held: `authenticate` at 3127, the `spki_sha256` body check at 3636, `PAIRING_WINDOW`/`PAIRING_ATTEMPTS` at `action.rs:972-973`, the single-use `Paired` arm at `remote.rs:65-71`, the no-SAN certificate at `adapters/remote.rs:101-110` with the SPKI hash at 112, `SCHEMA_VERSION` 16, `VIEWS_SCHEMA_VERSION` 7, `IDENTITY_SCHEMA_VERSION` 1, `new_device` at `core/remote.rs:166-175`, `run_remote_effect`/`SaveIdentity` at `app.rs:680`/`2893`, the test names at `tests/remote.rs:426,658,682,1406,1494`, `PairingInvite.parse` at `Info.swift:114-116`.

## Departures caught
- WebSocket will not open from Safari: caught (departure 7; polling is the default, socket is an upgrade that refetches on `changed`). It does not send `watch` for the open session's content over the socket, but the 3 s poll covers that.
- Certificate download is not an option: caught (click-through is the trust step, download not built, with the right reason). But the plan then reissues the certificate anyway, which is scored under decisions.
- Each page file is a connection: caught with a different fix (two files, CSS inlined, `Connection: close` so a load does not hold a slot for 30 s, a test with three idle strangers). It does not raise `MAX_UNAUTHENTICATED`.
- Device name is the page's to send: caught (`device_name` from `navigator.userAgent`, no host parsing).
- Beyond the reference: the excerpt is not on the wire (departure 1, and the plan corrected the facts file's claim that `last_message` is on the record, which the tree confirms); no endpoint tells a device its scope after reload (departure 2, adds `GET /api/v1/device`); revoked and never-paired share one 401 code (departure 3); the URL key does not protect a page served by an impostor (departure 4); `replaceState` cannot promise the code never hit history (departure 5); `local_address()` is a route source address, not necessarily what a phone reaches (partial on the loopback point); `crypto.randomUUID` needs a secure context so the op id uses `getRandomValues`.
- Missed: the pair reply still carries the token in its body even when the cookie is set.

## Decisions to overturn
- Reissuing the certificate with address and `.local` SANs, `serverAuth` EKU and an 825-day validity, driven by two new actions, a new effect, two new `Identity` fields and a 365-day renewal rule. The reference keeps the identity unbound from address and expiry (spec §5, §9), and reissue does not make Safari open `wss://` anyway. Reverse the whole mechanism.
- `localStorage` as the page's token store by default, with the cookie landed on the host but gated behind a `COOKIE_ONLY` flag in the script, and the `bearer.<token>` subprotocol as the socket transport that follows from it. The reference makes the `HttpOnly; Secure; SameSite=Strict` cookie the store, with `read_request` filling `bearer` from it. Reverse.

## Over-specification
- The certificate reissue block and everything downstream of it (identity fields, `certificate_params`, `reissue_certificate`, `CertificateNamesSeen`, `IdentityReissued`, `ReissueCertificate`, their core and adapter tests, the design.md "to run" list of owner items) is roughly a fifth of the plan and would be cut outright.
- Three token transports where one is needed, the `COOKIE_ONLY` toggle, the `RemoteReply.cookie` site count and the "count at edit time" aside, and the facts-file corrections about grep counts add another several percent. About a quarter to a third of the plan in total.

## Hand-off
- Drop the certificate reissue and its core machinery, and make the cookie the token store instead of `localStorage` with the subprotocol; the rest stands.

## Notes
- best: Found and verified that the card excerpt is not on the wire, against the facts file it was handed, and still shipped a working list from `card` and `waiting_reason`.
- worst: Builds a certificate reissue pipeline across identity, core, effects and tests that the spec's identity model argues against and that fixes nothing for Safari's `wss://`.