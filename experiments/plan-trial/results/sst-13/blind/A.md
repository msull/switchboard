# Plan: #13 — typed conflict errors and `save(expected_version)`

## 0. State of play (read first)

The issue body was written against 19.0.0. Both parts of #13 are **already implemented and released**:

- Commit `7bfdeb0` ("Typed conflict errors and a save(expected_version) primitive (#13)", 2026-08-24) is an ancestor of `HEAD`.
- Its CHANGELOG entries sit under `## [21.0.0] 2026-08-24` (`CHANGELOG.md:244-290`), not under `## [Unreleased]`.
- Current version is `22.0.0` (`pyproject.toml:7`). `git tag --contains 7bfdeb0` lists `21.0.0` and `22.0.0`.

The investigation notes at `tickets/c3a45d04/investigate/1/root/notes.md` are mostly right, with three corrections:

1. The #13 CHANGELOG entries are under 21.0.0, not Unreleased (see above).
2. The eventually consistent latest-version read in `update_existing` is at `dynamodb_memory.py:571-574`. The consistent read at `:589-594` is the post-write read, which is already strongly consistent.
3. Notes deviation 3 is real and is the only defect left. It is in scope (section 2).

So the remaining work is one consistency fix, two decisions recorded, two doc corrections, and the tests. Nothing here is a new feature, and no version bump or release is planned (`inv fullrelease` does that).

## 1. What is already in place (verified, do not redo)

| Symbol | Location | Status |
|---|---|---|
| `ConflictError(ValueError)` | `src/simplesingletable/exceptions.py:43` | done |
| `ResourceExistsError(ConflictError)` | `exceptions.py:66` | done |
| `TransactionConditionFailedError(TransactionError, ConflictError)` | `exceptions.py:96` | done |
| `VersionConflictError(TransactionConditionFailedError)` with `expected_version`, `actual_version` | `exceptions.py:127` | done |
| Re-exports from `transactions.py`, and from package root | `transactions.py`, `src/simplesingletable/__init__.py` | done |
| `transact_write_safe` raises `VersionConflictError` on `ConditionalCheckFailed` | `dynamodb_memory.py:296-327` | done |
| `update_existing` raises `VersionConflictError` on version mismatch | `dynamodb_memory.py:575-582` | done |
| Local backend raises the same types | `local_storage_memory.py` (~434, 502, 557, 568) | done |
| `create_new` raises `ResourceExistsError` on a taken id | `dynamodb_memory.py:1098-1130` | done |
| `ResourceRepository.save(id, obj_in, expected_version, clear_fields=None, changed_by=None, audit_metadata=None)` | `extras/repository.py:183-265` | done |
| Tests | `tests/test_typed_conflicts_and_save.py` (379 lines, 23 tests: 18 Docker-backed, 1 local-only, 4 with no fixture) | done, but the Docker-backed tests were never run in the investigation (Docker was down) |

Note the issue's `_is_lost_race` string match in `ResourceRepository` is a consumer-side workaround. Nothing in this library needs to change for it. The CHANGELOG already tells users to delete it.

## 2. The remaining defect: eventually consistent reads on the write path

`DynamoDbMemory.get_existing` (`dynamodb_memory.py:371-376`) and `read_existing` (`:526-533`) both default to `consistent_read=False`.

Three reads on the write path use that default:

- **A.** `extras/repository.py:246`: `existing = self.ddb.get_existing(id, self.model_class)`. This is the `save(expected_version=N>0)` pre-image guard.
- **B.** `dynamodb_memory.py:571-574` in `update_existing`, the versioned branch: `latest_resource = self.read_existing(existing_id=..., data_class=data_class)`. This is the latest-version check. Every versioned `update(obj)` and `save` goes through it.
- **C.** `update(id)` only. `extras/repository.py:178` calls `self.read(id)`, which calls `get()` (`:128-133`), which calls `_get()` (`:351-358`). `_get` checks the cache, then calls `get_existing` with the default. So `update(id)` gets its pre-image from the same stale read as A. `update(obj)` skips this read, because the caller supplies the object.

**Failure mode.** Shortly after a write, an eventually consistent read can return the previous version. The caller then gets a false `VersionConflictError` (a false 409) for a write that did not race anyone. The version-conditioned write (`_update_existing_versioned`) is unchanged, so this is never a lost update. Only the false positive is wrong.

**Why all three.** Fixing only A still leaves B stale. Scenario: A reads v3 correctly, then B reads v2 from a lagging replica, `existing.version (3) != latest.version (2)`, and the call raises. Fixing only C leaves `update(id)` stale for the same reason. So all three must change.

**Decision:** pass `consistent_read=True` to all three reads.

**Cache on the `update(id)` path (C).** The cache is not bypassed. A cache hit can be stale only if another process wrote the record after it was cached. That is a genuine conflict, the behaviour the library already had, and it is not replica lag, so it is out of scope. Bypassing the cache would also remove every cache hit from `update(id)`. T4 therefore clears the cache before it runs, so it exercises the miss path (section 6).

**Cost:** one strongly consistent read per versioned update, which costs 2× the read capacity units of an eventually consistent read. This applies to every versioned `update()` and `save()`, not only `save()`, and to every `update(id)` cache miss. Record it in the CHANGELOG (section 5). `LocalStorageMemory` reads are file reads and are always consistent, so it needs no change.

## 3. Decisions on the two deviations from the issue text

Both are settled here. The second engineer should not reopen them.

**D1. Signature stays `save(id, obj_in, expected_version, ...)`.** The issue proposed `save(obj_in, expected_version)`. The id is required because `expected_version=0` is a create and there is no existing object to take an id from. The guard is still an integer compared against a fresh read, plus a version-conditioned write, so the TOCTOU concern is addressed. Keep it. Update the issue text rather than the code.

**D2. A create collision raises `ResourceExistsError`, not `VersionConflictError`.** The issue said "`VersionConflictError` either way". `ResourceExistsError` is a sibling under `ConflictError`, and it shipped in 21.0.0. Changing it now would break `except ResourceExistsError` callers. Callers who write `except ConflictError` catch both, which is the stated goal. Keep it. The `save` docstring already documents this.

## 4. Files to change

Before editing `repository.py`, confirm the subclass facts this section relies on. `VersionedResourceRepository` (`versioned_repository.py:71`) subclasses `ResourceRepository` and does not override `_get`, `get`, `read` or `update`. `ReadOnlyResourceRepository` (`readonly_repository.py:46`) is a separate class with no `update`, so it is unaffected.

### 4.1 `src/simplesingletable/extras/repository.py`

**Change 1 (path A).** Line 246. Make the pre-image read strongly consistent and say why in the comment.

```python
        # Deliberately bypasses the repository cache: a cached pre-image could be
        # stale, which would turn the guard into a rubber stamp. Strongly consistent
        # for the same reason: an eventually consistent read can return the version
        # before our own last write and report a conflict that did not happen.
        existing = self.ddb.get_existing(id, self.model_class, consistent_read=True)
```

No signature change to `save`. The docstring's `Raises:` section is already correct.

**Change 2 (path C).** `_get` gains a `consistent_read` keyword with a default, so `get()` and every other caller are unchanged:

```python
    def _get(self, id: Any, consistent_read: bool = False) -> T | None:
        if self._cache:
            cached = self._cache.get(str(id))
            if cached is not None:
                return cached
        result = self.ddb.get_existing(id, self.model_class, consistent_read=consistent_read)
        if result is not None and self._cache:
            self._cache.put(str(id), result)
        return result
```

In `update()` (`:146-180`), replace the `else` branch that currently calls `self.read(id_or_obj)` (`:177-178`):

```python
        if isinstance(id_or_obj, self.model_class):
            existing = id_or_obj
        else:
            existing = self._get(id_or_obj, consistent_read=True)
            if existing is None:
                raise ValueError(f"{self.model_class.__name__} with id {id_or_obj} not found")
```

The not-found message must match `read()`'s (`:135-141`), so callers that match on it keep working. `read()` itself does not change.

### 4.2 `src/simplesingletable/dynamodb_memory.py`

Lines 571-574 in `update_existing`, versioned branch:

```python
            latest_resource = self.read_existing(
                existing_id=existing_resource.resource_id,
                data_class=data_class,
                consistent_read=True,
            )
```

No signature change. Nothing else in this file needs to change.

### 4.3 `AGENT_KNOWLEDGE.md`

- **Line 1781** says `# │   └── VersionConflictError              # Back-compat alias.`, which is wrong. `VersionConflictError` is a subclass that also sits under `ConflictError`, not an alias. Replace the hierarchy block at roughly lines 1773-1781 with the real tree:

  ```python
  # ConflictError(ValueError)                 # Root of the conflict family.
  # ├── ResourceExistsError                   # Conditional create hit an existing item.
  # └── TransactionConditionFailedError       # Also a TransactionError.
  #     └── VersionConflictError              # A write lost a version race.
  # TransactionError(Exception)               # Transaction failure.
  ```

  Keep the import block at 1774-1777 and add `ConflictError` and `ResourceExistsError` to it. Import them from `simplesingletable.exceptions` or the package root, as the code at line 441 already does.

- **Add a `save` subsection** next to the existing `except ConflictError` example (lines ~441-483). Keep it short:

  ```python
  try:
      doc = repo.save(doc_id, changes, expected_version=known_version)  # 0 = must not exist
  except ConflictError:
      ...  # re-read and retry, or return 409
  ```

  It should say: `0` creates and raises `ResourceExistsError` if the id is taken. `>0` updates from that exact version and raises `VersionConflictError` if the resource is missing or at another version. Both are `ConflictError`.

### 4.4 Not changed

- `exceptions.py`, `transactions.py`, `__init__.py`, `local_storage_memory.py`: no change.
- `README.md`: no change. It does not mention these types.
- `pyproject.toml`, `src/simplesingletable/__init__.py` version, `uv.lock`: no change (no version bump, no dependency change).

## 5. CHANGELOG entry

Add as the last bullet of the existing `### Fixed` block under `## [Unreleased]`. That block starts at `CHANGELOG.md:88` (the heading) and its last line is `:106` (`find them.`, the end of the #18 entry). Insert the new bullet after line 106. Leave the blank line at 107 before `## [22.0.0] 2026-09-30` at line 108. The `### Fixed` heading already exists, so do not create another one. Do **not** add a new #13 entry or a `## [X.Y.Z]` header.

Match the bullet and bold-lead style of the #18 entry above it. The text:

```markdown
* **`ResourceRepository.save` and `update(id)` could raise a spurious `VersionConflictError` right after a write.**
  The pre-image read in `save`, the pre-image read in `update(id)` (on a cache miss), and the latest-version
  check in `DynamoDbMemory.update_existing` used DynamoDB's eventually consistent reads. A read that landed
  on a replica still holding the previous version reported a conflict that did not exist. All three reads
  are now strongly consistent. The conditional write is unchanged, so a concurrent writer is still rejected;
  only the false positive is removed. Each versioned update now uses a strongly consistent read, which costs
  twice the read capacity units of the previous eventually consistent read.
```

## 6. Tests to add

All in `tests/test_typed_conflicts_and_save.py`, reusing its existing `repo` fixture (line 66, built on `dynamodb_memory`) and its model and schema definitions. These run against DynamoDB Local, so Docker must be running (see section 7).

The file has 23 tests today: 18 use a Docker-backed fixture (`repo` or `dynamodb_memory`), 1 uses `local_storage` only, and 4 use no fixture. The new tests are added alongside them.

Names in the test code below are not module-level. They are defined inside each test, as the existing file does with literal ids such as `"doc-1"`. Define them in each test: `ID = "doc-1"`, `create_obj = {"title": "t", "content": "c"}`, `update_obj = {"content": "c2"}`. The model class is `Doc`, which is also `repo.model_class`. Payloads are dicts, which `save` and `update` validate against `CreateDoc` and `UpdateDoc`.

**T1. `test_save_pre_image_read_is_strongly_consistent`**

Spy on `dynamodb_memory.get_existing` with `unittest.mock.patch.object(..., wraps=...)`. Create a resource at version 1, then call `repo.save(ID, update_obj, expected_version=1)`. Assert that the `get_existing` call made by `save` has `consistent_read=True`. Then call `repo.clear_cache()` and `repo.update(ID, update_obj)` under the same spy. `update(ID)` calls `get_existing` more than once: for its pre-image (path C), and again through `read_existing` (`dynamodb_memory.py:541`) for the latest-version check and the post-write read. Assert that every `get_existing` call made during that update has `consistent_read=True`, so the check does not depend on call order. This checks the calls directly. T4 checks the behaviour.

**T2. `test_update_existing_latest_read_is_strongly_consistent`**

Pre-read the object with `existing = dynamodb_memory.get_existing(ID, Doc)` **before** installing the spy, so the spy only sees the calls made by the update. Spy on `dynamodb_memory.read_existing` with `wraps=`, then call `dynamodb_memory.update_existing(existing, changes)`. Assert that the first `read_existing` call has `consistent_read=True`.

**Shared helper for T3 and T4** (module level in the test file):

```python
def _lag_latest_reads(monkeypatch, ddb, record_id, model_class, stale):
    """Make the eventually consistent latest-version read of one record return ``stale``.

    DynamoDB Local is always strongly consistent, so the lag is simulated. Only default
    reads (``consistent_read=False``) of ``version=0`` for ``record_id`` and ``model_class``
    are lagged. Every other read passes through unchanged.
    """

    def lag(real):
        def fake(existing_id, data_class, version=0, consistent_read=False, **kwargs):
            if not consistent_read and version == 0 and data_class is model_class and existing_id == record_id:
                return stale
            return real(existing_id, data_class, version=version, consistent_read=consistent_read, **kwargs)

        return fake

    monkeypatch.setattr(ddb, "get_existing", lag(ddb.get_existing))
    monkeypatch.setattr(ddb, "read_existing", lag(ddb.read_existing))
```

Both patched methods are called with the same leading parameters, so one fake covers the pre-image read (A) and the latest-version read (B). The patch is set on the instance, so `real` is the bound method captured before the patch.

**T3. `test_save_is_not_fooled_by_a_lagging_read` (regression, fails before the fix)**

```python
def test_save_is_not_fooled_by_a_lagging_read(repo, dynamodb_memory, monkeypatch):
    repo.save(ID, create_obj, expected_version=0)                 # v1
    stale = dynamodb_memory.get_existing(ID, repo.model_class)    # snapshot of v1, taken before the lag starts
    repo.save(ID, update_obj, expected_version=1)                 # v2 written

    _lag_latest_reads(monkeypatch, dynamodb_memory, ID, repo.model_class, stale)

    result = repo.save(ID, update_obj, expected_version=2)
    assert result.version == 3
```

Before the fix, `save`'s pre-image read (A) returns the lagged v1. The guard at `extras/repository.py:247` (`existing.version != expected_version`, with `expected_version=2`) raises `VersionConflictError` before `_update` runs, so the write is never reached. There is no genuine race. After the fix, the pre-image read is consistent, returns v2, the guard passes, and the call returns v3. The conditional write is not what T3 tests. T4 covers it.

**T4. `test_update_by_id_is_not_fooled_by_a_lagging_read` (regression, fails before the fix)**

The same lag as T3, but through `update(ID, ...)` by id, which is path C. The repository cache is cleared first. A cache hit would return the fresh v2 and skip the read this test is about, so the lag would never be hit.

```python
def test_update_by_id_is_not_fooled_by_a_lagging_read(repo, dynamodb_memory, monkeypatch):
    repo.save(ID, create_obj, expected_version=0)                 # v1
    stale = dynamodb_memory.get_existing(ID, repo.model_class)    # snapshot of v1, taken before the lag starts
    repo.save(ID, update_obj, expected_version=1)                 # v2 written
    repo.clear_cache()                                            # force the miss path

    _lag_latest_reads(monkeypatch, dynamodb_memory, ID, repo.model_class, stale)

    result = repo.update(ID, update_obj)
    assert result.version == 3
```

Before the fix, `update(ID)`'s pre-image (C) returns v1 and the latest-version read (B) returns v1. The conditional write fails with `VersionConflictError`. After the fix, C is consistent, so the call returns v3.

**T5. `test_genuine_concurrent_write_still_conflicts`**

Guard against over-correcting. Pre-read `existing` at v2. Write v3 normally with `repo.update(ID, ...)`. Then call `repo.save(ID, update_obj, expected_version=2)`. Assert `VersionConflictError`, and that `expected_version == 2` and `actual_version == 3`. No fakes are needed. This shows that strong reads still detect real conflicts.

**T6. Local backend parity: none needed.** Local reads are file reads and always consistent.

Run T3 and T4 against the unmodified code first, and confirm they fail with `VersionConflictError`. That proves they test what they claim.

## 7. Verification

1. Start Docker, then run `pytest tests/test_typed_conflicts_and_save.py -v`. The 18 Docker-backed tests (section 6) have never executed, so this is the first real run. Every test in the file must pass, not only the new ones.
2. Confirm T1-T4 fail before section 4 is applied, and pass after. T3 and T4 fail with `VersionConflictError`. T1 and T2 fail on their `consistent_read=True` assertions, because the default read is still in place. T4 passes after section 4.1 change 2, which covers path C.
3. Run the full suite: `pytest`. Watch `tests/test_local_storage.py` (asserts old message text) and `tests/test_blob_storage_bugfixes.py:101-113` (asserts non-latest version message). Both should still pass, since message text is unchanged.
4. Run `inv lint`.
5. No dependency change, so `uv lock --check` is not affected.
6. Confirm `CHANGELOG.md` still has exactly one `## [Unreleased]` heading and no new version header.

## 8. Issue #13 closure (outward-facing: needs the user's explicit go-ahead)

Every step in this section is visible to other people, or cannot be undone. The implementer does not run any of them. Each one needs the user's explicit go-ahead, given for that specific action. Code changes and tests in sections 4-7 do not need this.

- **Needs go-ahead:** comment on #13 that the feature shipped in **21.0.0** (`7bfdeb0`), with the issue's line references updated (`dynamodb_memory.py:190`, `:442`, and `transactions.py:809` no longer exist).
- **Needs go-ahead:** record decisions D1 and D2 as accepted, with the rationale from section 3.
- **Needs go-ahead:** close #13 when the section 4 fix is released. The release is a separate step (`inv fullrelease`), not part of this plan.
- **Needs go-ahead:** tell the #10 consumer to delete `_is_lost_race` and catch `ConflictError`, which the 21.0.0 CHANGELOG already says.

## 9. Summary of changes

| File | Change |
|---|---|
| `src/simplesingletable/extras/repository.py:246` | `consistent_read=True` on the pre-image read (path A); comment |
| `src/simplesingletable/extras/repository.py:351` (`_get`) and `:177-178` (`update()`) | `_get` gains `consistent_read`; `update(id)` uses it and raises the same not-found `ValueError` as `read()` (path C) |
| `src/simplesingletable/dynamodb_memory.py:571-574` | `consistent_read=True` on the latest-version read (path B) |
| `tests/test_typed_conflicts_and_save.py` | add T1-T5 and the `_lag_latest_reads` helper |
| `CHANGELOG.md` (`## [Unreleased]`, `### Fixed`, after line 106) | one entry from section 5 |
| `AGENT_KNOWLEDGE.md` (~1773-1781, ~441-483) | correct hierarchy; add `save` subsection |
| No public signature changes | `save`, `update`, `update_existing`, `get_existing`, and the exception classes are unchanged |
