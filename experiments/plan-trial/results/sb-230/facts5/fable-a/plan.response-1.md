# Response to feedback, round 1

## 1. The dialog's trust signal is defeated by the plan's own reissue policy

Accepted; the plan takes the first option. The once-a-minute reissue
from `Tick` is gone, and the certificate changes at an address only at
bind, when it must. Edits, all in the review copy of the plan:

- **"The signals", "Trust in the certificate (decision 1)"**, the
  opening paragraph: after "the owner's refusal to accept a second
  warning", a new passage says the signal holds only if the host's own
  certificate at an address is stable, that Chromium keys the
  click-through to the certificate's fingerprint so every reissue would
  show the warning the dialog says means an impostor, and that the
  reissue policy therefore changes the certificate at an address only
  at bind and never while the port is up. The decision is stated where
  the decision is made, as asked, not only in the UI section.
- **The same section, `make_certificate` bullet**: the SANs are the one
  address the URL carries (`local_address()`, the outward route) plus
  loopback and `localhost`, not every interface's address, so a VPN, a
  tether or a container bridge coming up changes nothing.
- **The same section, "When"** is now "When: at bind only, never from
  `Tick`": reissue in `bind_remote` before `tls_config` when
  `certificate_names` lacks `local_address()` or the expiry is `None` or
  within 30 days. It says why one warning per address is what the
  sentence covers (a new address is a new origin; the host-only cookie
  and `localStorage` do not follow, citing the Settled cookie line),
  what a running Mac does when its address changes (serves the old
  certificate until the next bind; a browser at the new address warns
  once as for any new address), and restates the Unresolved IP-SAN line
  against this policy. It also answers the rebind cost directly: a
  reissue's `SaveIdentity` leaves `listen` unchanged, so
  `run_remote_effect` only re-advertises, no rebind rides on a reissue,
  no paired device's socket or kept connection is touched, and
  `remote_bound` stays `Option<Listen>`. The question of what dropping
  the listener does to open sockets is therefore moot, as the feedback
  says it is under this option.
- **The same section, `local_addresses()` bullet**: the list is for the
  dialog's text only, and the bullet says those addresses are not in the
  certificate, so a browser at one of them warns once at that address.
- **"The actions and effects"**: the `CertificateReissued` bullet no
  longer widens the rebind comparison; `run_remote_effect` is unchanged.
  `reissue_if_needed` is called from `bind_remote` alone, after the key
  is read and before `tls_config`; "Nothing runs on `Tick`."
- **"The UI"**: the pairing sentence now reads "warns about this Mac's
  certificate once per address … a second warning at the same address
  means another device answered", which is true under the new policy.
  The host section gains a line "Certificate issued for <addresses>,
  until <date>" from the two new identity fields, so the owner can check
  what the browser was shown before clicking.
- **"The tests"**: the reissue test is renamed to say "at bind" and
  gains the negative case: with the port up, a subscriber open and the
  stored names emptied behind the app's back, thirty `Tick`s change
  nothing and the subscriber's socket stays open.
- **"Follow-ups"**: the DNS-name item now says it is the thing that
  would make both decisions true at once, and why it is deferred rather
  than dropped: the URL would depend on the phone resolving `.local`
  over mDNS (iOS does, Android browsers often do not), the Linux build
  has no advertiser to take the name from (`adapters/bonjour.rs` is a
  stub there, README Layout line 384), and the owner's one-address
  question is still open.
- **"Contract additions"**: `local_addresses()` is marked as for the
  dialog's list.

Not changed: the two new `Identity` fields and the schema bump stand,
since the bind-time comparison still needs the names and the expiry
without an X.509 parser; and no `certificate_issued` field is added,
because the first option was taken and the expiry line in the dialog is
enough to show what the certificate is.
