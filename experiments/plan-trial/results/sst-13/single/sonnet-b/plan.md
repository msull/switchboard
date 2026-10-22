# Plan: issue #13 — typed conflicts on the non-transactional path + `save(expected_version)`

## Finding: most of #13 is already on `main`

The issue says "verified on `main` at 19.0.0". The tree is at **22.0.0**, and the work has landed
(it is described under `## [Unreleased]` in `CHANGELOG.md`, lines ~225-309):

| Issue item | State at HEAD (`a82a3f6`) |
|---|---|
| Conflict hierarchy | `exceptions.py`: `ConflictError(ValueError)` → `ResourceExistsError`, `TransactionConditionFailedError` → `VersionConflictError` (with `resource_type`, `resource_id`, `expected_version`, `actual_version`). Re-exported from `transactions.py` and the package root. |
| `transact_write_safe` | `dynamodb_memory.py:~296-327` raises `VersionConflictError` when any cancellation reason is `ConditionalCheckFailed`; other cancellations stay bare `ValueError`. |
| `update_existing` stale pre-image | `dynamodb_memory.py:~576` raises `VersionConflictError`. |
| Taken id on create | `dynamodb_memory.py:~1127` raises `ResourceExistsError`. |
| `LocalStorageMemory` parity | `local_storage_memory.py:434,502,557,568`. |
| `save` primitive | `ResourceRepository.save(id, obj_in, expected_version, clear_fields, changed_by, audit_metadata)` at `extras/repository.py:183`. `0` = create (create schema), `>0` = update from that version (update schema), negative → `ValueError`, non-versioned model → `TypeError`, bypasses the cache. |
| Tests | `tests/test_typed_conflicts_and_save.py` (hierarchy, back-compat, DynamoDB + local backends, `save` happy/stale/missing/exists/cache/schema cases). |

**Do not rebuild any of that.** An engineer who starts from the issue text will duplicate it.

Two deliberate differences from the issue's proposal, to leave as they are:

* Shipped signature is `save(id, obj_in, expected_version)`, not `save(obj_in, expected_version)`.
  The issue wanted the pre-read *object* passed so the library conditions on that exact version.
  `save` instead re-reads and compares `existing.version == expected_version` and then conditions the
  write on the freshly read item. A writer landing between the caller's read and `save`'s read is
  caught by the version comparison; one landing between `save`'s read and write is caught by the
  condition. Equivalent safety, and it supports create-by-id. Keep.
* Non-versioned `save` is out of scope (`_put_nonversioned_resource` has no condition). Keep as is.

## Remaining work (small): the gaps that survive

### 1. `SingletonVersionedResource.saved_updated_singleton` still raises bare `ValueError`

`src/simplesingletable/extras/singleton.py:41`:

```python
if existing.version != self.version:
    raise ValueError("Cannot update from non-latest version")
```

This is exactly the condition `VersionConflictError` names, and it is the same message callers were
string-matching. Change to:

```python
from simplesingletable.exceptions import VersionConflictError
...
if existing.version != self.version:
    raise VersionConflictError(
        "Cannot update from non-latest version",
        resource_type=type(self).__name__,
        resource_id=str(existing.resource_id),
        expected_version=self.version,
        actual_version=existing.version,
    )
```

Message text is unchanged; `VersionConflictError` is a `ValueError`, so existing handlers still work.
(Confirm `existing.resource_id` is the right attribute on `BaseSingleton`/resources by reading
`models.py`; adjust if the id attribute is named differently.)

### 2. Populate structured context in `transact_write_safe`

`dynamodb_memory.py:326` raises `VersionConflictError(message, cancellation_reasons=...)` with no
`operation_indexes`. The reasons list is positional with `transact_items`, so fill it:

```python
failed = [i for i, r in enumerate(cancellation_reasons) if r and r.get("Code") == "ConditionalCheckFailed"]
raise VersionConflictError(message, cancellation_reasons=cancellation_reasons, operation_indexes=failed) from e
```

Do not try to guess `resource_id`/`expected_version` here (the helper only sees raw transact items);
leave those `None`, which the `VersionConflictError` docstring already permits.

### 3. Docs

* `README.md`: grep for "non-latest version", `ValueError`, and any conflict-handling or
  `ResourceRepository` section; if there is no mention of `ConflictError`/`save`, add a short
  "Handling write conflicts" example (the `except ConflictError` snippet from the CHANGELOG).
  Do **not** touch the version line in the README (bumpver owns it).

## Tests to add (`tests/`, run against DynamoDB Local via existing fixtures)

Append to `tests/test_typed_conflicts_and_save.py`, reusing `Doc`, `repo`, `dynamodb_memory`:

1. `test_singleton_stale_save_raises_version_conflict` — define a `SingletonVersionedResource`
   subclass; create it, take two copies, `saved_updated_singleton` on the first, then on the stale
   second: assert `pytest.raises(VersionConflictError)`, that it is also `ValueError`/`ConflictError`,
   `str(exc)` still contains `"non-latest version"`, and `expected_version`/`actual_version` are set.
2. `test_transact_write_safe_reports_failed_operation_indexes` — build a two-item `TransactWriteItems`
   where only the second item has a failing `ConditionExpression` (e.g. `attribute_exists(pk)` on a
   missing key), call `transact_write_safe(dynamodb_memory.dynamodb_client, items)`, assert
   `VersionConflictError` with `operation_indexes == [1]`. (Check `conftest.py` for the client
   attribute name.)
3. `test_non_condition_cancellation_stays_plain_value_error` — only if a cheap way to trigger a
   non-conditional cancellation exists (e.g. duplicate key in one transaction → `ValidationException`
   is not a cancellation, so skip if it cannot be produced). Do not mock boto3 for this.

Existing tests need no edits. Run `pytest tests/test_typed_conflicts_and_save.py`, then full `pytest`
and `inv lint`.

## CHANGELOG

Add under the existing `## [Unreleased]` → `### Changed` (do not create a version header; do not
bump anything — `inv fullrelease` does that):

```markdown
* **`SingletonVersionedResource.saved_updated_singleton` raises `VersionConflictError`** (#13) instead
  of a bare `ValueError("Cannot update from non-latest version")`. Message unchanged; it remains a
  `ValueError`. `transact_write_safe` now also fills `operation_indexes` on the `VersionConflictError`
  it raises, identifying which transact items failed their condition.
```

## Out of scope
Version bump/release, non-versioned `save`, changing the `save` signature, any further `TypeError`
conversions (already done for the 19 sites).
