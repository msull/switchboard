| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| N | 3 | 2 | 0 | 0 | no |

## Grounding
- `src/core/model.rs:12`: found (`SCHEMA_VERSION` is 16).
- `src/ports/remote.rs:17`: found (`IDENTITY_SCHEMA_VERSION` is 1).
- `src/ports/remote.rs:39-40`: found (`listen` has `#[serde(default)]`, supporting the additive-field precedent).
- `facts/1/repo/facts.md`: found in the supplied facts text, as instructed; absent from the checkout. Its browser uncertainties and proposed alternatives match the plan’s citations.
- `tests/remote.rs`: found (integration tests use a pinned Rust TLS client; they cannot establish browser fragment, history or certificate-override behavior).

## Departures caught
- Safari’s WebSocket limitation: caught, although treated as unresolved rather than established. REST fallback protects collection updates, but short-interval polling of the open session’s content is not clearly specified.
- Certificate download is not a viable choice with the existing certificate: caught with a wrong fix. The plan introduces certificate reissuance and retains a download fallback, contrary to the reference’s click-through-only decision.
- Parallel static fetches exhaust unauthenticated slots: caught with a wrong fix. Three files remain, and increasing the pool depends on a spike; capacity for a second unpaired device is not secured.
- Device naming belongs in page script: caught; uses `navigator.userAgent`.
- Missing card excerpt: caught; the replacement with existing fields is justified by the reference erratum.
- Missing scope after cookie-based reload: caught; `/device` supplies it.
- Cookie pairing would otherwise expose the token in its response body: caught; browser replies omit it.
- LAN invite versus loopback listener: missed; certificate-address handling does not enable LAN listening.
- Additional findings: identifies indistinguishable revoked/unknown tokens and unavailable screens for stopped sessions, with reasonable page states.

## Decisions to overturn
- Remove the served-certificate lifecycle, persistence, rebind machinery and added dependency; reference decision 1 explicitly chooses no reissue or download.
- Honor the pairing window’s scope instead of forcing browser devices to `read`; reference decision 6 preserves ordinary device pairing and the requested reply flow.
- Serve static constants on the connection thread; reference decision 5 explicitly rejects frame-side static routing.
- Reduce the page to two files and settle connection capacity before implementation; reference departure 3 requires room for another device.
- Use the specified query-form invitation rather than replacing the contract with a fragment. The fragment has a defensible privacy benefit, but conflicts with reference decision 4.
- Name the cookie by server identity rather than port, following reference decision 2.
- Explicitly watch Claude conversation content and poll open-session content during socket failure; reference decision 3 requires both. The plan expressly watches only non-Claude screens.

## Over-specification
- Roughly half is removable: the rejected certificate subsystem and its tests, repeated contract inventories, detailed field/function prescriptions, and numerous owner-dependent fallback designs. These obscure the smaller browser implementation and leave competing implementations in the hand-off.

## Hand-off
- No: it changes the acceptance flow, adds substantial certificate machinery the reference rejects, and leaves connection capacity and content refresh behavior insufficiently settled.

## Notes
- best: Finds the missing excerpt and reload-scope gaps, and specifies useful cookie/revocation integration tests.
- worst: Turns browser trust uncertainty into a certificate subsystem and forced read-only pairing, substantially expanding scope while weakening the requested flow.