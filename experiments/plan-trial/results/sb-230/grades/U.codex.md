| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| U | 3 | 2 | 0 | 1 | no |

## Grounding
- `facts.md`: found (the supplied text contains the Settled, Unresolved and For the owner sections cited; checked against that text as instructed).
- `src/ui/cards.rs:176–180`: found (agent captions come from the conversation cache; other captions come from UI state).
- `cards.rs:558`: found (`agent_excerpt` derives an excerpt from the last conversation turn).
- `src/ports/events.rs:24`: found (`EventKind::Stopped` carries optional `last_message`).
- `docs/remote-client.md:207`: found (the spec incorrectly describes `last_message` as session metadata, exactly as the plan reports).

## Departures caught
- Safari rejects the clicked-through certificate for WebSockets: caught with a wrong fix. The plan treats this established limitation as a spike question, makes polling conditional, and postpones sockets entirely if it fails. Polling covers waiting and open content, not all collections.
- Certificate download is unavailable with the current certificate: caught with a wrong fix. It recognizes the missing SAN and chooses click-through, but omits the reference’s EKU and validity constraints and retains download as an unresolved fallback.
- Parallel page files consume unauthenticated slots: caught. Explicitly returning from `connection` fixes idle slot retention. However, three files and four slots leave second-device capacity unsettled.
- Device naming belongs in page script: caught (`navigator.userAgent` supplies `device_name`).
- Missing wire excerpt: caught, consistent with the reference’s erratum. Deferring the excerpt is not a reversal.
- Missing scope read after reload: caught; the additional authenticated device route addresses it.
- Indistinguishable revoked and rotated responses: caught, but the proposed socket-failure inference introduces a separate error.
- Headset paste parsing: correctly identifies an additional native path, but this does not establish that Bonjour plus typed-code pairing is absent; the tree supports both.
- LAN invite versus loopback listening: missed.
- Cookie pairing still returning a token body: not addressed; the plan chooses a different transport.

## Decisions to overturn
- Replace `localStorage` token persistence and subprotocol authentication with the reference’s opt-in, server-id-named `HttpOnly; Secure; SameSite=Strict` cookie, preserving native bearer headers.
- Make polling an unconditional working fallback across collections and open content, retaining sockets where available. The reference explicitly requires this rather than leaving Safari support behind an owner-run spike.
- Do not interpret socket failure or closure plus an unchanged `/info` key as revocation. A TLS failure or temporary disconnect would erase a valid token. Require an authenticated request’s 401 before applying the reference’s identity comparison.
- Reduce the page to two files and settle concurrent-device capacity, as the reference requires. Closing static connections is useful but does not settle peak parallel demand.
- Remove certificate download from the fallback choices unless explicitly proposing the certificate redesign the reference excludes.

## Over-specification
- Roughly one-third is removable: repeated contract inventories and implementation ordering, empty Signals and record-change sections, unrelated schema-version enumeration, exact UI gutter sizing, and numerous owner-call alternatives after nominal decisions. The detailed spike checklist also delegates decisions that the reference already settles.

## Hand-off
- no: authentication storage, Safari live updates, false revocation handling and connection capacity need substantive revision. The plan also omits explicit revision-based response application and the required test that embedded asset references are all served.

## Notes
- best: Strong code grounding exposes the missing excerpt and scope read, and correctly explains why `Connection: close` needs an actual connection-loop exit.
- worst: Every socket failure can become a false “revoked” state that deletes valid credentials, directly undermining the intended Safari experience.