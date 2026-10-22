# Plan: issue #13 — typed conflict errors and `save(expected_version)`

## Summary: no code change. Verify, then close the issue.

The issue was written against 19.0.0. Everything it asks for shipped in **21.0.0** (2026-08-24) in
commit `7bfdeb0` ("Typed conflict errors and a save(expected_version) primitive (#13)"). That commit
is in tags `21.0.0` and `22.0.0` and is an ancestor of this branch's HEAD (`a82a3f6`). The issue was
never closed. I re-checked each point against the current source; the evidence is below.

This plan has three parts: no source edits, no new tests, and no CHANGELOG entry. The work is
to run the existing tests against DynamoDB Local and then close #13 with the comment in step 3.

## Evidence: issue point → current code

| Issue asks for | Where it is now |
|---|---|
| `transact_write_safe` raises a typed conflict for `ConditionalCheckFailed` instead of a bare `ValueError` | `src/simplesingletable/dynamodb_memory.py:296-327`. It raises `VersionConflictError(message, cancellation_reasons=...)` when any cancellation reason has code `ConditionalCheckFailed`. Other cancellations still raise a bare `ValueError`. The message text is unchanged. |
| `update_existing` raises `VersionConflictError` instead of `ValueError("Cannot update from non-latest version")` | `dynamodb_memory.py:576-582`. Same message, plus `resource_type`, `resource_id`, `expected_version` and `actual_version`. |
| Types live in `exceptions.py` and stay `ValueError` subclasses | `src/simplesingletable/exceptions.py`. `ConflictError(ValueError)` at `:43`, `ResourceExistsError(ConflictError)` at `:66`, `VersionConflictError(TransactionConditionFailedError)` at `:127`. `TransactionConditionFailedError` is both a `TransactionError` and a `ConflictError`. `transactions.py` re-exports the same objects, and all of them are exported from the package root. |
| A versioned create on a taken id is distinguishable | `dynamodb_memory.py` versioned create re-raises the conditional-put failure as `ResourceExistsError`. |
| `repo.save(obj, expected_version)`: `0` creates conditionally, `>0` updates from that exact version, a typed conflict either way | `src/simplesingletable/extras/repository.py:183`, `ResourceRepository.save(id, obj_in, expected_version, clear_fields=None, changed_by=None, audit_metadata=None) -> T`. `VersionedResourceRepository` inherits it. `0` makes a conditional create, and an existing item raises `ResourceExistsError`. `>0` reads fresh, skipping the repository cache, then raises `VersionConflictError` if the item is absent or at another version. Otherwise it updates, and the write is conditioned on that version. A negative version raises `ValueError` and a non-versioned model raises `TypeError`. |
| `LocalStorageMemory` parity (implied) | `src/simplesingletable/local_storage_memory.py` raises the same types. |

**The shipped API differs from the issue's proposal in two intentional ways.**

1. **Signature.** The shipped method is `save(id, obj_in, expected_version)`, not
   `save(obj, expected_version)`. The issue's TOCTOU concern is still covered, because the
   DynamoDB write is conditioned on `#version = :previous_version` (`_update_existing_versioned`)
   and not on the pre-read. The pre-read only provides the pre-image and lets the call fail early.
   The 21.0.0 CHANGELOG records this decision, so it is not a gap.
2. **Exception type.** The issue asks for "`VersionConflictError` either way". That is not what
   shipped. `expected_version=0` on a taken id raises `ResourceExistsError`, and `>0` raises
   `VersionConflictError`. These are siblings under `ConflictError`: `ResourceExistsError` is not a
   `VersionConflictError` (`exceptions.py:66` vs `:127`; pinned by
   `test_resource_exists_is_a_conflict_but_not_a_transaction_error` and
   `test_save_with_expected_version_zero_refuses_to_clobber`). The common handler for both modes
   is therefore `except ConflictError:`. A caller who writes the issue's `except VersionConflictError:`
   will **not** catch a lost create race. This is deliberate: a taken id is not a transaction
   condition failure. The close comment in step 3 says it outright so callers migrating off
   `_is_lost_race` pick the right class.

## Files to change

None.

## Public API changes

None. For reference, this is what 21.0.0 already shipped as a breaking release:

```python
# simplesingletable / simplesingletable.exceptions
class ConflictError(ValueError): ...
class ResourceExistsError(ConflictError): ...
class TransactionConditionFailedError(TransactionError, ConflictError): ...
class VersionConflictError(TransactionConditionFailedError): ...

# simplesingletable.extras.repository.ResourceRepository
def save(
    self,
    id: Any,
    obj_in: CreateSchemaType | UpdateSchemaType | dict,
    expected_version: int,
    clear_fields: set[str] | None = None,
    changed_by: str | None = None,
    audit_metadata: dict | None = None,
) -> T: ...
```

## Tests

No new tests. Coverage already exists in `tests/test_typed_conflicts_and_save.py`, which has 23
tests:

- the hierarchy and back-compat `isinstance` checks
- re-exports
- structured context
- a stale `update_existing`
- a create with a taken id
- losing the race at write time
- a transaction condition failure caught as `ConflictError`
- `LocalStorageMemory` parity
- every `save` mode: create, clobber refused, current version, stale version, missing, schema per mode, `clear_fields`, negative version, non-versioned model
- cache bypass
- an explicit test that `save` replaces the issue's `_is_lost_race` message matching
- `TypeError` for the wrong class

Related suites: `test_transaction_loud_failures.py`, `test_transaction_optimistic_put.py`,
`test_repository.py`, `test_versioned_repository.py` and `test_local_storage.py`.

### Step 1: run the tests (needs Docker)

The investigation could not start DynamoDB Local. Only the 5 pure type tests ran; the 18 DB-backed
ones failed at fixture setup. Run on a machine where Docker is available:

```bash
uv run pytest tests/test_typed_conflicts_and_save.py tests/test_transaction_loud_failures.py \
  tests/test_transaction_optimistic_put.py tests/test_repository.py tests/test_versioned_repository.py \
  tests/test_local_storage.py
```

Expected result: everything passes. If a test fails, treat it as a regression since 21.0.0 and
open a new bug with the failing output. Do not reopen this plan to fix it.

## CHANGELOG

**No entry under `## [Unreleased]`.** The work is already recorded under `## [21.0.0] 2026-08-24`
(about lines 219–310: "A conflict hierarchy, rooted at `ConflictError` (#13)",
"`ResourceRepository.save(id, obj_in, expected_version)` (#13)", and "19 wrong-class call sites
now raise `TypeError` (#13)"). Adding another entry would show the work as shipping twice.

## Step 2: no branch, PR or release

Nothing gets committed. The branch `dispatch/13-typed-conflict-errors-on-the-non-transac` can be
deleted without merging.

## Step 3: close #13

Close the issue only after step 1 passes:

```bash
gh issue close 13 --comment "$(cat <<'EOF'
Shipped in 21.0.0 (7bfdeb0); see the 21.0.0 CHANGELOG.

- Non-transactional conflicts are typed. `transact_write_safe` raises `VersionConflictError` on
  `ConditionalCheckFailed`, `update_existing` raises `VersionConflictError` from a stale pre-image,
  and a versioned create on a taken id raises `ResourceExistsError`. All of these are `ConflictError`,
  which subclasses `ValueError`, so existing `except ValueError` handlers still work.
- `ResourceRepository.save(id, obj_in, expected_version)`: `0` creates conditionally, and `>0`
  updates only from that version. The signature takes the id rather than a pre-read object. The
  write is conditioned on `expected_version`, so the TOCTOU gap is closed either way.
- Unlike the proposal here, the two modes raise different classes: `expected_version=0` raises
  `ResourceExistsError` and `>0` raises `VersionConflictError`. Both are `ConflictError`, so catch
  `ConflictError`, not `VersionConflictError`; `except VersionConflictError:` misses a lost create.

Replace the message-matching workaround with `except ConflictError:`.
EOF
)"
```

## Out of scope (separate issues if wanted)

1. A non-versioned `save` / conditional create for `DynamoDbResource`. Today it raises `TypeError`,
   and the 21.0.0 CHANGELOG lists it as "not covered yet".
2. A `save(obj)` convenience form that takes `expected_version` from `obj.version`. Low value.
3. The `VersionConflictError` from `transact_write_safe` has no `resource_type` / `resource_id`. It
   only knows the cancellation index, which is acceptable as it is.
