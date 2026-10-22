# Response to plan feedback 4

Edited file: `/Users/sbgragent/Library/Application Support/Dispatch/tickets/c3a45d04/review/1/repo/plan.md`. The earlier plan at `tickets/c3a45d04/plan/1/repo/plan.md` is unchanged.

Points 1 to 7 are confirmations that earlier fixes are still satisfied. The plan is unchanged for them:

1. **`update(id)` false conflict: confirmed, no change.** §4.1 change 2 and the §2 cache decision are as before.
2. **T4 sequence: confirmed, no change.** §6 T4 still clears the cache first.
3. **Fake scope: confirmed, no change.** §6 `_lag_latest_reads` is as before.
4. **Outward-facing steps: confirmed, no change.** §8 is as before.
5. **Nits: confirmed, no change.** §5 and the §1 and §6 test count are as before.
6. **T3 failure mechanism: confirmed, no change.** §6 T3 is as before.
7. **Undefined test names: confirmed, no change.** §6 is as before.

8. **T1 assertion precision: confirmed, no change.** The point is covered by the §6 T1 edit from the last round. The `read_existing` delegation is at `dynamodb_memory.py:541-542`, as the feedback says.

## A correction I made on my own

Re-reading §7 step 2, I found it said T1–T4 fail before the fix without saying how. That was incomplete. T3 and T4 fail with `VersionConflictError`. T1 and T2 fail on their `consistent_read=True` assertions, because the default read is still in place. I checked T2: its first `read_existing` call is the latest-version read, which runs with the default before the fix.

Changed: **§7 step 2** now states the failure mode for each test.

I did not run any tests, because Docker is not available here. The DynamoDB-backed tests are still unverified.
