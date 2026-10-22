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
4. Close #13 as delivered in 21.0.0 (outward-facing; see step 5).

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

Optional, if it fits the existing `LocalStorageMemory` tests in the same file: repeat test 1 against `LocalStorageMemory`, built the same way the file's existing `LocalStorageMemory` tests build it (use `tempfile`, which is already imported). Skip this if `LocalStorageMemory.get_existing` does not accept the `consistent_read` argument that `ensure_exists` passes. Check that signature before deciding; do not add a shim.

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
  unchanged, so existing handlers keep working.
```

This is type-only, so it goes under "Changed". It is not breaking.

## 5. Issue close-out (outward-facing; confirm before doing it)

Closing #13 and commenting on it are visible to other people. Get the user's go-ahead before doing either. Once confirmed:

- Close #13 with a comment that says it was delivered in 21.0.0 in commit `7bfdeb0`, with the typed hierarchy and `save()` shape described in section 0.
- Note in that comment that `save` takes `id` as its first argument (the shipped form), not the two-argument form in the issue.
- Open follow-up issues for:
  - non-versioned conditional create (section 1, out of scope);
  - whether `save` should also accept a two-argument form, if the team still wants it. Default: no.

The singleton fix from sections 2-4 can ship in the next release as a small change. If it is done in this branch, reference #13 in the CHANGELOG bullet and in the commit message. Do not put it in the close-out comment as a new feature.

## 6. Acceptance checklist

- [ ] `src/simplesingletable/extras/singleton.py` raises `VersionConflictError` with the four context fields, same message text.
- [ ] `tests/test_typed_conflicts_and_save.py` has the `TestSingletonVersionConflict` tests from section 3, and they pass on DynamoDB Local.
- [ ] The three message-text assertions in section 3 are unchanged and still pass.
- [ ] `CHANGELOG.md` has one bullet under `## [Unreleased]` / `### Changed`. No new version header. No edit to `pyproject.toml`, `__init__.py`, or the README version line.
- [ ] `inv lint` is clean.
- [ ] #13 is closed only after the user confirms (section 5).
