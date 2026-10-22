| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| v4-haiku-a | 3 | 2 | 0 | 1 | no |

## Grounding
- `inputs/facts.md`: found (the supplied facts text, as instructed; absent from the tree).
- `docs/remote-client.md:207`: found (incorrectly describes `last_message` as session metadata).
- `src/ports/events.rs:24`: found (`Stopped` carries `last_message: Option<String>`).
- `src/adapters/hooks.rs:71`: found (the hook input has `last_message`; this location declares it rather than constructing the event).
- `src/core/events.rs:132`: found (`Stopped { pending, .. }` updates stop time and pending work, discarding `last_message`).

## Departures caught
- Safari’s socket fails after certificate click-through: caught; polling supports the page independently, with socket watches as an enhancement.
- Certificate download is not a viable alternative under the existing identity model: caught with a wrong fix; mandates certificate reissue and download rather than retaining click-through trust.
- Parallel static requests consume the four unauthenticated slots: caught with an incomplete fix; closing responses helps, but three page files and a deferred pool decision do not establish capacity for a second device.
- Device naming belongs in page script: caught; uses `navigator.userAgent`.
- Missing wire excerpt, added by the reference errata: caught; uses existing last-response reads and correctly rejects the facts file’s record claim.
- Scope unavailable after reload, added by the errata: caught; adds an authenticated device endpoint.
- Cookie pairing still exposing the token in the response body, added by the errata: caught; browser pairing omits it.
- LAN invitation versus loopback listener, added by the errata: missed.
- Additional findings: identifies indistinguishable authorization error codes, possible URL recording before script execution, and the browser limitation of the spec’s relay claim. These are useful qualifications; the literal-site count correction is housekeeping.

## Decisions to overturn
- Mandatory certificate reissue, identity schema migration, and renewal lifecycle conflict with reference decision 1’s explicit no-reissue choice.
- `/host.cer` and profile-install instructions conflict with reference decision 1’s click-through-only trust model and decision 5’s static allowlist.
- Three page files sharing four unauthenticated slots conflict with reference decisions 5–6’s two-file design and explicit second-device capacity requirement. `Connection: close` reduces occupancy but does not settle concurrent admission.

## Over-specification
- Roughly 40% can be cut: the unwanted certificate lifecycle, repeated contract and implementation inventories, speculative owner follow-ups, facts bookkeeping, and tests specifying incidental details such as an exact CSP string. The repeated descriptions add length without resolving the remaining admission problem.

## Hand-off
- no: remove the certificate migration/download branch and settle concurrent page-load capacity before implementation.

## Notes
- best: Correctly catches the nonexistent excerpt field and supplies polling that keeps Safari functional without WebSockets.
- worst: Turns certificate uncertainty into a substantial identity migration contrary to the reference’s chosen trust model.