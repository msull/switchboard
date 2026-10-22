# Plan: issue #13 — typed conflict errors + `save(obj, expected_version)`

## Status: almost entirely shipped; one residual gap remains

Verified against the worktree (HEAD `a82a3f6`, package 22.0.0). Both halves of #13 landed in 21.0.0 (commit `7bfdeb0`, CHANGELOG `## [21.0.0]`):

- Typed errors: `ConflictError(ValueError)`, `ResourceExistsError`, `TransactionConditionFailedError`, `VersionConflictError` live in `src/simplesingletable/exceptions.py` and are exported from the package root. `transact_write_safe` and `update_existing` (`dynamodb_memory.py` ~l.577) and `local_storage_memory.py` raise them.
- `ResourceRepository.save(id, obj_in, expected_version, ...)` exists at `extras/repository.py:183`. It is tested in `tests/test_typed_conflicts_and_save.py`.

Do **not** re-implement any of that. Do not change `save` to take the pre-read object: the shipped `save(id, ...)` re-reads and conditions on the version, so it has no TOCTOU hole, and the choice is documented in the 21.0.0 notes.

**The remaining gap** is `SingletonVersionedResource.saved_updated_singleton` (`src/simplesingletable/extras/singleton.py:41`). It still raises a bare `ValueError("Cannot update from non-latest version")` for exactly the condition `VersionConflictError` names. A singleton caller still has to match message text to tell a lost write from a programming error, which is the problem #13 describes. The plan is to close it.

Out of scope, deliberately:
- Non-versioned conditional create for `save`. It is explicitly deferred; `_put_nonversioned_resource` has no condition.
- A `save` equivalent on `VersionedResourceRepository`. Not requested.
- Any version bump or release (`inv fullrelease` does it).

## Files to change

### 1. `src/simplesingletable/extras/singleton.py`

Add the import next to the existing relative import:

```python
from ..exceptions import VersionConflictError
```

Replace the bare raise in `SingletonVersionedResource.saved_updated_singleton`:

```python
existing = self.ensure_exists(memory)
if existing.version != self.version:
    raise VersionConflictError(
        "Cannot update from non-latest version",
        resource_type=type(self).__name__,
        resource_id=existing.resource_id,
        expected_version=self.version,
        actual_version=existing.version,
    )
return memory.update_existing(existing, self)
```

Notes for the implementer:
- Keep the message text **identical**, so `pytest.raises(ValueError, match=...)` callers and tests keep passing.
- Check the keyword names against the `VersionConflictError.__init__` signature in `exceptions.py`, and mirror the call at `dynamodb_memory.py:577` (`resource_type`, `resource_id`, `expected_version`, `actual_version`). Use whatever that file actually defines if it differs from the above.
- `expected_version` is the version the caller holds (`self.version`). `actual_version` is the stored latest (`existing.version`). This matches the convention at `dynamodb_memory.py:577`.
- Update the method docstring with one line: "Raises `VersionConflictError` (a `ValueError` subclass) if this object's version is not the latest."
- `SingletonResource` (non-versioned) needs no change.
- **Keep the pre-check; do not delete it in favour of `update_existing`.** `update_existing` would raise an equivalent `VersionConflictError` (it re-reads the latest and compares), so the two checks look redundant. The pre-check stays because it fails before any write is attempted and carries singleton-specific context (`resource_type=type(self).__name__`, the singleton's own id). Add a one-line code comment saying so, so a reviewer does not ask why there are two checks.

No public API signature changes. The only behaviour change is that the raised exception is now a `VersionConflictError` (still a `ValueError`, same message).

### 2. `CHANGELOG.md`

Add under the existing `## [Unreleased]`. There are already `### Added` (l.10), `### Changed` (l.56), `### Deprecated` and `### Fixed` (l.88) sections, so append a bullet to **`### Fixed`**, after the `AuditConfig.exclude_fields` entry. Do not add a version header.

```markdown
* **`SingletonVersionedResource.saved_updated_singleton` raises a typed conflict** (#13). Saving a
  stale singleton raised a bare `ValueError("Cannot update from non-latest version")`, the one write
  path the 21.0.0 typed-conflict work missed. It now raises `VersionConflictError` (with
  `resource_type`, `resource_id`, `expected_version`, `actual_version`) so callers can tell a lost
  optimistic write from a programming error without matching message text. `VersionConflictError`
  is a `ValueError` subclass and the message is unchanged, so existing `except ValueError` handlers
  keep working.
```

### 3. `AGENT_KNOWLEDGE.md` (section 6, Singleton Pattern, ~l.966)

In the **Key behavior** list under "SingletonVersionedResource (Versioned)", the line `- Versioned singletons check that you're updating from the latest version` is the one to extend. Change it to:

```markdown
- Versioned singletons check that you're updating from the latest version; a stale `saved_updated_singleton` raises `VersionConflictError` (a `ConflictError`, which is a `ValueError`) with `expected_version` / `actual_version` set
```

No other section of the guide changes.

## Tests to add

There are currently no singleton tests apart from incidental `FormConfig` use in `tests/test_form_data.py`. Add the new tests to the **existing** `tests/test_typed_conflicts_and_save.py` (it already imports `VersionConflictError`, uses the `dynamodb_memory` fixture from `tests/conftest.py`, and covers this feature), as a new class at the end of the file. Add an import of `SingletonVersionedResource` from `simplesingletable.extras.singleton` at the top.

Define a module-level model:

```python
class VersionedSettings(SingletonVersionedResource):
    theme: str = "light"
```

Class `TestSingletonConflicts`, using the `dynamodb_memory` fixture (DynamoDB Local). Tests:

**Why DynamoDB only, no `LocalStorageMemory` case:** `LocalStorageMemory` has its own `update_existing`, but singletons are DynamoDB-only. `BaseSingleton.ensure_exists` and `saved_updated_singleton` are typed against `DynamoDbMemory`, so there is no local-storage singleton path to test. The local-storage typed-conflict parity is already covered in the existing file.

**Table state between tests:** the singleton id is fixed (the class name), so the tests would share one item if the table persisted. It does not: `dynamodb_memory` (`tests/conftest.py:85`) is function-scoped and calls `reset_local_dynamodb_test_table` (a truncate) before every test, so each test starts empty. Still derive all version assertions from `a.version` rather than literals, so the tests do not depend on that.

1. `test_stale_singleton_save_raises_version_conflict`
   - `a = VersionedSettings.ensure_exists(dynamodb_memory)`.
   - `b = a.model_copy(update={"theme": "dark"})`, then `b.saved_updated_singleton(dynamodb_memory)` succeeds. Assert the returned version is `a.version + 1`.
   - Now `a.model_copy(update={"theme": "blue"}).saved_updated_singleton(dynamodb_memory)` must raise `VersionConflictError`.
   - Assert `exc.expected_version == a.version`, `exc.actual_version == a.version + 1`, `exc.resource_type == "VersionedSettings"`, and `exc.resource_id == "VersionedSettings"` (`ensure_exists` creates the singleton with `override_id=cls.__name__`, `singleton.py:12`).
   - Assert the stored value is still `"dark"`, reading it back with `dynamodb_memory.read_existing("VersionedSettings", VersionedSettings, consistent_read=True)`. The default `consistent_read=False` could return an eventually-consistent item; `ensure_exists` itself defaults to consistent reads, so match that.
2. `test_stale_singleton_error_is_still_a_value_error`
   - Same setup. `pytest.raises(ValueError, match="non-latest version")` must also catch it, and `isinstance(exc.value, ConflictError)` is true. This is the back-compat guarantee.
3. `test_current_singleton_save_succeeds`
   - Ensure, copy with a change, save. No raise; the version increments; a reread shows the new value. This guards against the check being inverted.

## Verification

- `pytest tests/test_typed_conflicts_and_save.py` (needs Docker for DynamoDB Local).
- `pytest tests/test_local_storage.py tests/test_blob_storage_bugfixes.py tests/test_form_data.py`. These match the old message text or use singletons, and should stay green.
- `inv lint`.
- Skim the edited `AGENT_KNOWLEDGE.md` bullet in context to confirm it reads correctly.
- Full `pytest` before opening the PR.

## Suggested handling of the issue itself

Reply on #13 that both halves shipped in 21.0.0 (link `7bfdeb0` and the changelog entry), that this PR closes the one remaining untyped site (singletons), and that `save(id, ...)` deliberately takes an id rather than a pre-read object because it re-reads and conditions on the stored version. Close #13 when the PR merges.
