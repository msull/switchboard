# Plan: issue #13 – typed conflict errors + `save(expected_version)`

## 0. Finding: the issue is already implemented

Verified in the worktree (branch tip `a82a3f6`, clean tree):

- Commit `7bfdeb0` "Typed conflict errors and a save(expected_version) primitive (#13)" is an ancestor of HEAD.
  It shipped in **21.0.0** (`CHANGELOG.md` `## [21.0.0] 2026-08-24`). The issue's "verified on 19.0.0" is stale.
- Part 1 (typed errors): `src/simplesingletable/exceptions.py` has `ConflictError(ValueError)`,
  `ResourceExistsError`, `TransactionError`, `TransactionConditionFailedError`, `VersionConflictError`.
  `DynamoDbMemory.transact_write_safe` raises `VersionConflictError` on `ConditionalCheckFailed`;
  `update_existing` (`dynamodb_memory.py:~577`) and `local_storage_memory.py:~503` raise `VersionConflictError`.
- Part 2 (`save`): `ResourceRepository.save(id, obj_in, expected_version, clear_fields, changed_by, audit_metadata)`
  at `src/simplesingletable/extras/repository.py:183`. `0` creates (raises `ResourceExistsError`), `>0` updates from
  that exact pre-image (raises `VersionConflictError`).
- Tests: `tests/test_typed_conflicts_and_save.py` covers both parts, including LocalStorage parity.

**Nothing in the issue's stated scope needs building.** Do not re-implement any of the above. The recommended
action on the issue is to close it as done in 21.0.0. The rest of this plan is a small follow-up for the one
place that still has the exact bug the issue describes.

## 1. Residual gap (the only code change)

`src/simplesingletable/extras/singleton.py:41`, in `SingletonVersionedResource.saved_updated_singleton`:

```python
if existing.version != self.version:
    raise ValueError("Cannot update from non-latest version")
```

This is the same stale-version condition the issue names, and it is the last bare `ValueError` for it in `src`
(`grep -rn "non-latest" src` shows only this one plus the two already-typed sites). A caller mapping
`ConflictError -> 409` would see this as a 400.

### Change

File to change: `src/simplesingletable/extras/singleton.py`.

1. Import: `from ..exceptions import VersionConflictError`.
2. Replace the raise with the structured form used elsewhere (`dynamodb_memory.py:577`):

```python
if existing.version != self.version:
    raise VersionConflictError(
        "Cannot update from non-latest version",
        resource_type=type(self).__name__,
        resource_id=existing.resource_id,
        expected_version=self.version,
        actual_version=existing.version,
    )
```

No signature changes. The message text is identical and `VersionConflictError` subclasses `ValueError`, so
existing `except ValueError` handlers and message matches keep working. Keep the keyword names exactly as
`VersionConflictError.__init__` defines them in `exceptions.py`; read that signature before editing and adjust if
it differs from the call used at `dynamodb_memory.py:577`.

Also update the docstring of `saved_updated_singleton` to add: "Raises `VersionConflictError` if this object is not
at the latest version."

`SingletonResource` (non-versioned) has no version check, so it is unchanged.

## 2. Tests (new, `tests/`, run against DynamoDB Local)

Add to `tests/test_typed_conflicts_and_save.py` (it is the home of the #13 tests). It uses the `dynamodb_memory`
fixture from `tests/conftest.py`. Add `SingletonVersionedResource` from `simplesingletable.extras.singleton`
to the imports, plus a model at module level:

```python
class VersionedSettings(SingletonVersionedResource):
    theme: str = "light"
```

Add a class `TestSingletonConflict`:

1. `test_stale_singleton_save_raises_version_conflict`
   - `a = VersionedSettings.ensure_exists(dynamodb_memory)`
   - `b = dynamodb_memory.update_existing(a, {"theme": "dark"})` (now at version 2; `a` is stale at version 1)
   - `a.theme = "blue"`; `with pytest.raises(VersionConflictError) as ei: a.saved_updated_singleton(dynamodb_memory)`
   - Assert `ei.value.expected_version == 1`, `ei.value.actual_version == 2`,
     `ei.value.resource_type == "VersionedSettings"`, and `isinstance(ei.value, ConflictError)`.
2. `test_stale_singleton_save_still_a_value_error` – same setup; `pytest.raises(ValueError, match="non-latest version")`
   (back-compat guard).
3. `test_current_singleton_save_succeeds` – `s = ensure_exists`; `s.theme = "dark"`;
   `out = s.saved_updated_singleton(dynamodb_memory)`; assert `out.version == 2` and `out.theme == "dark"`.

Run: `pytest tests/test_typed_conflicts_and_save.py -k Singleton`, then the full `pytest` (Docker required), then
`inv lint`.

## 3. Docs

- `AGENT_KNOWLEDGE.md` ~line 967 ("Versioned singletons check that you're updating from the latest version"):
  append "; a stale save raises `VersionConflictError` (a `ConflictError` / `ValueError`)".
- `AGENT_KNOWLEDGE.md` has no mention of `repo.save(...)` (`grep -n "save" AGENT_KNOWLEDGE.md` finds only unrelated
  hits and `saved_updated_singleton`). Add a short subsection near the `ConflictError` material
  (~lines 441-522) documenting `ResourceRepository.save(id, obj_in, expected_version, clear_fields=None,
  changed_by=None, audit_metadata=None)`:
  - `expected_version=0` → conditional create, `ResourceExistsError` if present
  - `>0` → update from that exact version, `VersionConflictError` if absent or at another version
  - non-versioned model → `TypeError`; negative → `ValueError`
  - example `try: repo.save(doc_id, changes, expected_version=v) except ConflictError: ...  # 409`
  Source the wording from the `save` docstring (`repository.py:183`) so the two do not drift.
- `README.md`: no change (it has no `save(` mention and no section for this area).

## 4. CHANGELOG

Under the existing `## [Unreleased]` section (it already has Added / Changed / Deprecated / Fixed subsections;
its `### Fixed` starts at line 88). Append to that `### Fixed`; do **not** write a version header, and do not
bump the version (`inv fullrelease` does both):

```markdown
* **`SingletonVersionedResource.saved_updated_singleton` raises `VersionConflictError` on a stale save** (#13
  follow-up). It previously raised a bare `ValueError("Cannot update from non-latest version")`, the one
  stale-version conflict left that could not be told apart from a programming error without matching the message.
  `VersionConflictError` subclasses `ValueError` and the message is unchanged, so existing handlers keep working;
  it now also carries `expected_version` / `actual_version`.
```

## 5. Explicitly out of scope

- Other `transact_write_safe` cancellations (not `ConditionalCheckFailed`) stay bare `ValueError`, by design per
  its docstring.
- No changes to `save`, the exception hierarchy, LocalStorageMemory, or the 21.0.0 tests.
- No version bump, no release, no edits to `pyproject.toml`, `__init__.py` `package_version`, or `uv.lock`
  (no dependency changes).

## 6. Summary of files touched

| File | Change |
|---|---|
| `src/simplesingletable/extras/singleton.py` | import + raise `VersionConflictError`; docstring |
| `tests/test_typed_conflicts_and_save.py` | new `VersionedSettings` model + `TestSingletonConflict` (3 tests) |
| `AGENT_KNOWLEDGE.md` | singleton note; new `repo.save` subsection |
| `CHANGELOG.md` | one `### Fixed` bullet under `## [Unreleased]` |
