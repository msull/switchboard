| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| v4-sonnet-a | 3 | 2 | 1 | 1 | note |

## Grounding
- `src/ports/events.rs:24`: found (`Stopped.last_message` is an optional event field).
- `src/adapters/hooks.rs:71`: found (`LogLine` carries `last_message`).
- `src/core/events.rs:132`: found (`Stopped { pending, .. }` updates stop metadata without retaining the message).
- `src/core/model.rs`: found (`SessionRecord` has no `last_message`; the file-wide search confirms its absence).
- `src/app.rs:1093`: found (`refresh_captions` refreshes conversations for transcript-based card captions).

## Departures caught
- Safari WebSocket failure after certificate click-through: caught; REST polling works independently, with socket watches as an upgrade.
- Certificate download requires changing the existing certificate: caught with a wrong fix; rejects download but nevertheless introduces certificate reissue, contrary to the reference.
- Parallel static fetches consume the four unauthenticated slots: caught with a wrong fix; closing connections helps, but three assets and assumed browser retries do not establish capacity for a second device.
- Device name must come from page JavaScript: caught; uses `navigator.userAgent` to populate `device_name`.
- Missing wire excerpt: caught; correctly rejects the facts file’s claim and uses existing reads, consistent with the reference erratum.
- Missing scope after reload, loopback-only listening, and token exposure in cookie pairing replies: caught; adds a device endpoint, dialog guidance, and token omission respectively.
- Additional departures beyond the reference: identifies indistinguishable unauthorized/revoked responses, the browser limitation of the relay guarantee, and existing frame-side rejection of static paths.

## Decisions to overturn
- Remove automatic certificate reissue, identity fields, schema change, and renewal machinery; reference decision 1 explicitly retains the existing certificate and uses click-through plus polling.
- Reduce the page to HTML and script and establish second-device pool capacity; reference decision 5 specifies two assets and permits increasing the unauthenticated pool to eight.

## Over-specification
- Roughly 40% is removable: certificate lifecycle machinery, repeated contract summaries, unchanged-layer sections, literal constructor inventories, and repeated unresolved-question and owner-follow-up discussions. The manual cookie gate also adds hand-off friction.

## Hand-off
- Keep the existing certificate, serve two assets, and resolve concurrent page-load capacity explicitly before implementation.

## Notes
- best: Corrects the supplied facts file’s nonexistent durable excerpt field while preserving the existing read model.
- worst: Turns certificate compatibility uncertainty into an unnecessary identity lifecycle change despite choosing click-through and polling.