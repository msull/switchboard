## Earlier points

1. Closed. The certificate is now persisted in `web-cert.json` with its own schema version, written through the atomic `save`, and `Identity` is untouched. The spike checks that a restart at the same address shows no warning, and the help text and docs say when the warning comes back.

## Cuts

- **Where this departs from the issue, the `history.replaceState` bullet:** it restates facts U4 and changes nothing an implementer builds. The UI section's "The code works once" already carries the consequence.
- **The record, last sentence** ("If the implementer finds a field has to be added… the bump is…"): this is CLAUDE.md's rule restated for a change the plan doesn't make.
- **Actions and effects, step 3, the list of `RemoteReply` literal sites** (1664, 1679, 2365-2376, 2754, 3570, 3723, 3736, 3767, `:829`): the compiler flags every one of them once the field is added. Say "every literal adds `set_cookie: None`" and drop the line numbers.
- **Decision 4 and the tests, the `pages` rate window (120 a minute) and its 429 test:** it is a second limit where one already suffices. The three files are constants answered on the connection thread and never reach the frame, and the 4-slot unauthenticated pool at accept (`src/adapters/remote.rs:701`) already bounds an unpaired client. Drop the window, its `Limits` field and its contract line.
- **Decision 4, step 4 and the tests, `HEAD` support:** no browser loading the page sends `HEAD`. Facts line 13 asks only that the method be defined. Answer it with the same 405 as the other methods, and drop the head-only flag on the general writer and the `HEAD` length test.
- **The tests, "`/api/v1/*` allowlist test (`:682`) and `/info` test (`:426`) still pass untouched":** the suite already enforces that, so this isn't a test to write.
- **For the owner, the O2/O3 bullet:** it says those items change nothing in the plan, so it has nothing to say. Drop it.
