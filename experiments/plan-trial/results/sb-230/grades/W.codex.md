| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| W | 3 | 2 | 0 | 1 | no |

## Grounding
- `control/src/lib.rs:637-688`: found (the complete `SessionView`; contains `card`, `waiting` and `waiting_reason`, but no excerpt).
- `docs/remote-client.md:201`: found (requires masking last-response and excerpt text before cutting).
- `docs/remote-client.md:207`: found (describes `last_response` and claims the excerpt is session metadata, confirming the documented mismatch).
- `src/app.rs:3754-3766`: found (`refuse` logs the method and path, without the query).
- `docs/remote-client.md:307-308`: found (defines the self-signed identity and SPKI pinning across certificate reissues).

## Departures caught
- Safari WebSocket failure after certificate click-through: caught operationally through polling, although labeled unresolved rather than established as in the reference.
- Certificate download cannot work with today's certificate: caught; chooses click-through without changing the identity.
- Parallel page resources exhaust unauthenticated slots: caught with a wrong fix; three retained connections leave insufficient room for a second device, and the test covers only one browser.
- Device naming belongs in page script: caught; explicitly uses `navigator.userAgent`.
- Missing wire excerpt, acknowledged in the reference errata: caught and reasonably deferred.
- Scope unavailable after cookie-backed reload, acknowledged in the errata: noticed but unresolved; “stored scope” contradicts the earlier storage description and does not recover subsequent scope changes.
- LAN invite versus loopback listener, acknowledged in the errata: missed; default-route/VPN discussion does not address the listener mismatch.
- Cookie pairing still exposing the token in the response body, acknowledged in the errata: caught; browser pairing omits it.
- Restored-tab Safari Strict-cookie WebSocket limitation: missed, although polling provides the reference's fallback.
- Additional finding: fragment invitations avoid transmitting the pairing code in HTTP requests; this is a proposed protocol departure, not a mismatch in existing code.

## Decisions to overturn
- Replace the fixed `switchboard_token` name with a server-id-qualified name, per reference decision 2; cookies ignore ports.
- Normalize cookie authentication into `bearer` in the adapter, per reference decision 2, instead of adding another request field and changing frame authentication.
- Restore query-form invitations, per reference decision 4. The fragment rationale is reasonable, but it deliberately changes the specified invitation contract.
- Reduce the page to two resources and account for a second device, per reference decision 5; the proposed four-slot arrangement does neither.
- Serve embedded constants on the connection thread, per reference decision 5, instead of routing static requests through the frame.
- Make polling start independently of WebSocket failure, per reference decision 3; the detailed algorithm waits for failure despite promising polling by default.

## Over-specification
- Roughly one-third is removable: repeated trust explanations, empty signals/actions sections, precise QR sizing, exhaustive mechanical edits and test names, speculative follow-ups, and contract additions repeating earlier decisions.

## Hand-off
- No: cookie collisions, static routing, connection capacity and the inconsistent polling/scope lifecycle need correction before implementation.

## Notes
- best: Identifies the missing excerpt accurately and preserves native pairing while keeping browser tokens inaccessible to script.
- worst: Its detailed connection-budget argument validates one browser while overlooking the second-device constraint.