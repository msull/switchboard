| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| AB | 3 | 2 | 0 | 1 | no |

## Grounding
- `facts/1/repo/facts.md`: found (the supplied facts text; absent from the checkout, as permitted by the grading instructions. The preceding `web/` is a proposed addition, not an existing-source citation).
- `Facts, Settled, line 17`: found (says fragments are excluded from request lines and referrers).
- `Facts, Unresolved, line 35`: found (history retention is unresolved; recommends assuming the invite can persist).
- `Facts, Unresolved, line 36`: found (iPad user-agent naming is uncertain; recommends an editable name).
- `Facts, Unresolved, lines 28–33`: found (conflicting Safari WebSocket evidence and an explicit polling fallback).

## Departures caught
- Safari WebSocket failure after click-through: caught operationally through polling and content watches, although described as unresolved rather than established.
- Certificate download is not viable with the existing certificate: caught with a wrong fix—introduces certificate reissue, address tracking and renewal against the reference’s explicit decision.
- Static files consume scarce unauthenticated connections: caught through immediate connection closure. This addresses prolonged occupancy, but three files and four slots do not guarantee capacity for a concurrent second device.
- Device naming belongs in page script: caught; the page supplies the user-agent-derived name.
- Missing wire excerpt, acknowledged in the reference errata: caught; bounded last-response reads and existing fields provide a reasonable alternative.
- Missing scope on reload, unreachable loopback invites, and token exposure in cookie-pair response bodies: caught; adds a device read, reachability guidance, and token omission.
- Additional findings: fragment privacy and same-site cross-port requests are explicitly addressed.

## Decisions to overturn
- Remove certificate reissue and its renewal/resolver machinery; reference decision 1 preserves the existing certificate and identity model.
- Restore query-form invites and their native parsing; reference decision 4 explicitly chooses `?code=…&key=…&id=…`. Fragment privacy is a defensible alternative, but it changes the requested contract.
- Remove mandatory confirmation before pairing; retain an automatic user-agent-derived name, consistent with the requested scan-to-pair flow. Naming uncertainty does not require blocking pairing.
- Reduce the page to two files or explicitly provide additional connection capacity; reference decision 5 specifies two files and permits eight unauthenticated slots. Closing connections is useful but does not reserve a slot during concurrent loads.

## Over-specification
- Roughly 40% is removable: certificate parsing, renewal, hot-swapping and associated tests dominate unnecessary work; repeated contract summaries, exact helper signatures and exhaustive mechanical test prescriptions add more.

## Hand-off
- No: certificate lifecycle expansion and changes to the invite and pairing flow need correction before implementation; concurrent page-load capacity also needs a defensible bound.

## Notes
- best: Polling, socket watches, cookie-only tokens and scope recovery form a coherent browser implementation.
- worst: An unresolved certificate concern becomes a substantial certificate lifecycle subsystem contrary to the reference’s chosen scope.