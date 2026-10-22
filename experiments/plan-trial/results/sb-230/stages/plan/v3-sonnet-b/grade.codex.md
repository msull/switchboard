| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| v3-sonnet-b | 3 | 2 | 0 | 1 | no |

## Grounding
- `facts.md` with its section: found (supplied text; its session-record claim is wrong, as the plan correctly identifies).
- `src/ports/events.rs:24`: found (`Stopped` carries `last_message: Option<String>`).
- `src/core/events.rs:132`: found (`Stopped { pending, .. }` retains pending information and ignores the message).
- `control/src/lib.rs:637-688`: found (`SessionView` exposes `card`, `waiting`, and `waiting_reason`, but no excerpt).
- `src/app.rs:3568`: found (`remote_info` returns public host information without the paired device’s scope).

## Departures caught
- Safari’s click-through does not enable WebSockets: caught operationally through polling, although the plan treats the documented limitation as unresolved.
- Certificate download requires an incompatible certificate redesign: caught with a wrong fix—rejects download but introduces the reissue machinery the reference rejects.
- Parallel page resources consume the four unauthenticated slots: caught with an incomplete fix—closing connections reduces occupancy, but three files and deferred device testing leave no demonstrated allowance for a second device.
- Device naming belongs in page script: caught; the page sends its user-agent-derived name.
- Missing excerpt on the wire: caught, including the erroneous facts-file claim; displaying existing fields agrees with the reference erratum.
- Scope unavailable after reload: caught; persists pairing metadata and acknowledges stale scope.
- LAN invite versus loopback listener: missed.
- Cookie pairing still exposes the token in the response body: missed as a problem; explicitly preserves that exposure.
- Additional findings: identifies indistinguishable unauthorized states, frame-inbox consumption by rejected paths, the JSON-dependent UI test, and the limits of a genuine page’s key comparison against an impostor-served page.

## Decisions to overturn
- Remove certificate reissue, identity fields, schema migration, and renewal actions; reference decision 1 keeps the existing certificate and click-through trust.
- Make the HttpOnly cookie the browser transport immediately; reference decision 2 rejects localStorage token storage and does not require competing browser transports.
- Reduce the page to HTML and script and settle capacity for another device; reference decision 5 specifies two files and permits increasing the unauthenticated pool.

## Over-specification
- Roughly 35–45% is avoidable: certificate lifecycle machinery, alternate transport plumbing, associated tests, repeated contract inventories, and repeated owner-dependent branches. These obscure an otherwise concrete implementation.

## Hand-off
- No: reverse the certificate and token decisions and resolve the static-resource connection budget before implementation.

## Notes
- best: Correctly traces the missing excerpt through the event, core, and wire types rather than trusting the supplied facts.
- worst: Converts browser uncertainty into certificate migration and three authentication transports instead of settling the reference’s simpler design.