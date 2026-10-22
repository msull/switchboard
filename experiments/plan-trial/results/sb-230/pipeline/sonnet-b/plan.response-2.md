# Response to feedback 2

Only `plan.md` in this directory was edited. Points 1 to 9 were marked resolved, so I changed nothing for them.

10. **The README bullet contradicted Decision 3.** Agreed, and fixed.
    - *Where it changed:* "Docs, in the same commit", the `README.md` bullet.
    - The dev-aid paragraph now keeps saying "Copy invite" puts the QR's JSON text on the clipboard, which it still does. It adds that "Copy link" copies the URL the QR now carries.
    - `switchboard-remote pair "$(pbpaste)"` pairs from either.
    - The CHANGELOG line is unchanged, as you said.

11. **The cookie name's form was ambiguous.** You're right: `server_id` is a `Uuid`, and its `Display` is the 36-character hyphenated form, not 32 hex. I pinned one form.
    - *Where it changed:* "Decision 2", "Port changes" (the `RemoteRequest` bullet), "The tests" and "Contract additions".
    - **The form:** the prefix `__Host-sb-` plus `server_id.simple()`, which is 32 lowercase hex digits. The name is always 42 characters. The hyphenated form stays in `/info`, the JSON invite and the URL's `id=`, and is never used in the cookie.
    - **One helper:** `remote::cookie_name(&Uuid)` builds the name. `Set-Cookie`, `read_request`'s filter and `authenticate` all use it or its prefix constant, so the three cannot drift.
    - **The cap:** `read_request` keeps only cookies named exactly 42 characters that start `__Host-sb-` (at most four). Values stay capped at 64 bytes, the token being 43.
    - **Tests:** the pairing test asserts the name is exactly `cookie_name(server_id)`, with 32 hex and no hyphen. The two-hosts test asserts the two names differ and are both 42 characters. It also asserts a cookie with the hyphenated id, or any other length, is not read and gives 401.
    - **Contract:** the line in "Contract additions" now states the form.
