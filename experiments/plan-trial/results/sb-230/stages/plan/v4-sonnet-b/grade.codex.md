| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| v4-sonnet-b | 3 | 2 | 1 | 1 | note |

## Grounding
- `src/core/model.rs:12`: found (`SCHEMA_VERSION = 16`).
- `:612`: found (`VIEWS_SCHEMA_VERSION = 7` in `src/core/model.rs`).
- `src/ports/remote.rs:17`: found (`IDENTITY_SCHEMA_VERSION = 1`).
- `control/src/lib.rs:636-688`: found (`SessionView` includes card, waiting fields and timestamps, but no excerpt).
- `src/bin/switchboard-hook.rs:37`: found (maps `last_assistant_message` to `last_message`).

## Departures caught
- Safari rejects the click-through certificate for WebSockets: caught; polling provides the fallback, with content watches when the socket works.
- Certificate download is not viable with the existing certificate: caught with a wrong fix; rejects download but introduces the reissue the reference explicitly excludes.
- Parallel static requests consume unauthenticated slots: caught with a wrong fix against the reference; three files retain the four-slot pool. Closing connections mitigates idle occupancy, but the plan does not establish capacity for a second simultaneous device.
- Device naming belongs in page JavaScript: caught; uses `navigator.userAgent` and the existing `device_name`.
- Missing wire excerpt, acknowledged in the reference errata: caught; uses existing last-response reads without exposing raw record text.
- Missing scope after reload, acknowledged in the errata: caught; adds an authenticated device read.
- LAN invite while listening on loopback, acknowledged in the errata: missed.
- Token remaining in the cookie pairing response body, acknowledged in the errata: caught; cookie-mode replies omit it.
- Additional findings: indistinguishable revoked/missing-cookie responses, frame-side handling of stray paths, and limits of the history-clearing promise.

## Decisions to overturn
- Remove certificate reissuance, identity fields, renewal actions and address-dependent certificate lifecycle; reference decision 1 explicitly chooses click-through without reissue.
- Fold CSS into HTML under an appropriate CSP hash and serve two files; reference decision 5 and departure 3 require a smaller load footprint and explicit consideration of the second device.

## Over-specification
- Roughly one-third is removable: speculative certificate machinery, repeated contract and implementation inventories, facts-file uncertainty bookkeeping, and detailed mechanical edits and test enumeration.

## Hand-off
- Remove certificate lifecycle changes and reduce the page to two files with an explicit simultaneous-device pool check before implementation.

## Notes
- best: Polling fallback, cookie authentication and content watches address the browser’s actual protocol constraints.
- worst: Speculative certificate reissuance becomes a substantial persistence and lifecycle change despite the reference’s explicit rejection.