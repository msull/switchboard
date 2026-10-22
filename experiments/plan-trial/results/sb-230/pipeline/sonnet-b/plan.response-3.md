# Response to feedback 3

Only `plan.md` in this directory was edited. Points 10 and 11 were marked resolved, so I changed nothing for them.

12. **The README bullet called the JSON "the QR's text".** You're right: the QR carries the URL once this lands, so that was false.
    - *Where it changed:* "Docs, in the same commit", the `README.md` bullet.
    - **README wording:** it now says "Copy invite" copies the JSON invite (`server_id`, `spki_sha256`, `code`, `addresses`), and the QR and "Copy link" carry the URL. What either button copies pairs with `switchboard-remote pair "$(pbpaste)"`.
    - **Existing README sentence:** the bullet names the sentence that goes false (the dialog's "Copy invite" "puts the QR code's text on the clipboard", `:275-277`) and says it is rewritten.
    - **Same slip in the code:** I checked `src/ui/remote.rs` and found the "Copy invite" hover text ("The QR code's text, to paste into a client", `:202`) and the comment above it (`:199-200`). They would also go false. The bullet now lists both, to become "The JSON invite, to paste into a client".
