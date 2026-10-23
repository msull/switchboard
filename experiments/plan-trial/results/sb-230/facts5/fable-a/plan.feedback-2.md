# Feedback, round 2 (cuts)

## Earlier points

1. Closed: the reissue now happens at bind alone, the SAN is the one address the URL carries, the sentence says "once per address", and `run_remote_effect` at `src/app.rs:2890-2915` does re-advertise without a rebind when `listen` is unchanged, as the response says, so the rebind cost is moot.

## Cuts, by section

**The signals, decision 1**

- `local_addresses()`, the `if-addrs` dependency, the dialog's address line, its README and Contract entries. The facts file puts "does the one address route to the phone" under "For the owner" and says the list is needed only "if no"; the plan builds that enhancement in front of its default, and the planner's own second answer says the extra addresses are not in the certificate, so a browser at one of them is a new origin with its own warning and pairing anyway. Ship the one address; add the list when the owner's phone proves it short.
- The sentence moving the `debug_assert_eq!` and the three struct literals to update: the compiler forces both.

**The page's files, decision 4**

- `PAGE_PER_MINUTE` and the prefix match in `Limits::limited`. The files are static slices answered on the connection thread before `try_send`, so a flood never reaches the frame or the inbox; the unauthenticated pool already bounds concurrency, and 60 a minute at four files a load is fifteen reloads before a legitimate owner sees 429. The issue's "which pool and rate window" is answered "the unauthenticated pool; no window, nothing past the thread is spent".

**The token, decision 2**

- The `Origin`-against-`Host` check, and with it `origin` and `host` on `RemoteRequest` and the `Origin: https://elsewhere:1` assertion. The facts settle that `SameSite=Strict` is sent on same-site requests only, which is the whole cross-site case; the plan calls the check "a second belt", which is the definition of a second mechanism where the first suffices. The one Safari caveat in the facts (Strict cookies missing from restored tabs) is a cookie not sent, which an `Origin` check does not touch.
- `Set-Cookie … Max-Age=0` on a 401 for a dead cookie, and its Contract line and test assertion. The page already clears `paired` on a 401 and shows "pair again", and the next `/pair` overwrites the cookie; a stale cookie on every request until then costs one header the host ignores.

**The tests**

- The `respond` unit test and the `web::file` unit test: the page test in `tests/remote.rs` already asserts every file's status, content type and headers and `/x` 404, so the suite forces nothing more.
- The `x509-parser` fallback sentence. rustls accepts a self-signed certificate as its own anchor through `RootCertStore` (rcgen's README example and rustls's own tests connect exactly this way), so the case is ruled out; the test is the rustls one.
- The negative case "thirty `Tick`s change nothing and the subscriber's socket stays open": nothing in the plan runs on `Tick` any more, so this tests the absence of code that is not written. The bind-time positive and "a second start reissues nothing" cover the policy.

**Contract additions**

- Drop `local_addresses()`, `origin`, `host`, `PAGE_PER_MINUTE`, the 401 `Set-Cookie` and `if-addrs` with the cuts above.
