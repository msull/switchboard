# Issue #13 — investigation notes

## Headline
**Already implemented and released.** Commit `7bfdeb0` "Typed conflict errors and a save(expected_version) primitive (#13)" is an ancestor of HEAD, tagged in 21.0.0 (current package_version 22.0.0). The issue text describes 19.0.0. CHANGELOG `## [21.0.0] 2026-08-24` already documents both halves. Nothing in `## [Unreleased]` relates to #13. CLAUDE.md is the only instruction file read besides this; the AGENT_KNOWLEDGE.md section list has no mention of conflicts or `save` (not checked in full beyond its first ~150 lines).

## Where the code is (already changed)
- `src/simplesingletable/exceptions.py`: `ConflictError(ValueError)` root; `ResourceExistsError(ConflictError)`; `TransactionError`; `TransactionConditionFailedError(TransactionError, ConflictError)`; `VersionConflictError` (adds `expected_version`/`actual_version`); `ResourceNotFoundError`. `transactions.py` re-exports the same objects; package root `__init__.py` exports them.
- `dynamodb_memory.py` `transact_write_safe` (~l.296): a cancellation containing `ConditionalCheckFailed` -> `VersionConflictError` (message text unchanged); other cancellations stay bare `ValueError`.
- `dynamodb_memory.py` `update_existing` (~l.576): raises `VersionConflictError("Cannot update from non-latest version", ...)` with resource_type/id and expected/actual version. Versioned create (~l.1124) converts the conflict to `ResourceExistsError`.
- `local_storage_memory.py` (~l.434, 502, 557, 568): raises the same types.
- `extras/repository.py:183` `ResourceRepository.save(id, obj_in, expected_version, clear_fields, changed_by, audit_metadata)`: 0 -> `_create(override_id=id)` conditional create (`ResourceExistsError`); >0 -> reads uncached pre-image, `VersionConflictError` if absent/mismatched, else `_update` (which is still conditioned on the stored version, so no TOCTOU). TypeError for non-versioned models, ValueError for negative version. Invalidates cache on conflict.

## Tests
`tests/test_typed_conflicts_and_save.py` (hierarchy, structured context, stale update, taken id, write-race, LocalStorage parity, all `save` modes, cache bypass, message-matching workaround replacement, TypeError change). Related: `test_blob_storage_bugfixes.py:113`, `test_local_storage.py:555` (still match old message via ValueError), `test_refactoring_features.py:174` ("Transaction failed:" text kept), `test_transactions_v2.py`.

## Behaviour change / API / classification (as shipped)
- Behaviour: lost optimistic writes on non-transactional path raise typed `ConflictError` subclasses instead of bare `ValueError` (still ValueError subclasses; messages unchanged). `TransactionConditionFailedError` became a `ValueError` too (ordering of `except` blocks can change). 21.0.0 also switched 19 wrong-class sites to `TypeError` (bundled, breaking).
- Public API: `ConflictError`, `ResourceExistsError`, `VersionConflictError`, `TransactionConditionFailedError` (root exports); `ResourceRepository.save`.
- CHANGELOG: part 1 = enhancement/behaviour change (breaking-ish), part 2 = enhancement ("Added"). Neither is a bug fix. Already recorded.

## Residual gaps vs. the issue (only if more work is wanted)
1. `extras/singleton.py:41` `saved_updated_singleton` still raises bare `ValueError("Cannot update from non-latest version")` — same condition, untyped. Cheapest remaining fix (small bug-fix-ish/consistency; would go under `[Unreleased]` -> Changed/Fixed). `test_local_storage.py`/`test_blob_storage_bugfixes.py` use `match=` on the message so they'd stay green if the message is kept.
2. Signature differs from the proposal: shipped `save(id, obj_in, expected_version)` rather than taking the pre-read object. Safe anyway (re-reads, conditions on version) — a deliberate design choice, documented.
3. `save` is versioned-only; non-versioned conditional create is explicitly deferred (`_put_nonversioned_resource` has no condition).
4. `readonly_*repository` classes don't write, so n/a. `VersionedResourceRepository` (extras/versioned_repository.py) not checked for a `save` equivalent.

## Recommendation
Close #13 as completed in 21.0.0 (or reduce it to gap 1). I did not run the test suite (needs Docker/DynamoDB Local) and made no code changes.
