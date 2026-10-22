# Response to plan.feedback-1.md

All five points accepted; `plan.md` in this directory was edited for each.

1. **Docs not updated.** Accepted. Added section "### 3. `AGENT_KNOWLEDGE.md`" under "Files to change". It extends the existing bullet "Versioned singletons check that you're updating from the latest version" (section 6, ~l.966) to say a stale `saved_updated_singleton` raises `VersionConflictError` (a `ConflictError`, hence a `ValueError`). I checked the file: that bullet exists at that location. I also added a matching item to "Verification".

2. **Test coverage / local storage / consistent read.** Accepted. In "Tests to add" I added a "Why DynamoDB only" paragraph stating that singletons are DynamoDB-only (`BaseSingleton` is typed against `DynamoDbMemory`), so no `LocalStorageMemory` case exists. Test 1 now reads the stored value back with `read_existing(..., consistent_read=True)`, since the default is `False` (verified in `dynamodb_memory.py`).

3. **Hedging on `resource_id`.** Accepted. In "Tests to add", test 1 now states the literal `"VersionedSettings"` and cites `override_id=cls.__name__` (`singleton.py:12`). The trailing "if this fails, assert against `existing.resource_id`" paragraph is removed.

4. **Table state between tests.** Accepted. I confirmed in `tests/conftest.py:85` that `dynamodb_memory` is function-scoped and truncates the table before each test (`reset_local_dynamodb_test_table`). The "Tests to add" section now records this, and still says to derive versions from `a.version` so the tests don't depend on it.

5. **Two checks.** Accepted. In "Files to change" → `singleton.py` notes, added a bullet saying the pre-check stays: it fails before any write and carries singleton-specific context, and it asks for a one-line code comment explaining why there are two checks.
