10. Mostly resolved: the README bullet no longer says "Copy invite" copies a URL, and "Copy link" is documented. One wording slip remains, raised as point 12.
11. Resolved. The cookie name is pinned to `__Host-sb-` plus `server_id.simple()`, 42 characters, built by one `cookie_name` helper, with tests for the wrong forms.

New point, created by the answer to 10:

12. The README bullet says "Copy invite" copies "the QR's JSON text". The QR now carries the URL, so that is false once this lands. Write it as "Copy invite copies the JSON invite (`server_id`, `spki_sha256`, `code`, `addresses`)", and say the QR and "Copy link" carry the URL.
