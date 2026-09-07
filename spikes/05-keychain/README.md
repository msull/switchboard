# Spike 5: secrets in the macOS Keychain (2026-09-06)

**Question.** Can Switchboard keep secret environment values in the login
Keychain and read them back after a rebuild without a prompt, given the
app is signed with a self-made "Code Signing" identity (not Developer ID)?

**Method.** `src/main.rs` stores, reads, and deletes generic-password items
(service `com.sadburger.switchboard.spike`) through the `security-framework`
crate. `run.sh` copies the binary, signs the copies in different ways, and
probes a `get` under a timeout: a Keychain prompt blocks the call, so a
missing result inside 8 s means "prompted".

**Results.**

| Reader | Result |
| --- | --- |
| The ad-hoc signed binary that created the item | no prompt |
| A different ad-hoc signed copy | prompt |
| A **different build** signed with the same identity and identifier (`codesign -s "Prompt Box Dev" -i com.sadburger.switchboard.spike`; CDHashes differ) | **no prompt** |
| Same identity, different identifier | prompt |

The item's ACL (from `security dump-keychain -a`) explains it: the creating
app is recorded by its designated requirement, `identifier "..." and
certificate leaf = H"..."`, not by a build hash.

**Answer.** Yes. Because `scripts/bundle.sh` signs the bundle with a stable
identity and `CFBundleIdentifier` `com.sadburger.switchboard`, every rebuild
reads its own items silently. Consequences for the product:

- Items go under one service name (`com.sadburger.switchboard`) so they
  are auditable in Keychain Access; the account is `global/<NAME>` or
  `project/<project id>/<NAME>`.
- An ad-hoc signed build (no identity in Keychain Access) prompts once per
  rebuild per item; the bundle script already warns about that case.
- A missing item is `errSecItemNotFound` (-25300); the adapter reports it
  as "absent", every other error as a failure.
- Values never touch argv: the crate calls `SecItemAdd`/`SecItemCopyMatching`
  directly. Do not shell out to `security(1)` for writes.
