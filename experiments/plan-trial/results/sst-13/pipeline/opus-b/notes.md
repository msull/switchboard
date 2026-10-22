# Issue #13 — investigation notes

## Headline: already implemented and released (21.0.0)

The issue says "verified on main at 19.0.0". Both halves were landed afterwards in
commit `7bfdeb0` "Typed conflict errors and a save(expected_version) primitive (#13)",
released in **21.0.0 (2026-08-24)**, and present in 22.0.0 and on this branch's base
(`a82a3f6`). `git tag --contains 7bfdeb0` → `21.0.0`, `22.0.0`. The CHANGELOG has full
21.0.0 entries for it (CHANGELOG.md:219–309).

So there is no code change to make for the issue as written. What remains is
documentation drift in AGENT_KNOWLEDGE.md (below), plus one known, documented scope gap.

## Point 1 — typed conflicts on the non-transactional path: DONE

`src/simplesingletable/exceptions.py` now holds the whole hierarchy:

```
ValueError
└── ConflictError                       (resource_type, resource_id)
    ├── ResourceExistsError             conditional create hit an existing item
    └── TransactionConditionFailedError (also TransactionError)
        └── VersionConflictError        (+ expected_version, actual_version)
```

Sites the issue named, as they stand now:

- `transact_write_safe` — `dynamodb_memory.py:296`. A `TransactionCanceledException`
  with any `ConditionalCheckFailed` reason raises `VersionConflictError` (line 326);
  other cancellations still raise bare `ValueError` with the same message format (line 327).
- `update_existing` (versioned) — `dynamodb_memory.py:575-582` raises
  `VersionConflictError("Cannot update from non-latest version", ..., expected_version,
  actual_version)`. Message text unchanged.
- `_create_new_versioned` — `dynamodb_memory.py:1124-1131` converts the conflict to
  `ResourceExistsError` (both conditions are `attribute_not_exists`).
- `_update_existing_versioned` — write is conditioned on
  `attribute_exists(...) and #version = :version` (≈ line 1192), so the window between
  `update_existing`'s read and its write is covered; a loss surfaces as
  `VersionConflictError` via `transact_write_safe`.
- `LocalStorageMemory` raises the same types (`local_storage_memory.py:434, 502, 557, 568`).
- `transactions.py` re-exports the same objects; package root exports them
  (`__init__.py:15-21`).

## Point 2 — save(expected_version): DONE, with a different signature than proposed

`ResourceRepository.save(id, obj_in, expected_version, clear_fields=None, changed_by=None,
audit_metadata=None)` — `src/simplesingletable/extras/repository.py:183`.

- `expected_version == 0` → validates against create schema, `_create(override_id=id)`;
  existing id → `ResourceExistsError`.
- `> 0` → validates against update schema, reads with `self.ddb.get_existing` (bypasses
  repo cache), raises `VersionConflictError` if absent or at a different version, else
  `_update(existing, ...)`, whose write is version-conditioned. Invalidates the cache entry
  on conflict.
- `< 0` → `ValueError`; non-versioned model → `TypeError`.

Differences from the issue's proposal, both deliberate per the CHANGELOG:
- Signature takes `id` explicitly rather than `save(obj_in, expected_version)`. The
  TOCTOU concern the issue raised (pre-read alone is not a guard) is instead handled by the
  condition on the stored version in the write itself.
- Versioned models only. Non-versioned conditional create is explicitly "not covered yet"
  because `_put_nonversioned_resource` (`dynamodb_memory.py:1056`) does a plain unconditioned
  `put_item`. This is the only real functional gap left, and it is outside the issue's
  wording (the issue's semantics are version-based).
- `VersionedResourceRepository` (`extras/versioned_repository.py:71`) subclasses
  `ResourceRepository` and does not override `save`, so it inherits it unchanged.

## Tests

`tests/test_typed_conflicts_and_save.py` (added in 7bfdeb0) covers everything above:
hierarchy/isinstance (86–117), stale pre-image and message compatibility (135–149), taken id
→ `ResourceExistsError` (158), losing the write race itself (168), transaction conditions
caught as `ConflictError` (184), LocalStorage parity (209), and `save` create / refuse-clobber
/ update / stale / missing / schema selection / clear_fields / negative / non-versioned /
cache bypass / workaround replacement (227–329), plus the `TypeError` wrong-class sites
(351–370). Related older coverage: `test_transaction_loud_failures.py`,
`test_transaction_optimistic_put.py`, `test_repository.py`, `test_versioned_repository.py`.
I did not run the suite (requires Docker).

## Remaining gaps (docs only)

AGENT_KNOWLEDGE.md was not updated for #13:
- §5 Repository Pattern (line 764 onward) does not mention `save()` / `expected_version` at all.
- §1 CRUD docs for `update_existing` / `create_new` don't say they raise
  `VersionConflictError` / `ResourceExistsError`.
- §14 "Exception hierarchy" (≈ lines 1772-1790) is stale: imports only from
  `transactions`, omits `ConflictError` / `ResourceExistsError`, and still labels
  `VersionConflictError` "Back-compat alias" while the class docstring now calls it the
  type raised on both write paths.
- README has no mention of `save` / `ConflictError`.

## Classification

- Public API affected (already shipped): `ConflictError`, `ResourceExistsError`,
  `VersionConflictError`, `TransactionConditionFailedError` (now a `ValueError`),
  `transact_write_safe`, `DynamoDbMemory.update_existing` / `create_new` (versioned),
  `LocalStorageMemory` equivalents, `ResourceRepository.save`, plus the 19 wrong-class
  sites now raising `TypeError` (breaking, 21.0.0).
- CHANGELOG category: already recorded in 21.0.0 — enhancement ("Added": hierarchy + `save`)
  with a breaking "Changed" note. Nothing new to add under `## [Unreleased]` unless the
  AGENT_KNOWLEDGE.md doc fix is done, which would be a docs-only change (no CHANGELOG entry
  needed, or a one-line "Documentation" note).

## Recommendation

Close #13 as fixed in 21.0.0, referencing 7bfdeb0 and the CHANGELOG entry. Optionally open a
small follow-up for the AGENT_KNOWLEDGE.md drift, and (separately, if wanted) a ticket for
conditional create on non-versioned resources.
