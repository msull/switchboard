# Plan: Issue #13 — finish the typed-conflict / `save(expected_version)` work

## 0. Read this first: most of #13 already shipped

The issue was written against 19.0.0. The repo is at **22.0.0**, and commit `7bfdeb0`
("Typed conflict errors and a save(expected_version) primitive (#13)") shipped in **21.0.0**
(see `CHANGELOG.md`, section `## [21.0.0] 2026-08-24`). Everything below is already on `main`:

| Issue ask | Where it is now |
|---|---|
| Typed conflicts that stay `ValueError` subclasses | `src/simplesingletable/exceptions.py`: `ConflictError(ValueError)` → `ResourceExistsError`, `TransactionConditionFailedError` → `VersionConflictError`. Exported from the package root. |
| `transact_write_safe` stops raising a bare `ValueError` on `ConditionalCheckFailed` | `dynamodb_memory.py:296-329` raises `VersionConflictError(cancellation_reasons=...)`. Other cancellations stay a bare `ValueError`. |
| `update_existing` raises `VersionConflictError` instead of `"Cannot update from non-latest version"` | `dynamodb_memory.py:566-573`. Same message, with `expected_version` / `actual_version` set. |
| Versioned create on a taken id | `_create_new_versioned` raises `ResourceExistsError` (`dynamodb_memory.py:1124-1132`). |
| `LocalStorageMemory` parity | `local_storage_memory.py:434, 502, 557, 568`. |
| `repo.save(..., expected_version)` | `extras/repository.py` `ResourceRepository.save(id, obj_in, expected_version, clear_fields=None, changed_by=None, audit_metadata=None)`. `0` is a conditional create; `>0` reads, compares, then updates; it bypasses the cache. |
| Tests | `tests/test_typed_conflicts_and_save.py` (379 lines). |

**Do not re-implement any of this, and do not add a second CHANGELOG entry for it.**

This plan covers only the places where the shipped code still breaks the issue's contract:
*a lost optimistic write is always a `ConflictError`, it carries structured context, and
`save` has no TOCTOU hole.* I found four places where that does not hold yet.

---

## 1. Gaps to fix

### Gap A: a delete that races an update becomes a bare `ValueError`

`DynamoDbMemory.update_existing` (versioned branch, `dynamodb_memory.py:562-565`) gets the
latest version with `self.read_existing(...)`. If the resource was deleted after the caller
read it, `read_existing` raises `ValueError("No item found with the provided key.")`.

`save(id, ..., expected_version=N)` is exposed to this. It does its own pre-read
(`repository.py`, `existing = self.ddb.get_existing(...)`), then calls `_update`, which calls
`update_existing`, which reads again. A delete between those two reads surfaces as a plain
`ValueError`. That is the "lost race reported as a 400" problem the issue exists to remove,
and it contradicts `save`'s docstring ("`VersionConflictError`: the resource is absent").

`LocalStorageMemory` has the same problem in two places: `update_existing` (`read_existing`
at `local_storage_memory.py:497`), and `_update_existing_versioned`, which raises
`ValueError("Resource does not exist")` (`local_storage_memory.py:565`) when v0 is missing.

### Gap B: the pre-reads are eventually consistent

`save` calls `self.ddb.get_existing(id, self.model_class)` with the default
`consistent_read=False`. `update_existing` calls `read_existing(...)` with the default too.
On real DynamoDB, a caller that has just written version N and immediately calls
`save(..., expected_version=N)` can read version N-1 and get a **spurious**
`VersionConflictError(actual_version=N-1)`. This is not a safety hole, because the
transaction condition on v0 still guards the write. It is a false 409, and a retry loop
written to the documented pattern ("re-read and retry") can spin on it. Both reads only
exist to make a version decision, so both should be strongly consistent. That costs one
extra RCU per read.

DynamoDB Local is always consistent, so the test for this asserts the read *parameters*
rather than trying to reproduce stale reads (see §4).

### Gap C: a conflict on the write itself has no context

When the race is lost **at the transaction** rather than at the pre-read check, i.e. a
writer slips in between `update_existing`'s read and `transact_write_items`, the error comes
from `transact_write_safe`. That error has `resource_type`, `resource_id` and
`expected_version` all `None`. This is the one race the pre-read check cannot catch, and it
is exactly the case where the caller most needs `exc.resource_id` to decide what to re-read.
`_update_existing_versioned` (`dynamodb_memory.py:1164`) knows all three values and should
attach them.

### Gap D: the docs still describe the pre-#13 hierarchy and do not mention `save`

* `AGENT_KNOWLEDGE.md` around line 1770 (section "Exception hierarchy") shows
  `TransactionError → TransactionConditionFailedError → VersionConflictError  # Back-compat alias`.
  It is missing `ConflictError`, `ResourceExistsError`, the `ValueError` base, and the
  non-transactional path.
* `save()` appears in neither `AGENT_KNOWLEDGE.md` §5 "Repository Pattern (v2)" (line ~764) nor
  `README.md` ("Extras Module", line ~124). The issue calls `save` the single most-wanted
  feature, yet the only place it is documented is the CHANGELOG.

### Considered and rejected (do not implement)

* **Changing the signature to `save(obj, expected_version)` with a pre-read object.** The
  issue argued for passing the object because a hand-rolled *pre-read* is TOCTOU. The shipped
  `save(id, obj_in, expected_version)` already avoids that: the write is conditioned on
  `expected_version` in DynamoDB (via `update_existing` → `_update_existing_versioned`'s
  `#version = :version` condition on v0), not on anything the caller read. Accepting an
  object as well would add a second way to state the expected version and a "they disagree"
  error case, with no safety gain. Renaming the `id` parameter would also break keyword
  callers. Keep the shipped signature.
* **`save` on non-versioned resources.** This stays a `TypeError`, as the 21.0.0 changelog
  already documents. A non-versioned resource has no version to condition on, and
  `_put_nonversioned_resource` is an unconditional `put_item`. Supporting it is a separate
  feature (conditional non-versioned create).
* **`ResourceNotFoundError` from `transactions.py` `_build_versioned_update_items`.** It is
  raised at queue time, before any write, so it is "not found", not a lost race. Leave it.

---

## 2. Code changes

No public signatures change. No new public names. Messages stay the same, so existing
`match=` users keep working.

### 2.1 `src/simplesingletable/dynamodb_memory.py`

**`DynamoDbMemory.update_existing`, versioned branch** (currently lines 561-575). Replace the
`read_existing` call with a consistent `get_existing`, and turn "absent" into a conflict:

```python
elif issubclass(data_class, DynamoDbVersionedResource):
    latest_resource = self.get_existing(
        existing_id=existing_resource.resource_id,
        data_class=data_class,
        consistent_read=True,
    )
    if latest_resource is None:
        # Deleted since the caller read it: the stored state is not what they
        # expected, which is a lost race, not bad input.
        raise VersionConflictError(
            "No item found with the provided key.",
            resource_type=data_class.__name__,
            resource_id=existing_resource.resource_id,
            expected_version=existing_resource.version,
            actual_version=None,
        )
    if existing_resource.version != latest_resource.version:
        raise VersionConflictError(  # unchanged
            "Cannot update from non-latest version", ...
        )
```

The message is kept identical to what `read_existing` raised, so any
`match="No item found"` handler still matches, and the type is still a `ValueError`.

**`DynamoDbMemory._update_existing_versioned`** (line 1164). Wrap the `transact_write_safe(...)`
call:

```python
try:
    transact_write_safe(self.dynamodb_client, [...])  # items unchanged
except VersionConflictError as e:
    raise VersionConflictError(
        str(e),
        cancellation_reasons=e.cancellation_reasons,
        resource_type=resource.__class__.__name__,
        resource_id=resource.resource_id,
        expected_version=previous_version,
        actual_version=None,  # a cancellation does not report the stored value
    ) from e
```

`str(e)` keeps the `"Transaction failed: Item 1: ConditionalCheckFailed - ..."` text, so
message matchers keep working. `VersionConflictError` is already imported in this module.
`_create_new_versioned` already does the same thing for `ResourceExistsError`, so there is
nothing to change there.

`transact_write_safe` itself does not change.

### 2.2 `src/simplesingletable/local_storage_memory.py`

Mirror §2.1 so the backends still raise identical types:

* `update_existing`, versioned branch (line ~497): the same `get_existing(...,
  consistent_read=True)` plus `None` → `VersionConflictError("No item found with the provided
  key.", ...)` change. (`consistent_read` is accepted and ignored here; pass it anyway so the
  code matches the DynamoDB backend.)
* `_update_existing_versioned` (line ~565): replace `raise ValueError("Resource does not
  exist")` with

  ```python
  raise VersionConflictError(
      "Resource does not exist",
      resource_type=resource.__class__.__name__,
      resource_id=resource.resource_id,
      expected_version=previous_version,
      actual_version=None,
  )
  ```

`VersionConflictError` is already imported (line 22).

### 2.3 `src/simplesingletable/extras/repository.py`

In `ResourceRepository.save`, make the pre-read consistent:

```python
existing = self.ddb.get_existing(id, self.model_class, consistent_read=True)
```

Update the comment above it to give both reasons: it skips the cache *and* reads
consistently, so a just-written version is not reported as a conflict. Nothing else changes.
The existing `except VersionConflictError` around `_update` already invalidates the cache,
and it now also catches Gap A's delete race and Gap C's enriched error.

Docstring tweak in the `Raises:` block:
`VersionConflictError: The resource is absent (including deleted mid-save), or at a different version.`

### 2.4 Docs

* **`AGENT_KNOWLEDGE.md` "Exception hierarchy"** (~line 1770). Replace the tree with the
  real one, importing from `simplesingletable`:

  ```
  ValueError
  └── ConflictError                        # stored state was not what the caller expected (→ 409)
      ├── ResourceExistsError              # conditional create hit an existing item
      └── TransactionConditionFailedError  # also a TransactionError
          └── VersionConflictError         # lost an optimistic-concurrency race (either write path)
  TransactionError                          # any transaction failure
  ```

  Drop the "Back-compat alias" comment. Add one sentence saying `except ConflictError` covers
  the transactional and non-transactional paths alike, and that conflicts carry
  `resource_type`, `resource_id`, `expected_version`, `actual_version` (the version fields
  are `None` where the raising site cannot know them).
* **`AGENT_KNOWLEDGE.md` §5**: add a `### save(): write only from a known version`
  subsection after "VersionedResourceRepository" (~line 836). Include the snippet from the
  21.0.0 CHANGELOG entry. Explain `expected_version=0` (create; `ResourceExistsError`) versus
  `>0` (update; `VersionConflictError`, including when absent or deleted mid-save). Note that
  it is versioned-only (`TypeError` otherwise), bypasses the cache, and that the write itself
  is conditioned, so it is TOCTOU-safe where `get()` then `update()` is not.
* **`README.md` "Extras Module"** (~line 128): extend the `ResourceRepository` bullet with
  "`save(id, obj, expected_version)` for optimistic create-or-update; conflicts raise
  `ConflictError`".

---

## 3. Public API after this change (unchanged, restated for the implementer)

```python
# simplesingletable.extras.repository
class ResourceRepository:
    def save(
        self,
        id: Any,
        obj_in: CreateSchemaType | UpdateSchemaType | dict,
        expected_version: int,
        clear_fields: set[str] | None = None,
        changed_by: str | None = None,
        audit_metadata: dict | None = None,
    ) -> T: ...

# simplesingletable.DynamoDbMemory / LocalStorageMemory
def update_existing(self, existing_resource, update_obj, clear_fields=None,
                    changed_by=None, audit_metadata=None) -> AnyDbResource: ...
```

Behaviour changes, all within existing `ValueError` handling:

| Situation | Before | After |
|---|---|---|
| Versioned `update_existing` on a resource deleted since it was read | `ValueError("No item found with the provided key.")` | `VersionConflictError`, same message, `actual_version=None` |
| `LocalStorageMemory` v0 missing at write time | `ValueError("Resource does not exist")` | `VersionConflictError`, same message |
| Versioned update loses the race at the transaction | `VersionConflictError` with no context | the same, plus `resource_type` / `resource_id` / `expected_version` |
| `save` pre-read and `update_existing` latest read | eventually consistent | `ConsistentRead=True` |

---

## 4. Tests

Add these to the existing **`tests/test_typed_conflicts_and_save.py`**, reusing its `Doc`,
`CreateDoc`, `UpdateDoc`, `repo`, `local_storage` and `dynamodb_memory` fixtures. All of them
run against DynamoDB Local through the `dynamodb_memory` fixture in `tests/conftest.py`,
except the `local_storage` ones.

Place each test under the matching section header ("Non-transactional write path" or "save()").

1. **`test_update_of_a_deleted_resource_is_a_conflict`** (non-transactional section)
   ```python
   doc = dynamodb_memory.create_new(Doc, CreateDoc(title="t", content="v1"))
   dynamodb_memory.delete_all_versions(doc.resource_id, Doc)
   with pytest.raises(VersionConflictError) as exc_info:
       dynamodb_memory.update_existing(doc, UpdateDoc(content="v2"))
   exc = exc_info.value
   assert (exc.resource_type, exc.resource_id) == ("Doc", doc.resource_id)
   assert (exc.expected_version, exc.actual_version) == (1, None)
   ```
2. **`test_update_of_a_deleted_resource_keeps_its_message`**: back-compat. The same setup, then
   `pytest.raises(ValueError, match="No item found with the provided key")`.
3. **Extend `test_losing_the_write_race_itself_raises_version_conflict`**. After the existing
   `cancellation_reasons` assertion, add:
   ```python
   exc = exc_info.value
   assert exc.resource_type == "Doc"
   assert exc.resource_id == doc.resource_id
   assert exc.expected_version == 99
   assert exc.actual_version is None
   assert "ConditionalCheckFailed" in str(exc)  # message unchanged for old matchers
   ```
4. **`test_save_when_deleted_between_read_and_write_conflicts`** (save section). This
   reproduces the real interleaving deterministically. Wrap `get_existing` so that, right after
   `save`'s pre-read returns, another actor deletes the resource:
   ```python
   doc = repo.save("doc-del", {"title": "t", "content": "v1"}, expected_version=0)
   real_get = dynamodb_memory.get_existing
   calls = {"n": 0}

   def get_then_delete(*args, **kwargs):
       result = real_get(*args, **kwargs)
       calls["n"] += 1
       if calls["n"] == 1:  # save()'s pre-read
           dynamodb_memory.delete_all_versions("doc-del", Doc)
       return result

   monkeypatch.setattr(dynamodb_memory, "get_existing", get_then_delete)
   with pytest.raises(VersionConflictError) as exc_info:
       repo.save("doc-del", {"content": "v2"}, expected_version=doc.version)
   assert exc_info.value.actual_version is None
   ```
   `DynamoDbMemory` is a plain (non-frozen, non-slots) dataclass, so `monkeypatch.setattr` on
   the instance works. Before the fix this raises a bare `ValueError` and fails
   `pytest.raises(VersionConflictError)`, so confirm it fails before you apply §2.1.
5. **`test_save_when_overtaken_between_read_and_write_conflicts`**. Same pattern, but the hook
   runs `dynamodb_memory.update_existing(result, UpdateDoc(content="other"))` on the first call
   instead of deleting. Unpatch inside the hook, or guard with the counter, so the nested
   `update_existing` does not recurse. Assert `VersionConflictError` with
   `expected_version == 1`, then assert that `repo.get("doc-x")` / a direct `get_existing`
   shows version 2 with `content == "other"`, i.e. the competing write was not clobbered.
6. **`test_save_and_update_read_the_version_consistently`**. Spy on
   `dynamodb_memory.dynamodb_table.get_item` and record the `ConsistentRead` kwarg of every
   call:
   ```python
   table = dynamodb_memory.dynamodb_table
   seen = []
   real = table.get_item
   monkeypatch.setattr(table, "get_item", lambda **kw: (seen.append(kw.get("ConsistentRead")), real(**kw))[1])
   repo.save("doc-c", {"content": "v2"}, expected_version=1)  # after creating doc-c at v1
   assert seen and all(seen), seen
   ```
   (`dynamodb_table` is cached on the instance in `_dynamodb_table`, so the patched object is
   the one used.) This is the only practical way to cover Gap B against DynamoDB Local.
7. **`test_local_storage_deleted_resource_is_a_conflict`**: parity, using the `local_storage`
   fixture.
   * Create, `delete_all_versions`, `update_existing` → `VersionConflictError` with
     `actual_version is None`.
   * Call `local_storage._update_existing_versioned(next_version, previous_version=1)` after
     deleting, where `next_version = doc.update_existing(UpdateDoc(content="v2"))`, and expect
     `VersionConflictError` with `match="Resource does not exist"`.

Also run the whole suite (`pytest`), and specifically `tests/test_local_storage.py`,
`tests/test_versioned_repository.py`, `tests/test_memory_versioning_methods.py` and
`tests/test_repository.py`. None of them should need changes: every type change here is
`ValueError` → a `ValueError` subclass with the same message. If one does fail, it was
asserting `type(exc) is ValueError`. Fix the assertion, and mention it in the changelog.

Run `inv lint` before committing.

---

## 5. CHANGELOG

Add the following to the **existing** `## [Unreleased]` section of `CHANGELOG.md`. `### Fixed`
already exists there (it holds the #18 entry), so append this bullet to that subsection.
Do **not** add a version header. Do **not** touch `pyproject.toml`, `__init__.py`'s
`package_version`, or the README version line.

```markdown
* **Every lost race on the versioned update path is now a `ConflictError`** (#13 follow-up).
  21.0.0 typed the conflicts, but three interleavings still escaped as a bare `ValueError` or
  without context:

  * Updating a versioned resource that was **deleted** after it was read (including inside
    `ResourceRepository.save`) raised `ValueError("No item found with the provided key.")`. It
    now raises `VersionConflictError` with the same message and `actual_version=None`.
    `LocalStorageMemory`'s `ValueError("Resource does not exist")` for the same race is now a
    `VersionConflictError` too.
  * Losing the race **at the write** (a writer slipping in after the pre-read check) raised a
    `VersionConflictError` with `resource_type`, `resource_id` and `expected_version` all
    `None`. They are now populated. The message is unchanged.
  * `save()`'s pre-read and `update_existing`'s latest-version read were eventually
    consistent, so saving straight after a write could report a spurious conflict against the
    previous version. Both now use `ConsistentRead=True`, which costs one extra read
    capacity unit per versioned update.

  All three are still `ValueError` subclasses with unchanged messages, so existing
  `except ValueError` and message-matching handlers keep working. Code that relied on a
  deleted resource producing something *other* than a `ConflictError` should catch
  `VersionConflictError` and check `actual_version is None`.
```

In the same `## [Unreleased]` section, add this under `### Changed` (which also already
exists):

```markdown
* `AGENT_KNOWLEDGE.md` and `README.md` now document `ResourceRepository.save()` and the full
  `ConflictError` hierarchy. The previous tree omitted the non-transactional path.
```

---

## 6. Order of work

1. Write tests 1-7 and confirm that 1, 4, 6 and 7 fail and that 3 fails on the new
   assertions. 2 and 5 may already pass. Write 5 anyway: it guards the condition on v0.
2. Apply §2.1, then §2.2, then §2.3. Re-run `tests/test_typed_conflicts_and_save.py`.
3. Run the full `pytest` (needs Docker), then `inv lint`.
4. Make the docs edits from §2.4 and add the CHANGELOG entries from §5.
5. Commit on a branch, referencing #13. No version bump and no release.
