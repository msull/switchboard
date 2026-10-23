# Feedback, round 1 (premises)

Checked against the worktree at `7fe8949`: every line reference the plan
cites in `src/app.rs`, `src/adapters/remote.rs`, `src/ports/remote.rs`,
`src/core/remote.rs`, `src/ui/remote.rs`, `src/script.rs`,
`control/src/lib.rs`, `src/ui/cards.rs`, `tests/remote.rs`,
`tests/ui.rs`, `src/core/tests.rs`, `docs/design.md`,
`docs/remote-client.md`, `README.md`, `client/README.md` and the Swift
files matches the tree as it is now; `IDENTITY_SCHEMA_VERSION` is 1
(`src/ports/remote.rs:17`); `if-addrs 0.15.0` is in `Cargo.lock:2365`
as `mdns-sd`'s dependency and `mdns-sd` is in the macOS table
(`Cargo.toml:72-77`); the request line's query is split from the path
at `src/adapters/remote.rs:495` and the frame logs `request.path` only
(`src/app.rs:2546`, `3765`), so "nothing logs the code" holds. The
cookie, the CSP, the pools, the QR size, the URL invite, the
`Sec-WebSocket-Protocol` refusal and the certificate's SAN/EKU/validity
each rest on a Settled line of the facts file, and the plan's two
defaults on Unresolved lines (poll behind the socket, pool of 8) work
whichever way those fall. Nothing in `src/core/model.rs` changes, the
contract is extended additively, no project dot-directory is written,
and the token goes to an `HttpOnly` cookie rather than script. One
objection.

## 1. The dialog's trust signal is defeated by the plan's own reissue policy

**The decision.** The pairing sentence (plan, "The UI") tells the owner:
"The browser warns about this Mac's certificate once; accepting it on
your own network is the trust step, and a second warning at this
address means another device answered." Decision 1 rests on this: "the
browser remembers the certificate it accepted, so a different
certificate at the same address warns again, which is the owner's
signal", and "after pairing the token is only as safe as the owner's
refusal to accept a second warning."

**The other decision.** "When." reissues the certificate from the
`Tick` pass once a minute whenever `certificate_names` does not contain
every address `local_addresses()` returns, and whenever
`certificate_expires` is within 30 days; `local_addresses()` is every
non-loopback, non-link-local address of every interface. On a laptop
that set moves whenever a VPN comes up (a `utun` address), a tethering
or second Wi-Fi network is joined, or a container bridge appears. Each
move is a new certificate at the same address.

**The premise, and what supports it.** The signal is real only if the
browser's click-through is keyed to the certificate, and it is: in
Chromium the exception is a map from host to `CertPolicy`, whose store
is `std::map<net::SHA256HashValue, net::Error> allowed_`, "the set of
fingerprints of allowed certificates"
(https://chromium.googlesource.com/chromium/src/+/HEAD/android_webview/browser/aw_ssl_host_state_delegate.h,
the same `CertPolicy` the content layer's `SSLHostStateDelegate::AllowCert(host, cert, error)`
is fed from,
https://chromium.googlesource.com/chromium/src/+/main/content/public/browser/ssl_host_state_delegate.h).
So the same premise that gives the owner a signal makes every reissue
produce that signal: the first VPN session after pairing shows the
warning the dialog just said means an impostor. Either the owner learns
to click through it, and the signal is gone, or the owner believes the
dialog and stops using the page. The plan does not say this anywhere;
it presents "reissue whenever the address set moves" as a default that
"works whichever way" the unresolved IP-SAN line falls, and it does for
the native pin (Settled), but not for the browser's trust step, which
is this plan's own addition. For Safari I could not settle whether the
exception is per certificate or per host (queries: `Safari iOS
self-signed certificate "visit this website" exception stored per
certificate new certificate warns again`, and the two under the facts'
first Settled claim; the HttpWatch guide
https://blog.httpwatch.com/2013 says only that it is a per-site
exception, not what it is keyed to). Chromium alone is enough to make
the sentence false for "another Mac" and "Chrome on Android", which
the plan's device names include.

**What the plan must decide.** One of:

- Keep the sentence and narrow the reissue so that the certificate at an
  address changes only when it must: reissue at bind (`bind_remote`,
  `src/app.rs:2936`) when the address the URL carries
  (`local_address()`, `src/adapters/remote.rs:962-971`) is not among
  `certificate_names` or the expiry is near, and never from `Tick`.
  Then the SAN names the addresses the owner was shown and the
  certificate is stable while the Mac keeps its address; a new address
  is a new origin anyway (a host-only cookie and `localStorage` do not
  follow it, facts line 22), so the browser pairs afresh there and sees
  one warning, which the sentence already covers.
- Or keep the policy and change the sentence to what is true: a second
  warning means either another device answered or this Mac reissued its
  certificate, and the dialog shows when it last reissued (the new
  `certificate_expires`, or a `certificate_issued` the record would
  need) so the owner can check before clicking. This weakens decision 1
  from "the owner's signal" to "a signal the owner can verify in the
  dialog", and the plan should say so under decision 1 rather than in
  the UI section.

Either way the follow-up "A DNS name in the SAN and the URL" is not a
follow-up but the thing that would make both decisions true at once,
and the plan should say why it is deferred.

**A cost that rides on the same decision and rests on nothing.** Every
reissue is `Effect::SaveIdentity`, which the plan has rebind the port
when the certificate differs (`run_remote_effect`,
`src/app.rs:2890-2915`); `bind_remote` starts with `self.remote = None`
(`src/app.rs:2937`), which drops the listener. The plan does not say
what that does to a paired native device's open WebSocket and kept
connections, and the once-a-minute trigger makes it happen on every
address move while a device is watching. If the first option above is
taken, the rebind happens only at bind and the question is moot; if the
second, the plan needs a line on it, with `RemotePort::drop`
(`src/adapters/remote.rs:751-767`) cited, or a swap of the
`ServerConfig` in `Shared` without a rebind.
