Revised `plan.md` as asked. Nothing was compiled or run; this is a plan edit. I did not open `facts.md`, because the sandbox only allows reads inside the worktree.

**Dropped**
- **Reissue:** the IP SAN work, `CertificateReissued` and its mirror action, the reissue logic in `bind_remote`, `issue_certificate` and `certificate_ip_names`. The "one existing field changes value" paragraph in The record is now a statement that `identity.json`'s `certificate` is untouched.
- **Export:** `ExportCertificate` (action and effect), `save_certificate_file`, the AirDrop and `switchboard-host.cer` text, and the dialog's "Reveal certificate file" button. The README `.cer` entry is gone from the docs step.
- **Tests and steps:** the core, adapter and UI tests for these, and the two order-of-work steps for the certificate. Steps are renumbered.
- **Dialog:** the sentence now says proceeding past the warning is the trust step and promises nothing about which browsers allow it. The Certificate section is one short paragraph saying "no change".
- **Spike:** it now uses today's certificate shape (P-256, no SAN) instead of one with an IP SAN. That makes the cookie test, Unresolved 3, run on the certificate users will actually get.

**Cookie name**
- **Name:** the cookie is `__Host-sb-<server_id>`, with the `server_id` taken from the same `/pair` reply. I checked the tree: `src/app.rs:3663` already puts it in the reply, as the identity's hyphenated UUID, so it is a valid cookie name.
- **Host side:** the host sets and reads only its own name, matched exactly in the `Cookie` header, and ignores the rest. The page never needs to know the name because the cookie is `HttpOnly`.
- **Key rotation:** `make_identity` keeps `server_id` (`src/adapters/remote.rs:93–100`), so a rotation does not orphan the cookie.
- **Tests:** two hosts each get a differently named cookie. A request carrying both cookies authenticates as each host's own device, and a request carrying only the other host's cookie gets 401. The old fixed name authenticates nothing.
- **Spike:** it now includes a second server on another port to confirm neither cookie overwrites the other.

**Contract additions** are now: no new `AppAction`, `Effect` or port method; `RemoteReply.headers` and `RemoteReply::new`; `RemoteRequest.{cookie, origin, host}`; `core::remote::invite_url`; and the wire items. The wire items are `GET /api/v1/me`, the `/pair` `transport` field, the cookie as a credential, the 403 `origin` code, and the three page paths.

**Left for the owner**
- The two certificate bullets are replaced by one: a trusted-certificate path, to be its own issue.
- It names the options: IP SANs (which Chrome needs), getting a file onto the device, or a local CA.
- It is tied to Unresolved 1 and 2: no click-through on the phone, an exception that does not persist, or iOS needing a trusted certificate for `wss://`.
- The page, the polling and the acceptance steps assume only that the browser has been let through. A browser that offers no way past the warning cannot load the page at all, and the plan says that case belongs to the follow-up.
- The cookie-fallback bullet is unchanged.
