| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| v3-sonnet-a | 3 | 2 | 0 | 1 | no |

## Grounding
- `inputs/facts.md`: found (checked against the supplied text as instructed; its Settled, Unresolved and owner-question sections match the citations).
- `control/src/lib.rs:636-688`: found (`SessionView` includes `card`, `waiting` and `waiting_reason`, but no excerpt).
- `§4.3 (the session.last_response bullet)`: found (`docs/remote-client.md:207` describes `last_message` as session metadata updated by `changed {session}`, despite its absence from the wire).
- `src/bin/switchboard-hook.rs:37`: found (maps `last_assistant_message` to `last_message`).
- `src/app.rs:3568`: found (`remote_info` returns host identity, capabilities and limits, without device scope).

## Departures caught
- Safari WebSocket failure after certificate click-through: caught; polling supports acceptance independently, with socket watches when available.
- Certificate download is unsuitable with the existing certificate: caught with a wrong fix; rejects download but introduces certificate reissuance contrary to the reference.
- Parallel page requests consume unauthenticated slots: caught with a wrong fix against the reference; closing static connections helps, but three files exceed its two-file decision and second-device capacity remains unresolved.
- Device naming belongs in page script: caught; uses `navigator.userAgent` and sends `device_name`.
- Missing wire excerpt, acknowledged by the reference errata: caught; waiting-session last-response reads are an accepted workaround.
- Missing scope after reload, acknowledged by the errata: caught; adds an authenticated device endpoint.
- Additional findings: identifies indistinguishable unauthorized error codes and frame-side rejection of static paths. Its browser relay analysis is useful, but later trust guidance contradicts it.

## Decisions to overturn
- Remove certificate renewal, identity fields and associated actions/effects; reference decision 1 explicitly retains the existing certificate and identity lifecycle.
- Use the HttpOnly cookie consistently, removing localStorage and transport probing; reference decision 2 rejects localStorage. Requiring a successful socket probe unnecessarily sends Safari polling users into token mode.
- Reduce static assets to HTML plus script; reference decision 5 specifies two files to limit unauthenticated connection pressure.
- Correct the claim that comparing a key displayed by the page authenticates an impostor-served page; reference decision 1 makes click-through the trust step and explicitly limits the URL-key check to a genuine page.

## Over-specification
- Roughly two-fifths is avoidable: certificate lifecycle machinery, three-transport probing and tests, repeated contract inventories, mechanical edit instructions and duplicated implementation ordering. These expand a small browser surface into identity and authentication infrastructure work.

## Hand-off
- no: certificate policy, token storage and misleading trust guidance need substantive revision before implementation.

## Notes
- best: Polling-first updates and explicit content watches handle the central Safari limitation.
- worst: Browser uncertainty drives unnecessary certificate renewal and a localStorage fallback precisely where polling should suffice.