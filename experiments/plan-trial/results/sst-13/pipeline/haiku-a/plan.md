# Plan: issue #13 — typed conflict errors and `save(expected_version)`

Repo: `/Users/sbgragent/.dispatch/worktrees/ce7eaf93` (HEAD `a82a3f6`, equal to `main`, tree clean).
Version on tree: 22.0.0. Do not bump the version or touch `pyproject.toml`, `src/simplesingletable/__init__.py`, or the `README.md` version line. `inv fullrelease` owns those.

## 0. Status: the issue is already delivered

Both halves of #13 shipped in **21.0.0** in commit `7bfdeb0` ("Typed conflict errors and a save(expected_version) primitive (#13)"). That commit is an ancestor of HEAD (verified with `git merge-base --is-ancestor 7bfdeb0 HEAD`).

| Issue claim (written against 19.0.0) | State on this tree |
|---|---|
| `transact_write_safe` turns `ConditionalCheckFailed` into bare `ValueError` | Fixed. `src/simplesingletable/dynamodb_memory.py:296-326` raises `VersionConflictError` when a cancellation carries `ConditionalCheckFailed`. Other cancellation reasons stay a bare `ValueError` by design. |
| `update_existing` raises `ValueError("Cannot update from non-latest version")` | Fixed. `dynamodb_memory.py:575-583` raises `VersionConflictError` with `expected_version`, `actual_version`, `resource_type`, `resource_id`. `local_storage_memory.py:502` does the same. |
| Repository callers must match message text | Not needed. `ConflictError` covers both paths. |
| No `save(obj, expected_version)` | Implemented as `ResourceRepository.save` at `src/simplesingletable/extras/repository.py:183`. Inherited by `VersionedResourceRepository`. |

The remaining work is one real gap in the singleton path, plus issue close-out. Nothing else in #13 needs code.

Typed hierarchy as shipped (`src/simplesingletable/exceptions.py`):

```
ValueError
└── ConflictError                        (resource_type, resource_id)
    ├── ResourceExistsError
    └── TransactionConditionFailedError  (also TransactionError)
        └── VersionConflictError         (expected_version, actual_version)
```

## 1. Scope of this plan

**In scope**

1. Convert the one remaining bare conflict raise, `SingletonVersionedResource.saved_updated_singleton`, to `VersionConflictError`.
2. Add tests for it.
3. Add one CHANGELOG bullet under `## [Unreleased]`.
4. Close #13 after this branch merges, with a comment that covers both the 21.0.0 delivery and this fix (outward-facing; see section 5).

**Out of scope — do not change**

- **The `save` signature.** The issue proposed `save(obj_in, expected_version)`. The shipped form is `save(id, obj_in, expected_version, clear_fields=None, changed_by=None, audit_metadata=None)`. The `id` is required because `save` may create, and creating needs an explicit id. Keep the shipped form. It is already documented in the 21.0.0 CHANGELOG entry.
- **Non-versioned conditional create.** `_put_nonversioned_resource` still writes unconditionally, and `save` raises `TypeError` for non-versioned models. This is a separate feature. Do not implement it here; open a follow-up issue.
- **`_build_versioned_update_items` raising `ResourceNotFoundError`** (`transactions.py:873`). That is a plain `Exception` for "missing" (not a race), and the issue does not ask for it. Leave it.
- **`get_or_create`.** Leave it. `save` does not depend on it.
- **The message text.** Keep `"Cannot update from non-latest version"` verbatim everywhere. Three existing tests assert on it, and they stay as they are (see section 4).

## 2. Code change: `SingletonVersionedResource` raises a typed conflict

### File: `src/simplesingletable/extras/singleton.py`

Current code (lines 35-42):

```python
class SingletonVersionedResource(DynamoDbVersionedResource, BaseSingleton):
    ...
    def saved_updated_singleton(self, memory: "DynamoDbMemory") -> Self:
        """Overwrites the existing Singleton with the current object;
        the version number on this resource must match the latest existing version in the database."""
        existing = self.ensure_exists(memory)
        if existing.version != self.version:
            raise ValueError("Cannot update from non-latest version")
        return memory.update_existing(existing, self)
```

Change the `raise` only. The signature, docstring, and the `ensure_exists` / `update_existing` calls stay the same.

Add the import at the top of the file:

```python
from ..exceptions import VersionConflictError
```

Replace the `raise` with:

```python
        if existing.version != self.version:
            raise VersionConflictError(
                "Cannot update from non-latest version",
                resource_type=self.__class__.__name__,
                resource_id=str(self.resource_id),
                expected_version=self.version,
                actual_version=existing.version,
            )
```

Notes for the implementer:

- `VersionConflictError` subclasses `ValueError` (through `ConflictError`), so every existing `except ValueError` handler still catches it.
- The message is unchanged on purpose.
- `resource_id` is the attribute the 21.0.0 `GsiIndex` CHANGELOG example uses. `ensure_exists` creates the singleton with `override_id=cls.__name__`, so `str(self.resource_id)` is the class name.
- Do not change `ensure_exists`, `BaseSingleton`, or `SingletonResource` (the non-versioned variant has no version to conflict on).
- No change is needed in `update_existing`. Its own race is already typed.

No other `src/` file needs to change. `grep` confirms the only remaining bare conflict raise is this one. The other two hits of the message string are already `VersionConflictError` (`dynamodb_memory.py:577`, `local_storage_memory.py:503`).

### File: `AGENT_KNOWLEDGE.md`

In the `**Key behavior:**` list of the Singleton Pattern section (around line 966), replace this bullet:

```
- Versioned singletons check that you're updating from the latest version
```

with:

```
- Versioned singletons check that you're updating from the latest version. A stale `saved_updated_singleton` raises `VersionConflictError` (a `ConflictError` and a `ValueError`), so catch that type to handle a lost race.
```

Leave the "Exception hierarchy" block (around line 1770) unchanged. It documents the transaction types imported from `simplesingletable.transactions`. It does not cover `ConflictError`, and extending it to the whole conflict family is a separate docs task, not part of this fix.

## 3. Tests

### File: `tests/test_typed_conflicts_and_save.py` (extend; do not create a new file)

The module docstring already says it covers #13, so the singleton tests belong here.

Add imports:

```python
from simplesingletable.extras.singleton import SingletonVersionedResource
```

Add a module-level model (defaults are required because `ensure_exists` creates with `{}`):

```python
class AppSettings(SingletonVersionedResource):
    theme: str = "light"
```

Add a class `TestSingletonVersionConflict` with these tests. Use the existing `dynamodb_memory` fixture (DynamoDB Local via Docker; see `tests/conftest.py:86`).

Give every new test a one-line docstring that says why the test matters, in the style of the file's existing tests (for example, `test_version_conflict_is_both_a_transaction_error_and_a_value_error`). Test 2's docstring must record the back-compat intent: a caller still on `except ValueError` or `except ConflictError` keeps catching a stale singleton write.

1. `test_stale_singleton_raises_version_conflict(dynamodb_memory)`
   - `first = AppSettings.ensure_exists(dynamodb_memory)`. Assert `first.version == 1`. Verify this in the run rather than assuming it.
   - `fresh = first.model_copy(update={"theme": "dark"})`; `fresh.saved_updated_singleton(dynamodb_memory)`. This moves the stored version to 2.
   - `with pytest.raises(VersionConflictError) as exc: first.saved_updated_singleton(dynamodb_memory)`.
   - Assert `exc.value.expected_version == 1`, `exc.value.actual_version == 2`, `exc.value.resource_type == "AppSettings"`, `exc.value.resource_id == first.resource_id`.
   - Assert `"Cannot update from non-latest version" in str(exc.value)`.

2. `test_stale_singleton_is_catchable_as_conflict_and_value_error(dynamodb_memory)`
   - Same setup as test 1.
   - Assert `pytest.raises(ConflictError)` catches it, and separately that `pytest.raises(ValueError)` catches it.

3. `test_stale_singleton_write_does_not_change_stored_state(dynamodb_memory)`
   - Same setup. After the failed stale write, read back with `AppSettings.ensure_exists(dynamodb_memory)`.
   - Assert `theme == "dark"` and `version == 2`. The rejected write must not be applied.

4. `test_current_version_singleton_still_saves(dynamodb_memory)` (regression guard)
   - Create, update once with the current object, then update again from the object returned by the first update (or from a fresh `ensure_exists`).
   - Assert no exception and the final `version` is correct.

5. `test_local_storage_stale_singleton_raises_version_conflict(local_storage)` (required, not optional)
   - Same flow as test 1, using the `local_storage` fixture that the backend-parity test (`test_local_storage_raises_the_same_types_as_dynamodb`, line 209) already uses.
   - `ensure_exists` works on `LocalStorageMemory`: `get_existing` accepts `consistent_read` (`local_storage_memory.py:221`) and `create_new` accepts `override_id` (`local_storage_memory.py:343`). The check in `singleton.py` runs before any memory call, so the same assertions apply to both backends.
   - Do not add a skip condition or a shim.

### Existing tests to keep unchanged

These assert the message text on purpose, as a backwards-compatibility guard. Do not convert them to typed checks, and do not delete them:

- `tests/test_typed_conflicts_and_save.py:154`
- `tests/test_blob_storage_bugfixes.py:113`
- `tests/test_local_storage.py:555`

### Run

Docker must be running for the DynamoDB-backed tests. The investigation could not run them because Docker is not installed on the investigation machine.

```bash
pytest tests/test_typed_conflicts_and_save.py -v
pytest -k "singleton or non_latest or non-latest" -v
pytest tests/test_typed_conflicts_and_save.py tests/test_local_storage.py tests/test_blob_storage_bugfixes.py
inv lint
```

No dependency change, so no `uv lock` is needed.

## 4. CHANGELOG

### File: `CHANGELOG.md`, section `## [Unreleased]`, subsection `### Changed`

Append one bullet to the end of the existing `### Changed` block under `## [Unreleased]`. Do not touch `## [22.0.0]`, `## [21.0.0]`, or any other released section. Do not add a version header. `inv fullrelease` stamps the header from the `## [Unreleased]` marker.

Text:

```markdown
* **`SingletonVersionedResource.saved_updated_singleton` raises `VersionConflictError`** (#13). A
  stale version used to raise a bare `ValueError`; it now raises `VersionConflictError`, with
  `expected_version` and `actual_version` set. It is still a `ValueError` and the message is
  unchanged, so existing handlers keep working. A caller that catches `TransactionError` will now also catch this case, as it already does for the same condition on `update_existing`.
```

Put it under "Changed". It is not breaking for `except ValueError` handlers. It does broaden `except TransactionError` handlers: `VersionConflictError` subclasses `TransactionConditionFailedError`, which subclasses `TransactionError`, so a stale singleton write that used to escape as a bare `ValueError` is now caught by them. That is consistent with `update_existing`, which already raises `VersionConflictError` for the same condition (`dynamodb_memory.py:576`). `grep` finds no `except TransactionError` in `src/`, so the library itself is unaffected. The bullet above states the broadening so downstream callers can see it.

## 5. Issue close-out (outward-facing; confirm before doing it)

Closing #13 and commenting on it are visible to other people. Get the user's go-ahead before doing either. Once confirmed:

- After this branch merges, close #13 with a comment that says it was delivered in 21.0.0 in commit `7bfdeb0` (the typed hierarchy and `save()` shape, section 0), and that the stale-singleton gap was fixed in the merged PR and ships in the next release.
- Note in that comment that `save` takes `id` as its first argument (the shipped form), not the two-argument form in the issue.
- Open follow-up issues for:
  - non-versioned conditional create (section 1, out of scope);
  - whether `save` should also accept a two-argument form, if the team still wants it. Default: no.

The singleton fix from sections 2-4 ships on this branch, so #13 stays open until that branch merges. Reference #13 in the CHANGELOG bullet and in the commit message. The close-out comment reports the singleton fix as merged in the next release, not as part of 21.0.0.

## 6. Acceptance checklist

- [ ] `src/simplesingletable/extras/singleton.py` raises `VersionConflictError` with the four context fields, same message text.
- [ ] `tests/test_typed_conflicts_and_save.py` has the `TestSingletonVersionConflict` tests from section 3, and they pass on DynamoDB Local.
- [ ] The three message-text assertions in section 3 are unchanged and still pass.
- [ ] `CHANGELOG.md` has one bullet under `## [Unreleased]` / `### Changed`. No new version header. No edit to `pyproject.toml`, `__init__.py`, or the README version line.
- [ ] `AGENT_KNOWLEDGE.md` Singleton Pattern "Key behavior" list has the bullet from section 2. The Exception hierarchy block is unchanged.
- [ ] All new tests have one-line docstrings in the file's style (section 3). Test 5 (local backend) is present and passes.
- [ ] `inv lint` is clean.
- [ ] #13 is closed only after this branch merges and the user confirms (section 5).
