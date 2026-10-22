# Response to plan feedback, round 2

Earlier points 1-6: settled; no change.

## Cuts

### Where this departs from the issue
- **The "later issue adds" sentence and the revoked-hashes sentence:** cut.
- **Facts correction:** cut.

### The record
- **Decision 6's `kind` field:** cut. The section now ends at "The certificate in `identity.json` is untouched."

### The signals
- **The whole section:** cut.

### The actions and effects
- **1, why not install the certificate:** cut. One line stays: "Chrome and Firefox apply the exception to the page's own socket (Settled, Chrome and Firefox lines)." It is the Settled citation the chosen trust step rests on, and the brief requires every decision to cite one.
- **1, the follow-up issue if `fetch` fails:** cut.
- **2, the restated contract line and "Why not the cookie":** cut.
- **2, the optional `Origin` check:** cut. The bullet now ends where it answers the For the owner line.
- **3, the `.local` alternative:** reduced to "IP, because `.local` resolution on the device is Unresolved."
- **5, the `/pair`-scope fallback and "Spec §4.3 lists it":** cut. The §4.3 row stays in Docs.

### The UI
- **The 9-key restatement:** replaced by a pointer to the excerpt departure.
- **The post-failure `GET device` restatement:** replaced by "handled as Actions 1 says."

### The tests
- **The `/other` and `subscribe_at` "still passes" lines and "Fixtures do not change":** cut.
