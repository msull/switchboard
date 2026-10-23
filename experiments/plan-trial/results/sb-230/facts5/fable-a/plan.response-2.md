# Response to feedback, round 2

## Earlier points

1. Noted as closed; nothing further changed on the reissue policy.

## Cuts

**The signals, decision 1**

- `local_addresses()`, `if-addrs`, the dialog's address line, the
  README and Contract entries: cut. The `local_addresses()` bullet
  under "Trust in the certificate" is now one line saying
  `local_address()` is the one address the URL and the certificate
  carry. "Pairing from the URL", the Addresses bullet, says the list is
  the enhancement if the owner's phone proves the one address short and
  points at follow-ups. "The UI" lists only the full URL beside the QR.
  "Docs and status" says no new dependency is documented. A follow-up
  "Listing every interface's address in the dialog" is added, noting
  `if-addrs` 0.15 is already in the lock file. The Contract entries for
  `local_addresses()` and `if-addrs` are gone.
- The `debug_assert_eq!` sentence and the three struct literals: cut
  from "The record" and from the `reissue_certificate` bullet.

**The page's files, decision 4**

- `PAGE_PER_MINUTE` and the `Limits::limited` prefix match: cut. The
  "Pools and windows" bullet now says the files get no rate window,
  why (static slices answered before `try_send`, the pool bounds the
  threads), and answers the issue's question as "the unauthenticated
  pool, and no window". The Contract line drops the constant.

**The token, decision 2**

- The `Origin`-against-`Host` check, `origin` and `host` on
  `RemoteRequest`, and the `Origin: https://elsewhere:1` assertion:
  cut. The `read_request` bullet keeps one header, the cookie token.
  The `authenticate` bullet says cross-site requests are covered by
  `SameSite=Strict` alone, citing the Settled line. "The actions and
  effects" and the browser-pairs test in "The tests" drop the check and
  the assertion; the §5 paragraph in "Docs and status" names the
  `SameSite` rule instead of an `Origin` check. The Contract line for
  `RemoteRequest` is `cookie_token` alone.
- `Set-Cookie … Max-Age=0` on a 401: cut. The `authenticate` bullet
  says a revoked browser's stale cookie rides until the next `/pair`
  overwrites it, at the cost of one ignored header, and the page clears
  its own `paired` flag. The revoke test is renamed
  `a_revoked_browser_is_refused_and_its_socket_closed` and asserts the
  401 and the closed socket only. The Contract wire line names
  `Set-Cookie` on the `/pair` reply alone.

**The tests**

- The `respond` and `web::file` unit tests: cut from "Adapter unit
  tests"; the page test in `tests/remote.rs` stands as the one that
  forces them.
- The `x509-parser` fallback sentence: cut; the rustls `RootCertStore`
  test is the test.
- The thirty-`Tick` negative case: cut from the reissue test, which
  keeps the bind-time positive and "a second start reissues nothing".

**Contract additions**

- `local_addresses()`, `origin`, `host`, `PAGE_PER_MINUTE`, the 401
  `Set-Cookie` and `if-addrs` are all removed from the list, as the
  re-read of that section confirms.
