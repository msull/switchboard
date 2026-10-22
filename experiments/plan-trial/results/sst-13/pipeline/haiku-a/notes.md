# Issue #13 — investigation notes

Worktree: `/Users/sbgragent/.dispatch/worktrees/ce7eaf93` (HEAD `a82a3f6`, equal to `main`; tree clean)

## Headline: this issue is already implemented

The issue was written against 19.0.0. The work shipped in **21.0.0** (2026-08-24) via commit
`7bfdeb0` ("Typed conflict errors and a save(expected_version) primitive (#13)"), which is an
ancestor of HEAD. `pyproject.toml` is now at **22.0.0**. The issue's stated bugs are no longer in
the code:

| Issue claim | Current state |
|---|---|
| `transact_write_safe` turns `ConditionalCheckFailed` into bare `ValueError` (`dynamodb_memory.py:190`) | Fixed. `dynamodb_memory.py:296-326` raises `VersionConflictError` when a cancellation carries `ConditionalCheckFailed`. Other cancellations stay a bare `ValueError`, by design. |
| `update_existing` raises `ValueError("Cannot update from non-latest version")` (`dynamodb_memory.py:442`) | Fixed. `dynamodb_memory.py:575-583` raises `VersionConflictError` with `expected_version` / `actual_version` / `resource_type` / `resource_id`. LocalStorageMemory does the same at `local_storage_memory.py:502`. |
| `_is_lost_race` message matching is needed | Not present anywhere in `src/` or `tests/`. Callers can catch `ConflictError`. |
| No `save(obj, expected_version)` | Implemented: `ResourceRepository.save` at `src/simplesingletable/extras/repository.py` (search `def save`). Inherited by `VersionedResourceRepository`. |

The typed hierarchy is in `src/simplesingletable/exceptions.py`:

```
ValueError
└── ConflictError                        (resource_type, resource_id)
    ├── ResourceExistsError              conditional create hit an existing item
    └── TransactionConditionFailedError  (also TransactionError)
        └── VersionConflictError         (expected_version, actual_version)
```

`transactions.py` re-exports the same objects (`transactions.py:18-42`), so existing imports are unchanged.

## Public API changes already shipped (21.0.0)

- New: `ConflictError`, `ResourceExistsError`, `VersionConflictError` (and `TransactionError`,
  `TransactionConditionFailedError`, `ResourceNotFoundError`) exported from the package root (`__init__.py`).
- New: `ResourceRepository.save(id, obj_in, expected_version, clear_fields=None, changed_by=None, audit_metadata=None)`.
  - `0` creates through the create schema, with `ResourceExistsError` if the id is taken.
  - `>0` validates against the update schema and requires the stored version to equal `expected_version`. Otherwise it raises `VersionConflictError`, including when the resource is missing.
  - Negative values raise `ValueError`. Non-versioned models raise `TypeError`.
  - It reads with `self.ddb.get_existing`, bypassing the cache on purpose. The write is still conditioned by `_update`.
- Behaviour change: 19 wrong-class entry points now raise `TypeError` where they raised `ValueError` (TRY004 adoption, per the commit message). This is breaking for callers catching `ValueError` on those paths. Check the 21.0.0 "Changed" section before reusing this issue's wording.
- Behaviour change: `TransactionConditionFailedError` is now also a `ValueError`. Callers that relied on it not being one may notice.

Verify these against the CHANGELOG rather than trusting this list. I read the 21.0.0 "Added" block (CHANGELOG.md ~L244-300) and did not read its "Changed" block in full.

## Where things stand (the actual remaining gaps)

1. **`extras/singleton.py:41` still raises bare `ValueError("Cannot update from non-latest version")`.**
   `SingletonVersionedResource.saved_updated_singleton` is the same conflict with the old type. It was missed.
   - Fix: raise `VersionConflictError` with `expected_version=self.version`, `actual_version=existing.version`, and the resource context.
   - Compatible, since it stays a `ValueError`.
   - CHANGELOG: "Changed" (type-only). It is a consistency fix, so "Fixed" is also defensible.
   - Tests: none. `grep` for `saved_updated_singleton` and `SingletonVersionedResource` finds only `src/simplesingletable/extras/singleton.py`, so the change needs a new test.

2. **Signature differs from the issue's proposal.** The issue proposes `repo.save(obj_in, expected_version)`. The shipped form is `save(id, obj_in, expected_version, ...)`.
   - The id is required because `save` may create, and creating needs the id. The issue's shorter form cannot create with an explicit id.
   - This is a deliberate design choice, not a bug. Note it in the issue or the CHANGELOG if anyone expected the two-argument form.

3. **Non-versioned conditional create is not covered.** `_put_nonversioned_resource` writes unconditionally. The changelog says so explicitly. `save` rejects non-versioned models with `TypeError`, so this is a scope gap, not a bug in `save`. It needs a separate issue.

4. **`_build_versioned_update_items` still raises `ResourceNotFoundError`** (`transactions.py:873`). This is a plain `Exception`, not a `ConflictError`. It is the transactional update of a missing resource, which is "not found" rather than a race. Leave it, or decide explicitly. The issue does not ask for it.

5. **Message text is preserved on purpose.** Three tests still assert `match="Cannot update from non-latest version"`:
   - `tests/test_typed_conflicts_and_save.py:154`
   - `tests/test_blob_storage_bugfixes.py:113`
   - `tests/test_local_storage.py:555`

   Keep these to guard backwards compatibility. Do not switch them to typed checks as part of cleanup.

6. `get_or_create` still erases the create-vs-update distinction. That is no longer blocking, since `save` does not depend on it, but the issue's "defeats the guard" point is about hand-rolled code, not the library.

## Tests

- Relevant file: `tests/test_typed_conflicts_and_save.py`. About 25 tests cover the hierarchy, exports, structured context, update/create races, LocalStorageMemory parity, every `save` branch, cache bypass, and the wrong-class `TypeError`.
- Other coverage: `tests/test_transactions_v2.py` (VersionConflictError catching and TransactionConditionFailedError), `tests/test_transaction_optimistic_put.py`, `tests/test_transaction_condition_names.py`, `tests/test_local_storage.py:541-555`, `tests/test_blob_storage_bugfixes.py:101-113`, `tests/test_refactoring_features.py:174`, `tests/test_omit_none_attributes.py:96`.
- **Not run.** Docker is not installed here, so the DynamoDB-backed tests cannot run. `python` is not on PATH, and `/usr/bin/python3` has no pytest. Most of the file uses the `dynamodb_memory` / `repo` fixtures, so it needs Docker to run at all. The LocalStorageMemory-only tests could run after installing dev deps with `uv`.

## CHANGELOG guidance

- The #13 entries are already under `## [21.0.0]`. Do not add a second entry for the shipped work. Do not hand-write a version header; `Unreleased` is the only place for new entries (per CLAUDE.md).
- For gap 1 only, add a bullet under `## [Unreleased]` in "Changed" (or "Fixed"): `SingletonVersionedResource.saved_updated_singleton` raises `VersionConflictError` (a `ConflictError` / `ValueError`) instead of a bare `ValueError`. It is an enhancement with no breaking effect, and type-only.
- Gaps 2 and 3 are documentation or follow-up items, not CHANGELOG entries for this issue.

## Recommendation

Close #13 as delivered in 21.0.0, and link `7bfdeb0`. Open follow-ups for:
- the singleton conflict (gap 1, small fix);
- non-versioned conditional create (gap 3, a separate feature);
- whether `save` should accept a two-argument form (gap 2), if the team still wants it.

Re-verify on the current tree with Docker running (`pytest tests/test_typed_conflicts_and_save.py`) before closing, since I could not run it here.
