# Issue #13: Typed conflict errors + save(expected_version) — investigation notes

## Headline

Both parts of #13 are already implemented in this checkout. Commit `7bfdeb0 "Typed conflict errors and a save(expected_version) primitive (#13)"` is an ancestor of HEAD (`a82a3f6`, which equals `main`). The issue was written against 19.0.0; main is now at 22.0.0. The #13 entries sit under `## [Unreleased]` in `CHANGELOG.md` (lines ~244–290, under `### Added`), so they are not in a released version yet.

What remains is a short list of deviations from the issue text (below), one possible consistency fix (eventual-consistency reads in `save`), and verification against DynamoDB Local, which I could not run here.

## Behaviour now (what the code does)

### 1. Typed conflicts on the non-transactional path

- `src/simplesingletable/exceptions.py`
  - New `ConflictError(ValueError)`: root of the conflict family, with `resource_type`, `resource_id`.
  - New `ResourceExistsError(ConflictError)`: conditional create hit an existing item.
  - `TransactionConditionFailedError(TransactionError, ConflictError)`.
  - `VersionConflictError(TransactionConditionFailedError)`: adds `expected_version`, `actual_version`.
  - The transaction types moved here from `transactions.py`; `transactions.py` re-exports them, so existing imports still work.
- `src/simplesingletable/dynamodb_memory.py:296-327` — `transact_write_safe`: a `ConditionalCheckFailed` cancellation raises `VersionConflictError`. Other cancellations still raise a bare `ValueError`. The message text is unchanged.
- `src/simplesingletable/dynamodb_memory.py:548-577` — `update_existing` (versioned branch): raises `VersionConflictError("Cannot update from non-latest version", ...)` with `expected_version`/`actual_version` set. Message text unchanged.
- `src/simplesingletable/dynamodb_memory.py:1098-1130` — `create_new` (versioned): a conflict on the `attribute_not_exists` pair becomes `ResourceExistsError`.
- `src/simplesingletable/local_storage_memory.py` (lines ~434, 502, 557, 568) raises the same types, so the two backends match.
- `src/simplesingletable/__init__.py` (lines ~9–57) exports `ConflictError`, `ResourceExistsError`, `TransactionConditionFailedError`, `VersionConflictError` from the package root.

Because every type is a `ValueError` subclass, existing `except ValueError` handlers and message-matching tests still pass.

### 2. `save()` primitive

- `src/simplesingletable/extras/repository.py:183-263` — `ResourceRepository.save(id, obj_in, expected_version, clear_fields=None, changed_by=None, audit_metadata=None)`:
  - Raises `TypeError` for a non-versioned model.
  - Raises `ValueError` for a negative `expected_version`.
  - `expected_version=0`: validates `obj_in` against the create schema and calls `_create(override_id=id)`. A taken id raises `ResourceExistsError`.
  - `expected_version>0`: validates against the update schema, reads the pre-image with `self.ddb.get_existing` (bypassing the cache), and raises `VersionConflictError` if the resource is missing or at a different version. Otherwise it calls `_update`, which goes through `DynamoDbMemory.update_existing`. That path re-reads the latest version and writes conditioned on `previous_version` (`_update_existing_versioned`, `dynamodb_memory.py:1164-1200`), so a writer that slips in is still caught.
  - Cache is invalidated on conflict.

## Public API affected

| Symbol | Change | Kind |
|---|---|---|
| `exceptions.ConflictError` | new, exported at root | addition |
| `exceptions.ResourceExistsError` | new, exported at root | addition |
| `exceptions.VersionConflictError` | now raised by `update_existing` (both backends) and `transact_write_safe`; `expected_version`/`actual_version` populated on the update path | type change, still a `ValueError` |
| `dynamodb_memory.transact_write_safe` (module-level) | raises `VersionConflictError` for condition failures | type change, still a `ValueError` |
| `DynamoDbMemory.update_existing`, `LocalStorageMemory.update_existing` | same as above | type change |
| `ResourceRepository.save` | new | addition |
| `ResourceRepository.create/update` | propagate the typed errors (no signature change) | behaviour is typed, not new |

Unchanged: `ResourceRepository.read` still raises `ValueError` on not-found; `ResourceNotFoundError` still derives from `Exception`, not `ValueError`.

## Bug fix or enhancement?

**Enhancement** (`### Added`), which matches where the entries already sit. The change is additive and every new type is a `ValueError` subclass, so no existing handler or message match breaks. Apps that do `except ValueError -> 400` still get 400 unless they add a `ConflictError -> 409` handler above it. The issue's stated benefit requires that handler, and the CHANGELOG entry already shows it.

The `_is_lost_race` string match is in the consumer (#10's repository), not in this library, so nothing here needs to change for it. The CHANGELOG already tells users to delete it.

## Deviations from the issue text (decide whether to accept or change)

1. **`save` signature.** The issue proposed `save(obj_in, expected_version)`. The implementation takes `save(id, obj_in, expected_version, ...)`. It needs the id for `expected_version=0`, where there is no object yet. The issue's concern (don't pass a bare id and rely on a pre-read) is addressed: the guard is an int compared against a fresh read, plus a version-conditioned write. Acceptable, but it is a deviation.
2. **Create collision raises `ResourceExistsError`, not `VersionConflictError`.** The issue said "`VersionConflictError` either way". `ResourceExistsError` is a sibling under `ConflictError`, so `except VersionConflictError` will not catch a create collision, while `except ConflictError` does. The `save` docstring and CHANGELOG both say `ConflictError` subclass, so the docs are accurate. The choice is deliberate, but a caller who wrote the issue's `except VersionConflictError` would miss it.
3. **`save` pre-image read is eventually consistent.** `dynamodb_memory.py:371-376` (`get_existing`) defaults to `consistent_read=False`, and `save` calls it without the flag. The internal latest-version read in `update_existing` (around lines 580–593) also uses the default. Effect: a stale read can produce a spurious `VersionConflictError` (a false 409). It cannot produce a silent overwrite, because the write is conditioned on the version. Suggested fix: pass `consistent_read=True` to the `save` pre-image read. Whether to also change the `update_existing` read is a separate decision, since it affects every update.

## Tests that cover this

- `tests/test_typed_conflicts_and_save.py` — new; covers the typed errors on both backends, `save` create/update/stale/missing cases, schema validation by mode, `clear_fields`, cache non-trust, the wrong-class `TypeError`, and a "replaces the message-matching workaround" test.
- `tests/test_local_storage.py:536-555` — `update_from_wrong_version` and "No item found"; asserts old message text.
- `tests/test_blob_storage_bugfixes.py:101-113` — non-latest version message.
- `tests/test_transactions_v2.py`, `tests/test_resource_config_inheritance.py`, `tests/test_omit_none_attributes.py`, `tests/test_version_sorting_bug.py` — reference `VersionConflictError`/`expected_version` in passing.

### Verification run

- `uv run pytest tests/test_typed_conflicts_and_save.py tests/test_local_storage.py -q` → **29 passed, 18 errors**. All 18 errors are in `test_typed_conflicts_and_save.py` and come from the `docker_services` fixture (`docker compose up --build --wait`). Docker is not running in this environment, so those tests did not execute. `test_local_storage.py` passed.
- I did not run the full suite or the DynamoDB-backed tests. Those 18 tests are the ones that would exercise the DynamoDB `transact_write_safe` / `save` paths, so they remain unverified here.
- Note: `uv run` created `coverage_report/` in the worktree. `git status` is clean, so it is ignored by `.gitignore`.

## Documentation

- `AGENT_KNOWLEDGE.md:441-483` already shows `except ConflictError`.
- `AGENT_KNOWLEDGE.md:1776-1781` shows the hierarchy, but the comment reads `VersionConflictError # Back-compat alias.` Check that against the current code (it is a subclass, not an alias).
- `AGENT_KNOWLEDGE.md` does not appear to document `save(` (grep found no match for `save(`). Consider adding a section if the doc is meant to be complete.

## Suggested CHANGELOG action

Nothing new is needed for the behaviour that exists. The `[Unreleased]` `### Added` entries for #13 already cover the hierarchy and `save`. If deviation 2 (create collision is `ResourceExistsError`) or 3 (consistent read) is changed, update those entries. Both are additive, so no `### Changed` or `### Fixed` entry is required unless the `update_existing` error type change is considered a behaviour change for code that matches on the exact class (it is still a `ValueError`, so it likely is not).

## Open questions for the ticket

- Is #13 meant to close now that it is on main? If yes, the issue body's line references (e.g. `dynamodb_memory.py:190`, `:442`, `transactions.py:809`) are stale and should be dropped.
- Accept deviations 1 and 2 as designed, or change them?
- Should `save`'s pre-image read use `consistent_read=True` (deviation 3)?
- Run the 18 Docker-backed tests in CI (or locally with Docker) before closing.
