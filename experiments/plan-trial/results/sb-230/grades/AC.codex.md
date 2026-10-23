| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| AC | 1 | 2 | 0 | 1 | no |

## Grounding
- `web/index.html`: wrong (absent; explicitly proposed as a new file, not falsely described as existing).
- `web/app.js`: wrong (absent; proposed new file).
- `web/app.css`: wrong (absent; proposed new file).
- `facts.md`: found (the supplied text; supports the fragment and polling choices, but its certificate recommendations conflict with the owner-checked reference).
- `src/adapters/remote.rs:495–503`: found (splits the request target into path and query and collects query parameters).

## Departures caught
- Safari cannot reliably open the events socket after certificate click-through: caught; supplies polling, but its conditional content refresh can eventually stop detecting content changes.
- Certificate download is not viable with the existing certificate: caught with a wrong fix; adds SAN reissuance and export without addressing the reference’s EKU and validity requirements.
- Browser subresources consume separate unauthenticated slots: caught; closes static connections promptly. This mitigates retention, but three files and four slots leave simultaneous-device loading insufficiently addressed.
- Device naming belongs in the page: caught; uses `navigator.userAgent`.
- Additional departures: correctly identifies the missing excerpt, missing scope readback, loopback/LAN invite mismatch, indistinguishable unauthorized responses, and token exposure through the pairing response body.
- Additional observation: fragment pairing avoids sending the code in the initial request. I consider this a justified improvement over the reference’s query URL.

## Decisions to overturn
- Certificate reissuance and export conflict with reference decision 1: retain click-through and avoid changing certificate identity semantics.
- Fixed cookie name `__Host-sb` conflicts with decision 2: include the server id to prevent instances on different ports overwriting one another’s credentials.
- Three page files conflict with decision 5’s two-file limit. Closing connections helps, but does not establish adequate capacity for a second device.
- Omitting `watch` and polling content only after `/waiting` changes conflicts with decision 3. Recent reads do create temporary watches, so the reference’s categorical claim needs qualification; however, unchanged metadata can let those watches expire, after which content changes go undetected. Poll open content independently and register socket watches.

## Over-specification
- Roughly one third is removable: certificate machinery outside the reference’s scope, repeated contract inventories, schema and signal nonchanges, mechanical constructor instructions, and source-text policing tests. The unresolved-browser discussion also repeats across several sections.

## Hand-off
- No: certificate scope, cookie collisions, connection capacity, and sustained content refresh require substantive corrections before implementation.

## Notes
- best: Identifies several real protocol gaps beyond the original reference and keeps credentials out of page script.
- worst: Conditional polling undermines the Safari fallback that the plan correctly recognizes as essential.