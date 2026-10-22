# Plan: Issue #13 — typed conflict errors and `save(expected_version)`

## Finding: the issue is already implemented and released

Issue #13 was written against `main` at 19.0.0. The repository is now past 22.0.0, and the
whole of #13 landed in commit `7bfdeb0` ("Typed conflict errors and a save(expected_version)
primitive (#13)"), which shipped in **21.0.0 (2026-08-24)**. Nothing in the issue's two
sections is outstanding in code. Evidence, checked on the current `HEAD` (`a82a3f6`):

| Issue item | Where it lives now |
|---|---|
| Typed conflicts on the non-transactional path | `src/simplesingletable/exceptions.py:43-160` defines `ConflictError(ValueError)`, `ResourceExistsError(ConflictError)`, `TransactionConditionFailedError(TransactionError, ConflictError)`, `VersionConflictError(TransactionConditionFailedError)`, each carrying `resource_type`, `resource_id`, `expected_version`, `actual_version`, `cancellation_reasons`. |
| `transact_write_safe` raising bare `ValueError` | `src/simplesingletable/dynamodb_memory.py:326` raises `VersionConflictError(..., cancellation_reasons=...)` when a cancellation carries `ConditionalCheckFailed`. |
| `update_existing` raising `ValueError("Cannot update from non-latest version")` | `src/simplesingletable/dynamodb_memory.py:576` raises `VersionConflictError` with the same message text, so message-matching callers still work. |
| Conditional create on a taken id | `src/simplesingletable/dynamodb_memory.py:1127` raises `ResourceExistsError`. |
| Backend parity | `src/simplesingletable/local_storage_memory.py:434,502,557,568` raise the identical types. |
| Types have one home | Transaction exceptions moved to `exceptions.py`; `transactions.py:21-42` re-exports the same objects. All exported from `simplesingletable/__init__.py`. |
| `save` primitive | `src/simplesingletable/extras/repository.py:183-258`: `ResourceRepository.save(id, obj_in, expected_version, clear_fields=None, changed_by=None, audit_metadata=None) -> T`. `0` creates conditionally and raises `ResourceExistsError`; `>0` re-reads bypassing the repository cache, raises `VersionConflictError` if absent or at another version, and the write itself is still conditioned on the stored version. |
| Tests | `tests/test_typed_conflicts_and_save.py` (379 lines, 25 tests) covers the hierarchy, both write paths, LocalStorage parity, every `save` branch, cache bypass, and the TRY004 `TypeError` change. |
| CHANGELOG | Entries are recorded under `## [21.0.0]` (`CHANGELOG.md:227-310`). |

One deliberate divergence from the issue text: the issue proposed `repo.save(obj_in, expected_version)`;
the shipped signature is `save(id, obj_in, expected_version)`. The `id` is required because a
create with `expected_version=0` has no other source of identity. Do not change this.

**Do not re-implement any of the above.** A second implementation would produce duplicate
exception classes and a conflicting CHANGELOG entry.

## What remains

The only gap found is documentation. `README.md` has no mention of `ConflictError`,
`ResourceExistsError`, `VersionConflictError`, or `ResourceRepository.save`. Readers who
start from the README still see only `update_existing` and the plain CRUD pattern
(`README.md:265-287`), and nothing tells them that conflicts are typed.

Note on verification: running the test suite in this environment was blocked by the
session's permission settings, so the test run described in step 1 has **not** been done
here. Do it first.

### Step 1 — Confirm the shipped implementation is green

```bash
uv run pytest tests/test_typed_conflicts_and_save.py -q
uv run pytest -q
inv lint
```

Both need Docker running for DynamoDB Local (`tests/docker-compose.yml`). If anything in
`test_typed_conflicts_and_save.py` fails, that is a regression introduced after 21.0.0 and
should be fixed in place, not by adding new code paths. Do not proceed to step 2 until
this passes.

### Step 2 — Document the conflict hierarchy and `save` in `README.md`

Files to change: `README.md` only. No source changes.

**2a.** After the `### CRUD Operations Pattern` block (ends around `README.md:287`), add a new
subsection:

```markdown
### Conditional Writes and Conflict Handling

Every write that loses an optimistic-concurrency race raises a subclass of `ConflictError`,
on both the transactional and non-transactional paths. All of them are also `ValueError`,
so existing handlers keep working.

    ValueError
    └── ConflictError                        # stored state was not what you expected
        ├── ResourceExistsError              # conditional create hit an existing item
        └── TransactionConditionFailedError  # (also a TransactionError)
            └── VersionConflictError         # a write lost the race

Each carries `resource_type`, `resource_id`, `expected_version` and `actual_version`
(`None` when the resource is absent), so there is no need to match message text.

`ResourceRepository.save` collapses "create if absent, otherwise update from the version I
read" into one call with one failure type:

    from simplesingletable import ConflictError

    try:
        doc = repo.save(doc_id, changes, expected_version=known_version)
    except ConflictError:
        ...  # re-read and retry, or return HTTP 409

`expected_version=0` means the resource must not exist yet and validates `changes` against
the create schema. Any higher value means it must currently be at exactly that version and
validates against the update schema. The write is conditioned on the stored version, so a
writer that slips in between the read and the write is still rejected. `save` requires a
`DynamoDbVersionedResource` model and raises `TypeError` otherwise.
```

**2b.** In `### Extras Module` (`README.md:124-132`), extend the `ResourceRepository` bullet:

```markdown
- **Repository Pattern**: `ResourceRepository` for higher-level operations, including
  `save(id, obj_in, expected_version)` for conditional create-or-update
```

**2c.** In `## Common Gotchas and Considerations` (`README.md:511+`), add one bullet:

```markdown
- **Do not match exception messages to detect conflicts.** Catch `ConflictError` (or
  `VersionConflictError` / `ResourceExistsError`) and read `.expected_version` /
  `.actual_version` instead. Message text is kept stable only for back-compat.
```

Indent the fenced examples inside the README with four spaces or convert to triple-backtick
blocks to match the surrounding file style (`README.md` uses triple backticks).

### Step 3 — Tests

No new tests are required. `tests/test_typed_conflicts_and_save.py` already covers every
behaviour the README text asserts. The one thing worth checking while writing step 2 is that
each statement in the README maps to an existing test, so the docs cannot drift:

| README claim | Covering test |
|---|---|
| Every conflict is a `ValueError` | `test_version_conflict_is_both_a_transaction_error_and_a_value_error`, `test_resource_exists_is_a_conflict_but_not_a_transaction_error` |
| Structured attributes, `actual_version is None` when absent | `test_conflict_carries_structured_context_instead_of_message_text`, `test_save_of_a_missing_resource_conflicts_rather_than_creating` |
| One `except ConflictError` covers both paths | `test_transaction_condition_failure_is_now_catchable_as_a_conflict` |
| `save` with `0` creates / refuses to clobber | `test_save_with_expected_version_zero_creates`, `test_save_with_expected_version_zero_refuses_to_clobber` |
| `save` validates with the schema matching the mode | `test_save_validates_against_the_schema_matching_the_mode` |
| `save` requires a versioned model | `test_save_requires_a_versioned_model` |
| Cache is bypassed | `test_save_does_not_trust_a_cached_pre_image` |
| Message text stable for back-compat | `test_stale_update_message_and_value_error_handling_are_unchanged` |

If a claim has no covering test, drop the claim from the README rather than adding a test.

### Step 4 — CHANGELOG

Under the existing `## [Unreleased]` marker (`CHANGELOG.md:8`), inside the existing
`### Changed` heading (add it after the `GsiIndex` bullets; do not create a version header):

```markdown
* README now documents the `ConflictError` hierarchy and `ResourceRepository.save(id, obj_in,
  expected_version)` introduced in 21.0.0 (#13). No behaviour change.
```

Do not touch the 21.0.0 section. Do not bump the version; `inv fullrelease` does that.

## Out of scope

- Changing the `save` signature to match the issue's sketch. See the divergence note above.
- Adding `save` to `ReadOnlyRepository` / `ReadOnlyVersionedRepository`. They are read-only
  by design.
- Re-landing the TRY004 `ValueError` to `TypeError` change. It shipped in 21.0.0 and is
  already recorded as BREAKING there.
