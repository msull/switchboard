# Plan: Issue #13 — typed conflict errors and `save(obj, expected_version)`

## Finding: #13 is already implemented and released

The issue says it was verified on `main` at 19.0.0. This checkout is past that: HEAD (`a82a3f6`) contains
commit `7bfdeb0` "Typed conflict errors and a save(expected_version) primitive (#13)". It shipped in
**21.0.0** (CHANGELOG `## [21.0.0] 2026-08-24`, "Added" and "Changed"), and 22.0.0 has since been released.
Every claim in the issue is stale for this tree. I read the code and tests only; I did not run the test
suite (it needs Docker for DynamoDB Local).

**Recommendation: do not implement anything for #13.** Re-implementing would duplicate existing exceptions,
tests and a CHANGELOG entry. Close the issue (or have the owner confirm the two deviations below are
acceptable). Adding a second CHANGELOG entry under `## [Unreleased]` would misreport an old change as new.
There is no version bump and no release step in this plan.

## Issue requirement → where it already lives

### 1. Typed conflicts on the non-transactional path

| Issue complaint | Current state |
|---|---|
| `transact_write_safe` raised bare `ValueError("Transaction failed: …")` for `ConditionalCheckFailed` | `src/simplesingletable/dynamodb_memory.py:296-327`. When any cancellation reason has code `ConditionalCheckFailed` it raises `VersionConflictError(message, cancellation_reasons=…)`. Other cancellations still raise a bare `ValueError` (deliberate: capacity and validation problems are not conflicts). Message text is unchanged. |
| `update_existing` raised `ValueError("Cannot update from non-latest version")` | `dynamodb_memory.py:548-580`. Raises `VersionConflictError` with `expected_version=existing_resource.version`. |
| Exception home | `src/simplesingletable/exceptions.py`. Hierarchy: `ValueError` → `ConflictError` → {`ResourceExistsError`, `TransactionConditionFailedError` (also `TransactionError`) → `VersionConflictError`}. All carry `resource_type`, `resource_id`; `VersionConflictError` also carries `expected_version`, `actual_version`. |
| Keep existing `except ValueError` handlers working | Every type subclasses `ValueError`. |
| Re-exports | `transactions.py` re-exports the moved types (same objects). `ConflictError`, `ResourceExistsError`, `VersionConflictError` are exported from the package root (`__init__.py:15-57`). |
| `LocalStorageMemory` parity | `local_storage_memory.py:506-572` raises the same types. |

### 2. `save` primitive

`ResourceRepository.save` is at `src/simplesingletable/extras/repository.py:183`:

```python
def save(
    self,
    id: Any,
    obj_in: CreateSchemaType | UpdateSchemaType | dict,
    expected_version: int,
    clear_fields: set[str] | None = None,
    changed_by: str | None = None,
    audit_metadata: dict | None = None,
) -> T
```

- `expected_version == 0` → validates against the create schema and calls `_create(..., override_id=id)`. The conditional create raises `ResourceExistsError` if the item exists.
- `expected_version > 0` → reads the pre-image directly with `ddb.get_existing`, bypassing the repo cache. If it is absent or at a different version it raises `VersionConflictError` and invalidates the cache entry. Otherwise it calls `_update(existing, …)`, so the write is conditioned on that exact pre-image. This closes the TOCTOU window the issue describes.
- A negative `expected_version` raises `ValueError`. A non-versioned model raises `TypeError`.
- `VersionedResourceRepository` inherits `save`.

### Tests

`tests/test_typed_conflicts_and_save.py` (about 25 tests, run against the `dynamodb_memory` / `local_storage` fixtures) covers:

- the exception hierarchy and re-export identity;
- structured context on the exceptions;
- stale `update_existing`, taken-id create, a lost write race, and a transaction condition failure;
- `LocalStorageMemory` parity;
- `save` at version 0 (create, refuse to clobber), from the current version, from a stale version, on a missing resource, schema selection per mode, `clear_fields`, a negative version, a non-versioned model, a stale cache, and replacement of the `_is_lost_race` workaround.

### CHANGELOG

Already recorded under `## [21.0.0]`: "A conflict hierarchy, rooted at `ConflictError`" and "`ResourceRepository.save(id, obj_in, expected_version)`". **Make no change to `## [Unreleased]`.**

## Deviations from the issue text (decide, don't silently "fix")

1. **Signature.** The issue proposes `repo.save(obj_in, expected_version)`. The shipped signature is `save(id, obj_in, expected_version, …)`. A new-resource `save` needs a caller-chosen id, and update schemas carry none, so `id` is required. The issue's wording ("pass the pre-read object rather than the id") suggests accepting a model instance, which `update()` supports via `id_or_obj`. `save` does not: it always re-reads by id. That is arguably safer because it never trusts a caller-held or cached object. If the owner wants the object form anyway, the minimal change is an optional `isinstance(id, self.model_class)` branch that uses `id.version` as `expected_version`. This is not recommended without a decision.
2. **Version-0 conflicts are `ResourceExistsError`, not `VersionConflictError`.** The issue says "`VersionConflictError` either way". `ResourceExistsError` is a `ConflictError` but not a `VersionConflictError`, so `except VersionConflictError` misses create-collisions. The documented catch is `except ConflictError`. Options: leave as is (recommended; `ResourceExistsError` is more precise and the docs already point at `ConflictError`), or make `ResourceExistsError` subclass `VersionConflictError` (a hierarchy change that would need a CHANGELOG entry).
3. **Non-versioned `save` is out of scope.** `_put_nonversioned_resource` writes unconditionally, and the CHANGELOG states this.

## Optional, non-blocking follow-up

`README.md` does not mention `save` or `ConflictError` (grep finds nothing). If wanted, add a short "Optimistic writes" section showing `try: repo.save(id, changes, expected_version=v) except ConflictError: …`. Docs only: no code or test changes, and a CHANGELOG line under `## [Unreleased]` → `### Changed` only if the README is edited.

## Verification (only if someone wants to re-confirm)

```bash
docker compose -f tests/docker-compose.yml up -d   # DynamoDB Local, if conftest does not start it
pytest tests/test_typed_conflicts_and_save.py
pytest                                             # full suite
inv lint
```
