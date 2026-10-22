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
| Tests | `tests/test_typed_conflicts_and_save.py` (379 lines, 20+ tests) | done, but DynamoDB-backed tests were never run in the investigation (Docker was down) |

Note the issue's `_is_lost_race` string match in `ResourceRepository` is a consumer-side workaround. Nothing in this library needs to change for it. The CHANGELOG already tells users to delete it.

## 2. The remaining defect: eventually consistent reads on the save/update path

`DynamoDbMemory.get_existing` (`dynamodb_memory.py:371-376`) and `read_existing` (`:526-533`) both default to `consistent_read=False`.

Two reads on the `save(expected_version=N>0)` path use that default:

- **A.** `extras/repository.py:246`: `existing = self.ddb.get_existing(id, self.model_class)`. This is the pre-image guard.
- **B.** `dynamodb_memory.py:571-574` in `update_existing`, the versioned branch: `latest_resource = self.read_existing(existing_id=..., data_class=data_class)`. This is the latest-version check. `ResourceRepository.update()` goes through it too.

**Failure mode.** Shortly after a write, an eventually consistent read can return the previous version. The caller then gets a false `VersionConflictError` (a false 409) for a write that did not race anyone. The version-conditioned write (`_update_existing_versioned`) is unchanged, so this is never a lost update. Only the false positive is wrong.

**Why both reads.** Fixing only A still leaves B stale. Scenario: A reads v3 correctly, then B reads v2 from a lagging replica, `existing.version (3) != latest.version (2)`, and the call raises. So both must change.

**Decision:** pass `consistent_read=True` to both reads.

**Cost:** one strongly consistent read per versioned update, which costs 2× the read capacity units of an eventually consistent read. This applies to every versioned `update()` and `save()`, not only `save()`. Record it in the CHANGELOG (section 5). `LocalStorageMemory` reads are file reads and are always consistent, so it needs no change.

## 3. Decisions on the two deviations from the issue text

Both are settled here. The second engineer should not reopen them.

**D1. Signature stays `save(id, obj_in, expected_version, ...)`.** The issue proposed `save(obj_in, expected_version)`. The id is required because `expected_version=0` is a create and there is no existing object to take an id from. The guard is still an integer compared against a fresh read, plus a version-conditioned write, so the TOCTOU concern is addressed. Keep it. Update the issue text rather than the code.

**D2. A create collision raises `ResourceExistsError`, not `VersionConflictError`.** The issue said "`VersionConflictError` either way". `ResourceExistsError` is a sibling under `ConflictError`, and it shipped in 21.0.0. Changing it now would break `except ResourceExistsError` callers. Callers who write `except ConflictError` catch both, which is the stated goal. Keep it. The `save` docstring already documents this.

## 4. Files to change

### 4.1 `src/simplesingletable/extras/repository.py`

Line 246. Make the pre-image read strongly consistent and say why in the comment.

```python
        # Deliberately bypasses the repository cache: a cached pre-image could be
        # stale, which would turn the guard into a rubber stamp. Strongly consistent
        # for the same reason: an eventually consistent read can return the version
        # before our own last write and report a conflict that did not happen.
        existing = self.ddb.get_existing(id, self.model_class, consistent_read=True)
```

No signature change. The docstring's `Raises:` section is already correct.

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

Add under `## [Unreleased]` in `CHANGELOG.md`, after the existing `### Fixed` block (lines 81-~100) or create `### Fixed` if absent. Do **not** add a new #13 entry or a `## [X.Y.Z]` header.

```markdown
* **`ResourceRepository.save` and `update()` could raise a spurious `VersionConflictError` right after a write.**
  The pre-image read in `save` and the latest-version check in `DynamoDbMemory.update_existing` used
  DynamoDB's eventually consistent reads. A read that landed on a replica still holding the previous
  version reported a conflict that did not exist. Both reads are now strongly consistent. The
  conditional write is unchanged, so a concurrent writer is still rejected; only the false positive is
  removed. Each versioned update now uses a strongly consistent read, which costs twice the read
  capacity units of the previous eventually consistent read.
```

Check the existing `### Fixed` block's style before writing. Match its bullet and bold-lead pattern.

## 6. Tests to add

All in `tests/test_typed_conflicts_and_save.py`, reusing its existing `repo` fixture (line 66, built on `dynamodb_memory`) and its model and schema definitions. These run against DynamoDB Local, so Docker must be running (see section 7).

**T1. `test_save_pre_image_read_is_strongly_consistent`**

Spy on `dynamodb_memory.get_existing` with `unittest.mock.patch.object(..., wraps=...)`. Create a resource at version 1, then call `repo.save(ID, update_obj, expected_version=1)`. Assert that the `get_existing` call made by `save` has `consistent_read=True`.

**T2. `test_update_existing_latest_read_is_strongly_consistent`**

Pre-read the object with `existing = dynamodb_memory.get_existing(ID, Model)` **before** installing the spy, so the spy only sees the calls made by the update. Spy on `dynamodb_memory.read_existing` with `wraps=`, then call `dynamodb_memory.update_existing(existing, changes)`. Assert that the first `read_existing` call has `consistent_read=True`.

**T3. `test_save_is_not_fooled_by_a_lagging_read` (regression, fails before the fix)**

DynamoDB Local is always consistent, so reproduce the lag by faking it. This is the main regression test.

```python
def test_save_is_not_fooled_by_a_lagging_read(repo, dynamodb_memory, monkeypatch):
    repo.save(ID, create_obj, expected_version=0)           # v1
    stale = dynamodb_memory.get_existing(ID, repo.model_class)  # snapshot of v1
    repo.save(ID, update_obj, expected_version=1)           # v2 written

    real_get = dynamodb_memory.get_existing
    real_read = dynamodb_memory.read_existing

    def lagging(real, stale_value):
        def fake(*args, consistent_read=False, **kwargs):
            # Only the default (eventually consistent) read sees the lagging replica.
            return real(*args, consistent_read=True, **kwargs) if consistent_read else stale_value
        return fake

    monkeypatch.setattr(dynamodb_memory, "get_existing", lagging(real_get, stale))
    monkeypatch.setattr(dynamodb_memory, "read_existing", lagging(real_read, stale))

    result = repo.save(ID, update_obj, expected_version=2)
    assert result.version == 3
```

Before the fix, `save`'s pre-image read returns v1 and the call raises `VersionConflictError`. After the fix, both reads are consistent, and it returns v3.

Note: `fake` receives no `self`, because the patched attributes are instance attributes and not bound methods. The signature above already takes that into account.

**T4. `test_update_is_not_fooled_by_a_lagging_read` (regression, fails before the fix)**

The same as T3 but with `repo.update(ID, update_obj)` (see `extras/repository.py:146` for the signature). It isolates the `update()` path, which uses `update_existing` but not the `save` pre-image.

**T5. `test_genuine_concurrent_write_still_conflicts`**

Guard against over-correcting. Pre-read `existing` at v2. Write v3 normally with `repo.update`. Then call `repo.save(ID, obj, expected_version=2)`. Assert `VersionConflictError`, and that `expected_version == 2` and `actual_version == 3`. No fakes are needed. This shows that strong reads still detect real conflicts.

**T6. Local backend parity: none needed.** Local reads are file reads and always consistent.

Run T3 and T4 against the unmodified code first, and confirm they fail with `VersionConflictError`. That proves they test what they claim.

## 7. Verification

1. Start Docker, then run `pytest tests/test_typed_conflicts_and_save.py -v`. The 18 Docker-backed tests from the investigation have never executed, so this is the first real run. Every test in the file must pass, not only the new ones.
2. Confirm T3 and T4 fail before section 4 is applied, and pass after.
3. Run the full suite: `pytest`. Watch `tests/test_local_storage.py` (asserts old message text) and `tests/test_blob_storage_bugfixes.py:101-113` (asserts non-latest version message). Both should still pass, since message text is unchanged.
4. Run `inv lint`.
5. No dependency change, so `uv lock --check` is not affected.
6. Confirm `CHANGELOG.md` still has exactly one `## [Unreleased]` heading and no new version header.

## 8. Issue #13 closure

- Comment on #13 that the feature shipped in **21.0.0** (`7bfdeb0`), with the issue's line references updated (`dynamodb_memory.py:190`, `:442`, and `transactions.py:809` no longer exist).
- Record decisions D1 and D2 as accepted, with the rationale from section 3.
- Close #13 when the section 4 fix is released. The release is a separate step (`inv fullrelease`), not part of this plan.
- Tell the #10 consumer to delete `_is_lost_race` and catch `ConflictError`, which the 21.0.0 CHANGELOG already says.

## 9. Summary of changes

| File | Change |
|---|---|
| `src/simplesingletable/extras/repository.py:246` | `consistent_read=True` on the pre-image read; comment |
| `src/simplesingletable/dynamodb_memory.py:571-574` | `consistent_read=True` on the latest-version read |
| `tests/test_typed_conflicts_and_save.py` | add T1-T5 |
| `CHANGELOG.md` (`## [Unreleased]`, `### Fixed`) | one entry from section 5 |
| `AGENT_KNOWLEDGE.md` (~1773-1781, ~441-483) | correct hierarchy; add `save` subsection |
| No public signature changes | `save`, `update_existing`, `get_existing`, and the exception classes are unchanged |
