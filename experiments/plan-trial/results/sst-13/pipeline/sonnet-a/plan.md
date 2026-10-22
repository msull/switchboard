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

## 2. Tests (new, `tests/`)

Add to `tests/test_typed_conflicts_and_save.py` (the home of the #13 tests). That file is **module-level `test_*`
functions with fixtures, no test classes**; follow that. Add `SingletonVersionedResource` (from
`simplesingletable.extras.singleton`) to the imports and a model next to `Doc`/`Plain`:

```python
class VersionedSettings(SingletonVersionedResource):
    theme: str = "light"
```

Put the tests in a new section after `test_local_storage_raises_the_same_types_as_dynamodb`, under a banner
comment in the file's existing style (`# --- singleton save ---`). Fixtures: `dynamodb_memory` (conftest.py:85) and
`local_storage` (defined in the test file, line 75).

**Fixture isolation (checked):** `dynamodb_memory` is function-scoped and calls `reset_local_dynamodb_test_table`
(truncates the table) before every test, and `local_storage` is a fresh `TemporaryDirectory` per test. So
`ensure_exists` on the shared `SINGLETON#VersionedSettings` key always starts at version 1 and the absolute
`version` asserts below are safe.

1. `test_stale_singleton_save_raises_version_conflict(dynamodb_memory)`
   - `stale = VersionedSettings.ensure_exists(dynamodb_memory)` (version 1)
   - `dynamodb_memory.update_existing(stale, {"theme": "dark"})` (now version 2; `stale` is behind)
   - `stale.theme = "blue"`; `with pytest.raises(VersionConflictError) as exc_info: stale.saved_updated_singleton(dynamodb_memory)`
   - Assert `isinstance(exc_info.value, ConflictError)`, `(exc_info.value.expected_version, exc_info.value.actual_version) == (1, 2)`,
     `exc_info.value.resource_type == "VersionedSettings"`, `exc_info.value.resource_id == stale.resource_id`.
2. `test_stale_singleton_save_is_still_a_value_error(dynamodb_memory)` – same setup;
   `pytest.raises(ValueError, match="non-latest version")` (back-compat guard).
3. `test_current_singleton_save_succeeds(dynamodb_memory)` – `s = ensure_exists`; `s.theme = "dark"`;
   `out = s.saved_updated_singleton(dynamodb_memory)`; assert `out.version == 2` and `out.theme == "dark"`.
4. `test_local_storage_singleton_stale_save_raises_version_conflict(local_storage)` – backend parity, mirroring
   `test_local_storage_raises_the_same_types_as_dynamodb` (line 209). `saved_updated_singleton` is backend-agnostic
   (it only calls `ensure_exists`/`update_existing` on the memory), so it must raise the same type on both.
   Same setup as test 1 using `local_storage`; assert `VersionConflictError` and `(expected, actual) == (1, 2)`.
   Verified beforehand that `SingletonVersionedResource.ensure_exists` + `update_existing` work on
   `LocalStorageMemory` and that today the stale save raises a bare `ValueError` there, so this test fails before
   the fix and passes after.

Run: `pytest tests/test_typed_conflicts_and_save.py -k singleton` (tests 1-3 need Docker), then the full `pytest`,
then `inv lint`.

## 3. Docs

Two **separate** items; only (a) is part of the bug fix.

**(a) Singleton note (belongs with the fix)** – `AGENT_KNOWLEDGE.md` ~line 967 ("Versioned singletons check that
you're updating from the latest version"): append "; a stale save raises `VersionConflictError` (a
`ConflictError` / `ValueError`)".

**(b) Docs catch-up for the already-shipped #13 (independent of the fix; may be split into its own PR)**, all in
`AGENT_KNOWLEDGE.md`:

- Add a short subsection near the `ConflictError` material (~lines 441-522) documenting
  `ResourceRepository.save(id, obj_in, expected_version, clear_fields=None, changed_by=None, audit_metadata=None)`
  (`grep -n "save" AGENT_KNOWLEDGE.md` finds only unrelated hits and `saved_updated_singleton`):
  - `expected_version=0` → conditional create, `ResourceExistsError` if present
  - `>0` → update from that exact version, `VersionConflictError` if absent or at another version
  - non-versioned model → `TypeError`; negative → `ValueError`
  - example `try: repo.save(doc_id, changes, expected_version=v) except ConflictError: ...  # 409`
  Source the wording from the `save` docstring (`repository.py:183`) so the two do not drift.
- **Fix the stale exception tree in "Exception hierarchy" (~lines 1770-1785).** It currently says
  `VersionConflictError  # Back-compat alias.` under `TransactionConditionFailedError`, imports from
  `simplesingletable.transactions`, and says "All three classes carry ...". Checked against `exceptions.py`:
  `ConflictError(ValueError)` (line 43) is the root; `ResourceExistsError(ConflictError)` (66);
  `TransactionError(Exception)` (74); `TransactionConditionFailedError(TransactionError, ConflictError)` (96);
  `VersionConflictError(TransactionConditionFailedError)` (127) is a real subclass that adds
  `resource_type`/`resource_id`/`expected_version`/`actual_version`, not an alias. Replace the tree with:

  ```
  # ValueError
  # └── ConflictError                              # An optimistic-concurrency write lost.
  #     ├── ResourceExistsError                    # Create onto an id that is taken.
  #     └── TransactionConditionFailedError        # (also a TransactionError)
  #         └── VersionConflictError               # Stale/absent version; carries expected/actual_version.
  # TransactionError (Exception)                   # Any transaction failure.
  # └── TransactionConditionFailedError            # (above)
  ```

  Import from `simplesingletable` (or `simplesingletable.exceptions`; `transactions` re-exports the same objects),
  and reword "All three classes carry" so `cancellation_reasons`/`operation_indexes` are attributed to the
  transaction errors and the version fields to `VersionConflictError`. Before editing, re-read `exceptions.py`
  lines 43-160 and confirm which attributes each class really defines; write only what it shows.
- `README.md`: no change (no `save(` mention and no section for this area).

## 4. CHANGELOG

Under the existing `## [Unreleased]` section (it already has Added / Changed / Deprecated / Fixed subsections;
its `### Fixed` starts at line 88). Do **not** write a version header, and do not bump the version
(`inv fullrelease` does both). Two entries, matching the two kinds of change:

Append to `### Fixed`:

```markdown
* **`SingletonVersionedResource.saved_updated_singleton` raises `VersionConflictError` on a stale save** (#13
  follow-up). It previously raised a bare `ValueError("Cannot update from non-latest version")`, the one
  stale-version conflict left that could not be told apart from a programming error without matching the message.
  `VersionConflictError` subclasses `ValueError` and the message is unchanged, so existing handlers keep working;
  it now also carries `expected_version` / `actual_version`.
```

Add a `### Internal` subsection to `## [Unreleased]` (none exists there yet; 22.0.0 uses `### Internal` for
non-library changes) with the docs item from 3(b):

```markdown
### Internal

* **`AGENT_KNOWLEDGE.md` documents `ResourceRepository.save` and the real exception hierarchy** (#13). Adds the
  missing `save(id, obj_in, expected_version)` section and corrects the hierarchy tree, which still described
  `VersionConflictError` as a back-compat alias rather than a `ConflictError` subclass. Docs only; no library changes.
```

If 3(b) is split into its own PR, move this `### Internal` bullet with it.

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
| `tests/test_typed_conflicts_and_save.py` | new `VersionedSettings` model + 4 module-level tests (3 DynamoDB, 1 LocalStorage) |
| `AGENT_KNOWLEDGE.md` | singleton note; new `repo.save` subsection; corrected exception hierarchy tree |
| `CHANGELOG.md` | one `### Fixed` bullet and one `### Internal` (docs) bullet under `## [Unreleased]` |
