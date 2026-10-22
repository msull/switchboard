# Response to plan feedback 2

Edited file: `/Users/sbgragent/Library/Application Support/Dispatch/tickets/c3a45d04/review/1/repo/plan.md`. The earlier plan at `tickets/c3a45d04/plan/1/repo/plan.md` is unchanged.

Points 1 to 5 were confirmations. The plan is unchanged for them, and nothing in them needed a further edit.

1. **`update(id)` false conflict: confirmed, no change.** The plan still routes `update(id)` through `_get(id, consistent_read=True)` (§4.1 change 2), and the cache decision stays as recorded in §2.
2. **T4 sequence: confirmed, no change.** T4 still clears the cache first (§6 T4), so it reaches the miss path.
3. **Fake scope: confirmed, no change.** `_lag_latest_reads` (§6) is unchanged.
4. **Outward-facing steps: confirmed, no change.** §8 is unchanged.
5. **Nits: confirmed, no change.** §5 line references and the §1 and §6 test count are unchanged.

## 6. T3 misstates how the test fails before the fix. Accepted.

You are right. With the lagged v1 pre-image and `expected_version=2`, the guard at `extras/repository.py:247` raises before `_update` runs. The conditional write is never reached.

Changed: **§6 T3**, the paragraph after the code block. It now says the pre-fix failure comes from the guard, with the `existing.version != expected_version` condition, and that the write is never reached. It also says the conditional write is not what T3 tests, and that T4 covers it. I did not change the T3 code block or the verification step, which still holds.

## 7. §6 uses names the file does not define. Accepted.

You are right. The file has no module-level `ID`, `create_obj` or `update_obj`. The models are `Doc`, `CreateDoc` and `UpdateDoc`. The existing tests use literal ids such as `"doc-1"` and pass dict payloads.

Changed:
- **§6**, new paragraph after the file-count paragraph. It says these names are defined inside each test, as the existing file does. It gives the values: `ID = "doc-1"`, `create_obj = {"title": "t", "content": "c"}`, `update_obj = {"content": "c2"}`. It says the model is `Doc`, which is also `repo.model_class`, and that payloads are dicts validated against `CreateDoc` and `UpdateDoc`.
- **§6 T2**: `Model` is now `Doc`.
- **§6 T5**: `obj` is now `update_obj`.

I did not run any tests, because Docker is not available here. The DynamoDB-backed tests are still unverified.
