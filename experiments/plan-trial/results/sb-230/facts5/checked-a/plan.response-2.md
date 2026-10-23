## Earlier points

1. Closed by the reviewer; nothing further changed.

## Cuts

- **`history.replaceState` bullet:** cut from "Where this departs from the issue". The UI section's "The code works once" keeps the consequence.
- **The record, last sentence (the bump rule):** cut. The paragraph now ends at the `device_by_token` lookup.
- **Step 3, the `RemoteReply` line numbers:** cut. Step 3 now says "every literal adds `set_cookie: None`".
- **The `pages` rate window:** cut everywhere: decision 4's pool-and-rate bullet (now says the 4-slot pool at accept, `:701`, already bounds an unpaired client and adds no window), its 429 test, the docs line in `docs/remote-client.md` §4.1, and the contract-additions line.
- **`HEAD` support:** cut. Decision 4 now answers any method other than `GET`, `HEAD` and `OPTIONS` included, with 405 and `Allow: GET`. Step 4 lost its head-only sentence, step 3's writer lost the head-only flag, and the `HEAD` length test is gone. The `remote_page` unit test now checks a `HEAD` gets 405 beside the `POST`.
- **"Allowlist test and `/info` test still pass untouched":** cut from the tests.
- **The O2/O3 owner bullet:** cut from "For the owner, after this lands".

Not run: this was an edit to the plan only; I checked each cut by grep and by re-reading the surrounding lines (and fixed a stray double period one cut left).
