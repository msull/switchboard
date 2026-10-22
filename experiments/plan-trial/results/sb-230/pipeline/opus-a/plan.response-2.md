# Response to review 2

1–7. Marked resolved. The plan did not change for these.

8. **`hostname -s` is not the name mDNS answers for.** Accepted.
   - **Where this departs from the issue** (the address-change bullet):
     - The invite host is now `<LocalHostName>.local`.
     - The bullet says why `hostname -s` is wrong: it gives the short
       `HostName` when one is set.
     - It also says the Bonjour record proves nothing, because it has an
       empty address list.
   - **The signals:**
     - `local_host_name()` runs `scutil --get LocalHostName` once, under
       `cfg(target_os = "macos")`, and keeps the result in a `OnceLock`.
     - It returns `None` off macOS, when the command fails, or when the
       result is not one DNS label. The invite and the SAN then fall back
       to the LAN IP.
     - `default_server_name` keeps `hostname -s` for the display name
       only.
   - **The tests** (the spike's `.local` item):
     - The spike records `HostName`, `LocalHostName` and `hostname -s`.
     - On a Mac where the names are equal, as this one is, it sets a
       different `HostName` for the run. It then checks that the invite
       still carries `LocalHostName` and loads, and restores the old
       value.
     - Without this step the spike would pass here and miss the problem.
   - **Docs in the same commit** (known gaps):
     - The plan leaves the Bonjour record's host at
       `src/adapters/bonjour.rs:41` as it is.
     - That host is `<hostname -s>.local.` with no addresses, so it
       misleads native discovery in the same case. The problem predates
       this issue.
     - Changing what the discovery record advertises is outside the web
       surface, so it is named as a gap that needs its own fix.
