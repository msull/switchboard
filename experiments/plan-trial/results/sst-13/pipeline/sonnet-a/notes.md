# Issue #13 – investigation notes

Read: CLAUDE.md, AGENT_KNOWLEDGE.md, CHANGELOG.md, source, tests.

## Headline: already implemented and released
Commit `7bfdeb0` "Typed conflict errors and a save(expected_version) primitive (#13)" is an ancestor of
this branch (and of `main`). It shipped in **21.0.0** (CHANGELOG `## [21.0.0] 2026-08-24`); `pyproject.toml`
is at 22.0.0. The issue text ("Verified on main at 19.0.0") is stale. The branch tip (a82a3f6) has a clean
tree and no diff against that work. Nothing in the issue's scope remains to build.

## Where each part of the issue lives now
Part 1 – typed conflicts on the non-transactional path
- `src/simplesingletable/exceptions.py`: `ConflictError(ValueError)` root with `resource_type`/`resource_id`;
  `ResourceExistsError`; `TransactionError`; `TransactionConditionFailedError(TransactionError, ConflictError)`;
  `VersionConflictError` (adds `expected_version`/`actual_version`). Transaction types moved here;
  `transactions.py` re-exports the same objects. Exported from `simplesingletable/__init__.py`.
- `dynamodb_memory.py:~296` `transact_write_safe`: a cancellation containing `ConditionalCheckFailed` raises
  `VersionConflictError` (message text unchanged); other cancellations stay bare `ValueError`.
- `dynamodb_memory.py:~576` `update_existing`: stale pre-image raises `VersionConflictError("Cannot update
  from non-latest version", ...)` with structured fields.
- `dynamodb_memory.py:~1124` versioned create: the `VersionConflictError` from the transaction is
  re-raised as `ResourceExistsError`.
- `local_storage_memory.py:~502,557,568`: raises the same types.
Part 2 – `save`
- `extras/repository.py:183` `ResourceRepository.save(id, obj_in, expected_version, clear_fields, changed_by,
  audit_metadata)`. 0 -> conditional create (create schema, `override_id=id`, raises `ResourceExistsError`);
  >0 -> reads uncached pre-image, raises `VersionConflictError` if absent or at another version, otherwise
  `_update` (write conditioned on the version); cache invalidated on conflict. Non-versioned model ->
  `TypeError`; negative version -> `ValueError`. Inherited by `VersionedResourceRepository`.

## Tests
`tests/test_typed_conflicts_and_save.py` (needs DynamoDB Local via Docker): hierarchy/back-compat, stale
update, taken-id create, write-race, transaction conflicts catchable as `ConflictError`, LocalStorage parity,
and ~12 `save` cases (create, no-clobber, update, stale, missing, schema selection, clear_fields, negative,
non-versioned, no cached pre-image, replaces the `_is_lost_race` workaround). Related:
`test_transactions_v2.py`, `test_transaction_optimistic_put.py`, `test_omit_none_attributes.py`.

## Docs
CHANGELOG entries exist (21.0.0). AGENT_KNOWLEDGE.md mentions `ConflictError` (~lines 441-522) and the
hierarchy (~1776). README/docs have no `save(` mention.

## Possible residual gaps (small, optional)
1. `extras/singleton.py:41` `SingletonVersionedResource.saved_updated_singleton` still raises bare
   `ValueError("Cannot update from non-latest version")` – the exact condition the issue names. Only
   remaining stale-version `ValueError` in `src`. Fix: raise `VersionConflictError` (still a `ValueError`,
   same message). Would need a test; has no existing coverage in the new test file.
2. `dynamodb_memory.py:327` other cancellations remain bare `ValueError` (intentional per docstring).
3. Confirm AGENT_KNOWLEDGE.md documents `repo.save(...)` itself (grep for `save(` found only unrelated hits;
   `ConflictError` is covered).

## Classification if any change is made
Behaviour change: only gap 1 – a stale singleton save raises `VersionConflictError` (a `ValueError`
subclass, message identical, so existing handlers keep working). Public API: `SingletonVersionedResource.
saved_updated_singleton`. CHANGELOG: **Fixed** (bug fix, patch), under `## [Unreleased]` (never a
hand-written version header). The original issue as a whole is an enhancement (Added + Changed) and was already
recorded in 21.0.0. Recommend closing #13 as done, or scoping any follow-up to gap 1 plus a docs check.
