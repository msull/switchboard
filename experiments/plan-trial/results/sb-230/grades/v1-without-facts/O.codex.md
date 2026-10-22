| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| O | 2 | 1 | 0 | 1 | no |

## Grounding
- `facts.md`: wrong (absent from the tree; repeated “Settled” and “Unresolved” claims cannot be checked).
- `src/adapters/remote.rs:444`: found (`read_request` parses requests and separates the query into the request sent to the frame).
- `docs/remote-client.md` §4.3: found (the `session.last_response` paragraph incorrectly describes `last_message` as session metadata).
- `src/ports/events.rs:24`: found (`Stopped` carries an optional `last_message`).
- `src/adapters/hooks.rs:71`: found (`LogLine` includes `last_message`).

## Departures caught
- Safari click-through does not enable the events socket: caught with a wrong fix (acknowledges possible failure but accepts an unusable browser instead of providing polling).
- Certificate download is not viable with the existing certificate: caught with a wrong fix (recognizes certificate deficiencies but adds reissuance, contrary to the reference’s unchanged identity model).
- Parallel static requests occupy the four unauthenticated slots: caught with a wrong fix (three files, unchanged pool, and conditional connection closing leave second-device capacity unsettled).
- Device naming belongs in page script: caught (`navigator.userAgent` supplies `device_name`; no host header parsing).
- Missing card excerpt, recorded in the reference errata: caught; lazy last-response reads and correcting the spec are appropriate.
- Scope discovery after reload, recorded in the errata: caught through `/me`.
- LAN invite while listening on loopback, recorded in the errata: caught through an accurate loopback URL and explanation.
- Pair response exposing the token despite cookie authentication: not applicable to the selected bearer design.
- Additional departure claimed: fragment invites avoid sending pairing values in the request target. This is a proposed contract change, not a discrepancy between the issue and existing code.

## Decisions to overturn
- Replace socket-only operation with polling plus optional WebSocket updates, as reference decision 3 requires.
- Remove certificate reissuance, address tracking, renewal and identity schema changes; reference decision 1 retains the certificate.
- Replace script-readable `localStorage` tokens and subprotocol authentication with the server-specific HttpOnly cookie in reference decision 2.
- Resolve static connection capacity before implementation: reference decision 5 chooses two files and permits eight unauthenticated slots; this plan leaves three files competing for four slots.
- Restore query-form QR generation under reference decision 4. The fragment’s request-target advantage is reasonable, but the reference settles the emitted URL contract explicitly.

## Over-specification
- Roughly 40% is removable: detailed certificate migration machinery, repeated contract summaries, speculative owner-only fallbacks, exhaustive lexical JavaScript checks, and a separate pool spike where the plan should settle the resource policy.

## Hand-off
- No: the plan explicitly accepts failure on the target Safari devices and requires several architectural reversals before implementation.

## Notes
- best: Finds the nonexistent excerpt field and provides a bounded solution using existing reads.
- worst: Treats Safari live-update failure as acceptable instead of implementing the reference’s polling fallback.