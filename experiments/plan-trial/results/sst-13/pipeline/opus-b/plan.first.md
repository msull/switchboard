# Plan — issue #13: typed conflict errors + `save(obj, expected_version)`

## 0. Read this first: the feature has already shipped

The issue was filed against 19.0.0. Both halves landed later in commit `7bfdeb0`
("Typed conflict errors and a save(expected_version) primitive (#13)") and were released in
**21.0.0 (2026-08-24)**. `git tag --contains 7bfdeb0` → `21.0.0`, `22.0.0`. The branch base
(`a82a3f6`) contains it, and CHANGELOG.md:219–309 records it under `## [21.0.0]`.

I re-checked the investigation's claims against the tree. They hold:

| Issue asks for | Where it is now |
|---|---|
| Typed exceptions in one module, as `ValueError` subclasses | `src/simplesingletable/exceptions.py:43-160`: `ConflictError(ValueError)` → `ResourceExistsError`; `TransactionConditionFailedError(TransactionError, ConflictError)` → `VersionConflictError` |
| `transact_write_safe` stops flattening `ConditionalCheckFailed` to `ValueError` | `dynamodb_memory.py:296-329`: raises `VersionConflictError` (with `cancellation_reasons`) when any reason is `ConditionalCheckFailed`. Other cancellations stay bare `ValueError`, same message format |
| `update_existing` "non-latest version" becomes `VersionConflictError` | `dynamodb_memory.py:575-582`, message text unchanged, carries `expected_version` / `actual_version` |
| Conditional create of an existing id is typed | `_create_new_versioned` raises `ResourceExistsError` (`dynamodb_memory.py:1124-1131`) |
| `LocalStorageMemory` behaves the same way | `local_storage_memory.py:434, 502, 557, 568` |
| Exported | package root `__init__.py:9-22`, and still re-exported from `transactions.py` |
| `repo.save(..., expected_version)`: 0 creates conditionally, >0 updates from that exact version, conflict either way | `ResourceRepository.save` at `extras/repository.py:183-275`. `VersionedResourceRepository` inherits it |
| Tests | `tests/test_typed_conflicts_and_save.py` (hierarchy, back-compat, stale pre-image, taken id, losing the write race, txn conditions as `ConflictError`, LocalStorage parity, and 11 `save` tests) |

The shipped signature is not quite the one the issue proposed. That was deliberate and is
recorded in the 21.0.0 CHANGELOG entry:

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

It takes the id instead of a pre-read object. The TOCTOU concern is handled in the write
itself: `_update_existing_versioned` conditions on `attribute_exists(pk) AND #version =
:version`, so a pre-read is never the only guard. `save` reads with `self.ddb.get_existing`,
skipping the repository cache, and only uses that read to fail early and get the pre-image.

**Do not re-implement or change any of this.** No public API changes in this plan.

What's left is (a) documentation drift, since AGENT_KNOWLEDGE.md was never updated for #13,
and (b) one `save` branch no test covers. Once this work merges, close #13 as fixed in
21.0.0 and link `7bfdeb0` and the CHANGELOG entry.

Out of scope (separate ticket if wanted): conditional create for **non-versioned**
resources. `_put_nonversioned_resource` (`dynamodb_memory.py:~1056`) does an unconditioned
`put_item`, and `save` raises `TypeError` for non-versioned models. Both are documented as
intentional in 21.0.0.

---

## 1. Files to change

### 1.1 `AGENT_KNOWLEDGE.md`: four edits

**(a) §1 "Core CRUD Methods" (lines ~75-113).** Add the raised types as comments in the
existing code block. Change nothing else in it.

- Above `resource = memory.create_new(`, add:
  `# CREATE — returns the created resource. Versioned + taken override_id → ResourceExistsError`
  (replace the existing `# CREATE — returns the created resource` line).
- Replace `# UPDATE — returns updated resource` with:
  ```
  # UPDATE — returns updated resource.
  # Versioned: if existing_resource is not the latest version (someone else wrote first),
  # raises VersionConflictError (a ConflictError / ValueError). The write itself is
  # conditioned on the version, so a race between read and write is caught too.
  ```

**(b) §5 "ResourceRepository" methods block (lines ~791-803).** Add one line after
`repo.update(...)`:

```python
repo.save(id, obj_in, expected_version, clear_fields=None, changed_by=None, audit_metadata=None) -> T  # versioned only
```

Then, right after the methods code block and before `### VersionedResourceRepository`, add a
new subsection:

````markdown
### Optimistic writes: `save(id, obj_in, expected_version)`

Create-or-update, but only from the state you read. `expected_version=0` means *must not
exist yet* (validated against the create schema); any positive value means *must currently be
at exactly that version* (validated against the update schema). A lost race raises a
`ConflictError` subclass either way, so you write one `except` and no create/update branch:

```python
from simplesingletable import ConflictError

try:
    doc = repo.save(doc_id, changes, expected_version=known_version)  # 0 for a new doc
except ConflictError:
    raise HTTPException(status_code=409)  # re-read and retry, or report the conflict
```

- `expected_version=0` and the id exists → `ResourceExistsError`.
- `expected_version>0` and the resource is missing or at another version →
  `VersionConflictError` (`.expected_version`, `.actual_version`; `actual_version` is
  `None` when missing). A missing resource is never silently created.
- The write is conditioned on the stored version, so a writer that slips in after `save`'s
  read is still caught. `save` reads around the repository cache and evicts the entry on
  conflict.
- Versioned models only: a non-versioned model raises `TypeError`. A negative
  `expected_version` raises `ValueError`.
- Do not match exception message text (`"ConditionalCheckFailed"`, `"non-latest version"`).
  Catch `ConflictError`. All of these types still subclass `ValueError`, so old handlers
  keep working.
````

**(c) §14 "Exception hierarchy" (lines ~1770-1790).** Replace the import block and tree
comment with the current hierarchy. Keep the paragraphs that follow (`cancellation_reasons`,
`operation_indexes`, the try/except example) as they are, except for the one-word fix noted
below.

````markdown
```python
from simplesingletable import (
    ConflictError,
    ResourceExistsError,
    TransactionError,
    TransactionConditionFailedError,
    VersionConflictError,
)  # also importable from simplesingletable.exceptions / simplesingletable.transactions

# ValueError
# └── ConflictError                         # Stored state wasn't what the caller expected. Catch this for 409.
#     ├── ResourceExistsError               # Conditional create hit an existing item (create_new / save(..., 0)).
#     └── TransactionConditionFailedError   # Also a TransactionError. A condition didn't hold.
#         └── VersionConflictError          # Lost optimistic-concurrency race, on either write path.
#
# TransactionError (Exception)              # Any transaction failure.
# └── TransactionConditionFailedError
```

`ConflictError` carries `resource_type` and `resource_id` when known. `VersionConflictError`
adds `expected_version` / `actual_version` when known. Non-condition transaction cancellations
from the non-transactional path (`transact_write_safe`) are still plain `ValueError`.
````

In the next sentence, "All three classes carry:" should become "The `Transaction*` classes
(and `VersionConflictError`) carry:". `ResourceExistsError` is not a `TransactionError` and
has no `operation_indexes`. Check `exceptions.py:84-160` for which attributes each class
actually sets before finalizing this wording.

**(d) "Quick Reference: Key Imports" (lines ~2058-2063).** Replace the `# Transactions`
import block with:

```python
# Errors (all ValueError subclasses except TransactionError's non-conflict cases)
from simplesingletable import (
    ConflictError,
    ResourceExistsError,
    TransactionError,
    TransactionConditionFailedError,
    VersionConflictError,
)
```

### 1.2 `README.md`: optional, one bullet

In the feature list at ~line 128-129, after the Versioned Repository bullet, add:

```markdown
- **Optimistic writes**: `repo.save(id, obj, expected_version)` — create (`0`) or update from an exact version; a lost race raises `ConflictError`
```

No other README changes. Don't touch the version line; bumpver owns it.

### 1.3 No source changes

`src/` stays unchanged. If an implementer finds a docstring that contradicts the docs above,
the code wins. Fix the doc, not the code, and mention it in the PR.

---

## 2. Test to add

The suite is already thorough. One branch in `save` has no test: the
`except VersionConflictError` at `extras/repository.py:270-274`, which evicts the cache when
the conflict comes from the **write**, after `save`'s own pre-read passed. The existing
`test_save_does_not_trust_a_cached_pre_image` only reaches the pre-read branch (lines
259-268).

Add to `tests/test_typed_conflicts_and_save.py`, in the `save()` section after
`test_save_does_not_trust_a_cached_pre_image`. It uses the existing `Doc` / `CreateDoc` /
`UpdateDoc` models and the `dynamodb_memory` fixture (DynamoDB Local):

```python
def test_save_losing_the_race_after_its_read_conflicts_and_evicts_the_cache(dynamodb_memory, monkeypatch):
    """A writer that lands between save()'s pre-read and its write is still caught."""
    cached_repo = VersionedResourceRepository(
        ddb=dynamodb_memory,
        model_class=Doc,
        create_schema_class=CreateDoc,
        update_schema_class=UpdateDoc,
        cache_ttl_seconds=300,
    )
    cached_repo.save("doc-1", {"title": "t", "content": "v1"}, expected_version=0)
    cached_repo.get("doc-1")  # warm the cache at version 1

    real_get_existing = dynamodb_memory.get_existing
    raced = False

    def get_existing_then_race(*args, **kwargs):
        nonlocal raced
        pre_image = real_get_existing(*args, **kwargs)
        if not raced:
            raced = True
            # Another writer moves the resource forward after save() has its pre-image.
            dynamodb_memory.update_existing(pre_image, UpdateDoc(content="other writer"))
        return pre_image

    monkeypatch.setattr(dynamodb_memory, "get_existing", get_existing_then_race)

    with pytest.raises(VersionConflictError):
        cached_repo.save("doc-1", {"content": "loser"}, expected_version=1)

    monkeypatch.undo()
    stored = dynamodb_memory.get_existing("doc-1", Doc)
    assert (stored.version, stored.content) == (2, "other writer"), "the losing write must not land"
    assert cached_repo.get("doc-1").version == 2, "the conflict must evict the stale cache entry"
```

Implementation notes:
- `save` calls `self.ddb.get_existing(id, self.model_class)`. Patching the instance attribute
  on `dynamodb_memory` intercepts it because `cached_repo.ddb is dynamodb_memory`. Confirm
  `ResourceRepository.__init__` stores `ddb` as-is rather than wrapping it.
- `update_existing` re-reads the latest version before writing (`dynamodb_memory.py:~570`).
  If that internal read also goes through `self.get_existing`, the patched function runs
  again, but the `raced` flag keeps it from racing twice. The conflict then comes from
  `update_existing`'s version check or from the write condition. Both raise
  `VersionConflictError` inside `_update`, which is the branch under test. Don't assert on
  `cancellation_reasons`, since which of the two fires depends on that internal read.
- Run: `pytest tests/test_typed_conflicts_and_save.py -k save` and then the full
  `pytest`. Both need Docker for DynamoDB Local.
- If a doc test exists that greps AGENT_KNOWLEDGE.md, run it too. None exists today:
  `grep -rn AGENT_KNOWLEDGE tests/` is empty.

---

## 3. CHANGELOG entry

Add under `## [Unreleased]` in `CHANGELOG.md`. The section currently has `### Added`,
`### Changed`, `### Deprecated` and `### Fixed`, with no `### Documentation`. Add a new
`### Documentation` subsection after `### Fixed` and before `## [22.0.0]`. Do **not**
add a version header. `inv fullrelease` stamps it.

```markdown
### Documentation

* AGENT_KNOWLEDGE.md now covers `ResourceRepository.save(id, obj_in, expected_version)` and the
  `ConflictError` / `ResourceExistsError` / `VersionConflictError` hierarchy shipped in 21.0.0
  (#13). The §14 exception tree was stale: it listed only the transaction types and still
  called `VersionConflictError` a back-compat alias, when it is the type raised by
  non-transactional `update_existing` as well.
```

(The new test is test-only and needs no entry.)

---

## 4. Checklist

1. Edit AGENT_KNOWLEDGE.md §1, §5 (methods line + new subsection), §14 and Quick Reference
   as in 1.1.
2. Optionally add the README bullet (1.2).
3. Add the test in §2. Run `pytest tests/test_typed_conflicts_and_save.py`, then `pytest`.
4. Add the CHANGELOG `### Documentation` entry under `## [Unreleased]`.
5. `inv lint`.
6. No `pyproject.toml` / dependency change, so `uv.lock` is untouched. No version bump.
7. PR description: say #13 was implemented in 21.0.0 (`7bfdeb0`), that this PR only fixes
   docs and adds one test, and that the issue can be closed. Note that the non-versioned
   conditional-create gap is a separate follow-up.
