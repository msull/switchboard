| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| X | 3 | 2 | 0 | 1 | no |

## Grounding
- `src/adapters/remote.rs:102-110`: found (empty certificate names, common name set, self-signing; no explicit EKU or validity overrides).
- `control/src/lib.rs:637-688`: found (`SessionView` includes state, waiting reason and timestamps, but no excerpt).
- `docs/remote-client.md:207`: found (incorrectly describes `last_message` as session metadata).
- `src/core/model.rs`: found (durable model; no `last_message` field).
- `src/ui/cards.rs:558`: found (`agent_excerpt` derives text from the conversation’s last turn).

## Departures caught
- Safari’s WebSocket failure after certificate click-through: caught (polling keeps the page usable, although the plan calls the limitation unresolved).
- Certificate download is not a viable alternative with the existing certificate: caught with a wrong fix (rejects download but mandates certificate reissue, contrary to the reference).
- Parallel page resources consume scarce unauthenticated slots: caught with a wrong fix (raises the pool but serves four files and defers second-browser contention).
- Device name must come from page script: caught (`navigator.userAgent` supplies `device_name`).
- Missing card excerpt, acknowledged in the reference errata: caught; substituting state and waiting reason is justified.
- Scope unavailable after reload, acknowledged in the errata: caught; stores nonsecret scope metadata and handles command refusal.
- Cookie pairing would otherwise expose the token in JSON, acknowledged in the errata: caught; cookie-mode replies omit it.
- LAN invite versus loopback listener, acknowledged in the errata: missed.

## Decisions to overturn
- Remove certificate reissue, identity fields, migration and action machinery: reference decision 1 retains the certificate and uses click-through plus polling. Preserving native pins does not justify this additional certificate lifecycle.
- Name the cookie with the server ID: reference decision 2 prevents different ports on one host from overwriting each other’s credentials.
- Reduce the page to HTML and script and explicitly reserve capacity for another browser: reference decision 5 and departure 3 require this; eight slots alone do not settle contention.
- Replace the dialog’s claim that a second warning means another device answered: the plan’s own expiry-driven reissue can cause that warning legitimately, conflicting with reference decision 1’s accurate trust explanation.

## Over-specification
- Roughly 40–45% is expendable: certificate lifecycle machinery and its tests, repeated action/adapter/contract inventories, exact QR sizing, and extensive repetition of existing behavior. The documentation also invents a “page window of 60” after explicitly choosing no page rate window.

## Hand-off
- Substantial revision is needed: certificate policy, cookie isolation and resource budgeting require coordinated corrections, beyond a single guidance note.

## Notes
- best: Finds the nonexistent excerpt field and preserves cookie secrecy without changing native pairing replies.
- worst: Turns questionable certificate advice from the facts file into extensive mandatory implementation machinery.