# 27-dnssd-exit

**Question.** When a process that registered a service through mDNSResponder
dies without deallocating its `DNSServiceRef` (SIGKILL, as in a crash), does
mDNSResponder drop the registration, or does it linger until the TTL?

`dns-sd -R` is a `DNSServiceRegister` client, so its death is the same event
for mDNSResponder as the host's. `dns-sd -B` is a second client of the same
responder, not a second responder.

## Command

```sh
./spikes/27-dnssd-exit/run.sh <dir for logs>
```

## Output (macOS 27.2, 2026-11-10)

```
== SIGTERM: killed at 1794335881.472325000
10:37:58.376  Add  ... sbspike-TERM   (three interfaces)
10:38:02.752  Rmv  ... sbspike-TERM
== SIGKILL: killed at 1794335889.503448000
10:38:06.491  Add  ... sbspike-KILL
10:38:10.703  Rmv  ... sbspike-KILL
```

Registered 4 s before each kill; the `Rmv` arrives at the kill (the browser
log stamps differ from `date` by a fixed offset of under a second).

## What it does not show

Browse is answered from the local cache, so this shows mDNSResponder
deregistered the record, not that a TTL-0 goodbye reached the wire. Capturing
that needs `tcpdump` as root on UDP 5353 and a second Mac; left to the owner.

## The app itself, before and after the change (2026-11-10)

`target/debug/switchboard` with a throwaway `SWITCHBOARD_DATA_DIR`, a
`switchboard-test-*` tmux socket and `SWITCHBOARD_SCRIPT` holding
`remote-listen lan 43271`; `dns-sd -B _switchboard._tcp local.` running
throughout, then `dns-sd -L "<instance>" _switchboard._tcp local` and
`dns-sd -G v4v6 <host>.local`, then SIGKILL and ten more seconds of browse.

Baseline, `mdns-sd`:

```
11:52:03.622  Add  2   1 local.  _switchboard._tcp.  sbgr-agentbox-01s-Mac-Mini (d5aa03)   <- SIGKILLed 19 s earlier
11:52:04.579  Add  2   1 local.  _switchboard._tcp.  sbgr-agentbox-01s-Mac-Mini (662986)
-L: ... can be reached at sbgr-agentbox-01s-Mac-Mini.local.:43271 (interface 1)
     v=1 name=sbgr-agentbox-01s-Mac-Mini id=...
SIGKILL at 11:52:23: no Rmv in the next 10 s
```

The record was seen by the system's responder on interface 1 (`lo0`)
only, and a killed instance stayed listed.

After, `DNSServiceRegister`:

```
11:55:20.774  Add  3   1 ... (95ab54)
11:55:20.774  Add  3   7 ... (95ab54)
11:55:20.774  Add  2  16 ... (95ab54)
-L: ... can be reached at sbgr-agentbox-01s-Mac-Mini.local.:43271 (interface 7)
     v=1 name=sbgr-agentbox-01s-Mac-Mini id=...
SIGKILL at 11:55:39
11:55:40.351  Rmv  1   1 ... (95ab54)
11:55:40.351  Rmv  1  16 ... (95ab54)
11:55:40.351  Rmv  0   7 ... (95ab54)
```

`dns-sd -G v4v6` for the host name answers from the Mac's own records in
both runs (the two LAN IPv4 addresses, `127.0.0.1`, `::1` and the
link-local IPv6 addresses), so the record's IPv6 answers are the same
before and after; the listener is IPv4-only either way.

Not run: the wire capture (`sudo tcpdump -i any -vv -n 'udp port
5353'`), since `sudo` asks for a password on the agent box, so whether a
TTL-0 goodbye leaves the Mac on quit or kill is not captured for either
build; and the Local Network prompt on a signed bundle, which would mean
installing over the owner's app. A clean quit was not exercised either:
the app is quit from its menu, which an agent may not drive.

## Recommendation

FFI registration is dropped by the system on SIGKILL as well as on clean exit,
so the host does not need a `Drop` path to withdraw on a crash.
