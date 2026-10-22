# Response to plan feedback 3

Edited file: `/Users/sbgragent/Library/Application Support/Dispatch/tickets/c3a45d04/review/1/repo/plan.md`. The earlier plan at `tickets/c3a45d04/plan/1/repo/plan.md` is unchanged.

Points 1 to 7 are confirmations that earlier fixes are satisfied. None needed a further edit, so the plan is unchanged for them:

1. **`update(id)` false conflict: confirmed, no change.** §4.1 change 2 and the §2 cache decision are as before.
2. **T4 sequence: confirmed, no change.** §6 T4 still clears the cache first.
3. **Fake scope: confirmed, no change.** §6 `_lag_latest_reads` is as before.
4. **Outward-facing steps: confirmed, no change.** §8 is as before.
5. **Nits: confirmed, no change.** §5 and the §1 and §6 test count are as before.
6. **T3 failure mechanism: confirmed, no change.** §6 T3 is as before.
7. **Undefined test names: confirmed, no change.** §6 is as before.

## The T1 detail (unnumbered in the feedback). Accepted.

You are right that the T1 assertion needs to be precise. I checked the code: `read_existing` calls `self.get_existing` (`dynamodb_memory.py:541`), so during `update(ID)` the spy sees more than one `get_existing` call. Those are the pre-image from `_get` (path C), and the latest-version read and post-write read that go through `read_existing`. The post-write read was already consistent.

Changed: **§6 T1**. It now says `update(ID)` calls `get_existing` more than once, and asserts that every `get_existing` call made during that update has `consistent_read=True`. It no longer depends on which call is which.

I did not change the T4 behaviour check, which still covers the outcome.

I did not run any tests, because Docker is not available here. The DynamoDB-backed tests are still unverified.
