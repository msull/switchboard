# Issue #13 — investigation notes

## Headline: already implemented and released in 21.0.0

The issue is still **OPEN** on GitHub (`gh issue view 13`: no comments, not closed), but the
work it asks for landed in commit `7bfdeb0` ("Typed conflict errors and a save(expected_version)
primitive (#13)", 2026-08-24). That commit is in tags **21.0.0** and **22.0.0** and is an
ancestor of this branch's HEAD (`a82a3f6`). The issue text says "verified on main at 19.0.0",
so it predates the fix. CHANGELOG `## [21.0.0] 2026-08-24` already has the entries (lines
~219–310: "A conflict hierarchy, rooted at `ConflictError` (#13)",
"`ResourceRepository.save(id, obj_in, expected_version)` (#13)", and "19 wrong-class call sites
now raise `TypeError` (#13)").

**Recommendation:** no code change. Close #13 and point to 21.0.0 / `7bfdeb0`. Don't add a
CHANGELOG entry, because it would duplicate the 21.0.0 entries. The only reason to change code
is one of the optional follow-ups listed at the end.

## Issue point → current code

### 1. Typed conflicts on the non-transactional path — done

| Issue claim (19.0.0) | Now |
|---|---|
| `transact_write_safe` → `ValueError("Transaction failed: ...")` for all cancellations (`dynamodb_memory.py:190` then) | `dynamodb_memory.py:296-327`: raises `VersionConflictError` when any `CancellationReasons[].Code == "ConditionalCheckFailed"`. Other cancellations stay a bare `ValueError`. The message format is unchanged. |
| `update_existing` → `ValueError("Cannot update from non-latest version")` (`:442` then) | `dynamodb_memory.py:570-582`: raises `VersionConflictError` with the same message plus `resource_type`, `resource_id`, `expected_version`, `actual_version`. |
| (not in the issue) versioned create with a taken id | `dynamodb_memory.py:~1124`: catches the `VersionConflictError` from the `attribute_not_exists` puts and re-raises it as `ResourceExistsError`. |
| Where types live | `exceptions.py`: `ConflictError(ValueError)` → `ResourceExistsError`, and `TransactionConditionFailedError(TransactionError, ConflictError)` → `VersionConflictError`. `transactions.py:21,42` re-exports the identical objects. They are also exported from the package root (`__init__.py:15-57`). |
| LocalStorageMemory parity | `local_storage_memory.py:22,434,502,557,568` raises the same types. |

The write-side guard is real, not only the pre-read: `_update_existing_versioned`
(`dynamodb_memory.py:1164`, condition at about `:1192`) conditions the v0 put on
`#version = :previous_version`.

### 2. `save` primitive — done, with a different signature from the proposal

`src/simplesingletable/extras/repository.py:183-262`: `ResourceRepository.save(id, obj_in,
expected_version, clear_fields=None, changed_by=None, audit_metadata=None)`.

- `expected_version=0`: validates `obj_in` against the create schema, then calls `_create(...,
  override_id=id)`. A conditional create, so an existing id raises `ResourceExistsError`.
- `expected_version>0`: validates against the update schema, fetches the current item **bypassing
  the repository cache**, and raises `VersionConflictError` if the item is absent or at another
  version. Otherwise it calls `_update(existing, ...)`, which conditions the write on that version.
  On a conflict it invalidates the cache entry.
- A negative version raises `ValueError`. A non-versioned model raises `TypeError`.

How it differs from the issue's proposal `repo.save(obj, expected_version)`: the shipped API takes
`(id, obj_in, expected_version)` and does its own fresh read rather than accepting a pre-read
object. That still closes the TOCTOU hole the issue worries about, because the write is
conditioned on `expected_version`, not on whatever was read. It is a deliberate design choice
recorded in the 21.0.0 CHANGELOG, not a gap.

## Public API affected (as shipped in 21.0.0, breaking release)

- New exports: `ConflictError` and `ResourceExistsError`. `TransactionError`,
  `TransactionConditionFailedError`, `VersionConflictError` and `ResourceNotFoundError` are now
  also importable from `simplesingletable` / `simplesingletable.exceptions`.
- New method: `ResourceRepository.save(...)`, inherited by `VersionedResourceRepository`.
- Behaviour change: `TransactionConditionFailedError` / `VersionConflictError` are now also
  `ValueError`, so `except ValueError` placed before `except TransactionError` now takes the first
  branch.
- Bundled change: 19 wrong-class entry points raise `TypeError` instead of `ValueError` (TRY004
  adopted).

## CHANGELOG classification

**Enhancement.** It adds new exception types and a new API, and that is how 21.0.0 recorded it
(under `### Added` / `### Changed`, with a breaking-change preamble). The `ValueError` →
`ConflictError` retyping fixes an ergonomics problem rather than wrong behaviour: the types are
`ValueError` subclasses and the message strings are unchanged. Nothing is owed under
`## [Unreleased]`.

## Tests

- `tests/test_typed_conflicts_and_save.py` (23 tests) covers the hierarchy and back-compat
  `isinstance` checks, the re-exports, structured context, a stale `update_existing`, a taken-id
  create, losing the race at write time, transaction condition failure as `ConflictError`,
  LocalStorage parity, every `save` mode, cache bypass, the message-matching replacement, and
  `TypeError` for the wrong class.
- Related: `test_transaction_loud_failures.py`, `test_transaction_optimistic_put.py`,
  `test_repository.py`, `test_versioned_repository.py`, `test_local_storage.py`.
- **Run here:** `uv run pytest tests/test_typed_conflicts_and_save.py` gave 5 passed, 18 errors.
  All 18 errors are fixture setup failures: the `docker compose up` for DynamoDB Local is
  unavailable in this environment. The 5 passing tests are the pure type and hierarchy ones.
  The DynamoDB-backed tests were **not** verified in this session.

## Optional follow-ups (only if the maintainer wants more than closing)

1. **Non-versioned `save` / conditional create.** `save` raises `TypeError` for
   `DynamoDbResource`, because `_put_nonversioned_resource` writes without a condition. The
   21.0.0 CHANGELOG names this as "not covered yet". That would be a new enhancement, not #13.
2. A `save(obj, expected_version=obj.version)` convenience overload matching the issue's literal
   proposal. Low value: the shipped form already removes the branch and the TOCTOU risk.
3. `transact_write_safe`'s `VersionConflictError` carries no `resource_type` / `resource_id`. It
   doesn't know which item failed beyond the cancellation index. That is acceptable as is.
