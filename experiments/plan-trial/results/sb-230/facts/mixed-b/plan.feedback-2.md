# Feedback on plan.md, round 2

## Earlier points

1. Settled. Decision 4 now sends every request through one queue with `MAX_IN_FLIGHT = 2`, opens the socket only after the first authenticated reply, and applies the three retries to every API call. Test 7 fixes the pool bound in a test.
2. Settled. Decision 2 cites the For-the-owner line, states the leak to other ports beside what the cookie gains, keeps the bearer branch written out in full, and adds the warning to §5.
3. Settled. Control scope and the device name each cite their For-the-owner line and write the other branch, and a default acceptance set is named for the owner to confirm.
4. Settled. Decision 1 says what happens if the exception is lost between visits or within one, and the not-yet-run list now includes it.
5. Settled. `pairing_url` returns `Option`, and with no address the QR is the JSON, "Copy link" is not drawn and the old scan line shows.
6. Settled. The wrong `Cargo.toml:91` citation is gone (see the cut under decision 5).
7. Settled. Decision 2 cites the Fetch step and requires the default `fetch` mode, the host refuses `Origin: null`, and test 2 checks that `app.js` has no `mode:`.

## Cuts

- **Where this departs from the issue:** drop the "Keeps the token" bullet; decision 2 says the same thing. Drop the "native text path" bullet; decision 3 says the same thing.
- **Decision 5:** drop the sentence about `[package] include` and crates.io. "Embedded with `include_str!`, no build step" is all the implementer needs.
- **Decision 6:** drop it. It says only "no change" and points to The record.
- **The record:** drop the clause about a ticket landing in parallel ("that ticket's bump is its own").
- **The signals:** drop the list of `RemoteReply` literal lines (`:1664` … `:3767`, `adapters/remote.rs:829`). The compiler will point to every literal that needs `set_cookie: None`.
- **The page, step 6:** drop `crypto.randomUUID()`. Sixteen bytes from `crypto.getRandomValues` work in every case, so one way of making the op id is enough.
- **The tests, `tests/remote.rs` case 1:** drop the instruction to amend the two existing allowlist tests (`:426`, `:682`). They will fail and point to themselves.
- **The tests, case 6:** drop "`stalled_strangers_…` passes unchanged". The existing suite already enforces that.
- **The tests, case 7:** drop the second case, which replays the queue's shape by hand. The first case already proves the bound, and the queue itself lives in JS that no test runs.
- **The tests, case 2:** drop the checks that `MAX_IN_FLIGHT = 2` is declared and `new WebSocket` occurs once. They match source text and prove nothing about behaviour.
- **The tests, the "No Swift test or fixture changes" bullet:** drop it. It describes nothing to do.
