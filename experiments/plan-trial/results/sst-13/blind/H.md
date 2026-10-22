# Plan: Issue #13 — typed conflicts on the non-transactional path, and `save(obj, expected_version)`

## 0. Read this first: most of #13 has already shipped

The issue was written against 19.0.0. The tree is now at 22.0.0 (`[tool.bumpver]`), and commit
`7bfdeb0` ("Typed conflict errors and a save(expected_version) primitive (#13)") shipped in
**21.0.0**. See `CHANGELOG.md` → `## [21.0.0]`. Already done, and **not** to be redone:

| Issue ask | Status on `HEAD` |
|---|---|
| Typed conflicts live in `exceptions.py`, as `ValueError` subclasses | Done. `ConflictError` → `ResourceExistsError`, `TransactionConditionFailedError` → `VersionConflictError` (`exceptions.py:43-159`), all exported from the package root (`__init__.py:9-22`). |
| `transact_write_safe` stops raising a bare `ValueError` on `ConditionalCheckFailed` | Done (`dynamodb_memory.py:296-329`). It raises `VersionConflictError`, and other cancellations stay `ValueError`. |
| `update_existing` "non-latest version" check raises `VersionConflictError` | Done (`dynamodb_memory.py:575-582`, `local_storage_memory.py:~501`), with `expected_version`/`actual_version`. |
| Versioned create on a taken id | Done. It raises `ResourceExistsError` (`dynamodb_memory.py:1124-1131`). |
| `repo.save(..., expected_version)` | Done as `ResourceRepository.save(id, obj_in, expected_version, ...)` (`extras/repository.py:183-265`). |
| Tests | `tests/test_typed_conflicts_and_save.py` (379 lines). |

An audit of that implementation against the issue's stated goals ("callers delete their
message-matching workaround", "removes the branch, the string-matching and the TOCTOU footgun")
found six remaining gaps. **This plan closes those gaps and nothing else.** It contains no version
bump and no release step.

### The gaps

- **G1. A conflict from the write race itself carries no context (DynamoDB).** When another writer
  lands between `update_existing`'s version check and its `TransactWriteItems`, the v0 condition
  fails. `transact_write_safe` then raises `VersionConflictError(message, cancellation_reasons=...)`
  with `resource_type`, `resource_id` and `expected_version` all `None`
  (`_update_existing_versioned`, `dynamodb_memory.py:1178-1198`, does not wrap it, unlike
  `_create_new_versioned` at 1124). The 21.0.0 changelog says conflicts carry structured context.
  On this path they do not, and this is the path a real concurrent writer hits.
- **G2. A pre-image whose resource was deleted underneath raises a bare `ValueError`.**
  `update_existing` calls `read_existing` (`dynamodb_memory.py:571`), which raises
  `ValueError("No item found with the provided key.")` when the resource is gone. That is a lost
  race, not bad input, and it is exactly the case the issue says an app maps to 400. The
  `migrate_resource_format` recipe in `AGENT_KNOWLEDGE.md` (`except ConflictError: continue`) would
  crash on it. `save()` is affected too, because a delete between `save`'s own read and
  `update_existing`'s read surfaces as the bare `ValueError`. `LocalStorageMemory` has the same
  problem in two places: `update_existing` → `read_existing`, and
  `_update_existing_versioned` → `raise ValueError("Resource does not exist")`
  (`local_storage_memory.py:564-565`).
- **G3. The pre-image reads are eventually consistent.** `save()` calls
  `self.ddb.get_existing(id, self.model_class)` (`repository.py:246`), and `update_existing` calls
  `read_existing(...)` (`dynamodb_memory.py:571`). Both use the default `consistent_read=False`.
  A stale read gives one of two bad outcomes:
  - It reports an older version than the caller holds, so a caller with the true latest version
    gets a spurious `VersionConflictError` whose `actual_version` is wrong.
  - It reports the caller's version while the item has already moved on. That falls through to G1
    and produces a context-free conflict.
- **G4. `save()` with a falsy id silently creates under a new random id.** With
  `expected_version=0`, `_create` does `if override_id:` (`repository.py:334`), so
  `save(None, ..., 0)` or `save("", ..., 0)` falls through to `override_id_fn`/ULID. A "must not
  exist yet" guard on an id the caller never chose is meaningless.
- **G5. `save()` cannot take the pre-read object.** The issue's proposal is
  `repo.save(obj_in, expected_version)`, passing the **pre-read object**, so that the signature
  itself ties the guard to the state the caller read. What shipped takes only an id and re-reads.
  It is safe, but the caller must carry the version separately, and the pre-image the changes are
  applied to is the library's re-read, not the object the caller holds. `update()` already accepts
  `id_or_obj`. `save()` should accept a model instance too, with `expected_version` defaulting to
  `obj.version`.
- **G6. A stale cache entry makes `repo.update(id, ...)` fail until TTL expiry.**
  `ResourceRepository.update(id)` → `read()` → `_get()` serves the cached pre-image. If another
  writer has moved the item on, `update_existing` correctly raises `VersionConflictError`, but
  `_update` (`repository.py:361-378`) evicts nothing. So **every** retry re-serves the same stale
  entry and conflicts again until `cache_ttl_seconds` elapses, and re-read-and-retry, the advertised
  recovery for a `ConflictError`, cannot succeed. `save()` evicts on conflict; `update()` does not.

---

## 1. Files to change

| File | Change |
|---|---|
| `src/simplesingletable/dynamodb_memory.py` | G1, G2, G3 in `update_existing` and `_update_existing_versioned`. |
| `src/simplesingletable/local_storage_memory.py` | G2 parity in `update_existing` and `_update_existing_versioned`. |
| `src/simplesingletable/extras/repository.py` | G3, G4, G5 in `save`; G6 in `_update`. |
| `tests/test_typed_conflicts_and_save.py` | New tests (section 4). |
| `CHANGELOG.md` | Bullets under the existing `## [Unreleased]` subsections (section 5). |
| `README.md` | New "Optimistic concurrency" subsection (section 6). |
| `AGENT_KNOWLEDGE.md` | Fix the stale exception-hierarchy block (section 6). |

No changes to `exceptions.py`, `transactions.py`, `__init__.py`, `pyproject.toml`, or `uv.lock`. No
new dependencies.

---

## 2. Public API changes

### 2.1 `ResourceRepository.save` (additive, non-breaking)

```python
def save(
    self,
    id: Any,                                   # resource id, OR a pre-read instance of self.model_class
    obj_in: CreateSchemaType | UpdateSchemaType | dict,
    expected_version: int | None = None,       # was: required int
    clear_fields: set[str] | None = None,
    changed_by: str | None = None,
    audit_metadata: dict | None = None,
) -> T:
```

The parameter keeps the name `id`, not `id_or_obj`, so existing keyword callers
(`save(id="x", ...)`) are not broken. Existing positional and keyword calls behave exactly as
before, apart from the G3/G4 fixes.

Behaviour:

1. **Model check (unchanged).** If `self.model_class` is not a `DynamoDbVersionedResource`, raise
   `TypeError` (existing message).
2. **Pre-image form**: `isinstance(id, DynamoDbVersionedResource)`.
   - If it is not an instance of `self.model_class`, raise
     `TypeError(f"save() was given a {type(id).__name__}; this repository stores {self.model_class.__name__}")`.
   - `expected_version is None` → `expected_version = id.version`.
   - `expected_version != id.version` → raise
     `ValueError(f"expected_version={expected_version} contradicts the pre-image, which is at version {id.version}")`.
     This also catches `expected_version=0` with an instance, because stored versions start at 1.
   - Set `pre_image = id` and `resource_id = id.resource_id`.
3. **Id form**: anything else.
   - `expected_version is None` → raise `TypeError("save() requires expected_version when called with an id")`.
     A missing argument is a calling error, so this follows the TRY004 convention from 21.0.0.
   - Falsy id (`None`, `""`) → raise `ValueError("save() requires a resource id; got {id!r}")` (G4).
   - Set `pre_image = None` and `resource_id = id`.
4. `expected_version < 0` → `ValueError` (existing message, unchanged).
5. **`expected_version == 0`**: unchanged. Validate a dict against the create schema, then call
   `self._create(obj_in, override_id=resource_id, ...)`. That raises `ResourceExistsError` if the id
   is taken. This path is only reachable in the id form.
6. **`expected_version > 0`**: validate a dict against the update schema.
   - Id form: `existing = self.ddb.get_existing(resource_id, self.model_class, consistent_read=True)`
     (G3). If `existing is None` or its version is not `expected_version`, evict the cache entry and
     raise `VersionConflictError` (existing code and message, unchanged).
   - Pre-image form: `existing = pre_image`, with **no extra read**. `update_existing` does its own
     consistent check (G3) and the write is conditioned on the version, so the guard holds.
   - `return self._update(existing, obj_in, clear_fields=..., changed_by=..., audit_metadata=...)`.
     Cache eviction on conflict now lives in `_update` (G6), so remove the local
     `try/except VersionConflictError` around this call.

Docstring updates:
- Document the instance form, with an example:
  ```python
  doc = repo.get(doc_id)           # the read the guard is tied to
  ...
  repo.save(doc, changes)          # fails with VersionConflictError if doc is no longer current
  ```
- State that in the instance form, `obj_in` is applied on top of **the given instance**, as with
  `update(obj, ...)`, so local mutations to that instance are written.
- Add to `Raises:`:
  - `TypeError` when `expected_version` is missing in the id form, or when the instance is the
    wrong class.
  - `ValueError` for a falsy id, or an `expected_version` that contradicts the instance.

### 2.2 `DynamoDbMemory.update_existing` / `LocalStorageMemory.update_existing` (behavioural)

The signature is unchanged. The versioned branch changes in three ways:
- It reads the latest item with a **strongly consistent** read.
- When the resource no longer exists, it raises **`VersionConflictError`**. The message stays
  `"No item found with the provided key."`, so `match=` and `except ValueError` handlers keep
  working. `actual_version` is `None`.
- A condition failure on the write carries `resource_type`, `resource_id` and
  `expected_version=<the version that was read>`. `actual_version` is `None` because DynamoDB does
  not report it. The message stays the original `"Transaction failed: Item 1: ConditionalCheckFailed - ..."`
  string, so the legacy `"ConditionalCheckFailed" in message` matcher still matches.

The non-versioned branch is untouched. It remains an unconditional put, as documented.

---

## 3. Implementation steps

### 3.1 `dynamodb_memory.py` — `update_existing` (lines 570-584)

Replace:

```python
latest_resource = self.read_existing(
    existing_id=existing_resource.resource_id,
    data_class=data_class,
)
if existing_resource.version != latest_resource.version:
```

with:

```python
# Strongly consistent: an eventually-consistent read can report an older version
# (a spurious conflict) or miss a newer one (the conflict then surfaces from the
# write condition instead).
latest_resource = self.get_existing(
    existing_resource.resource_id,
    data_class,
    consistent_read=True,
)
if latest_resource is None:
    # Deleted since the caller read it: a lost race, not bad input. Message kept
    # identical to read_existing's so existing match/except ValueError still work.
    raise VersionConflictError(
        "No item found with the provided key.",
        resource_type=data_class.__name__,
        resource_id=existing_resource.resource_id,
        expected_version=existing_resource.version,
        actual_version=None,
    )
if existing_resource.version != latest_resource.version:
```

The rest of the branch is unchanged.

### 3.2 `dynamodb_memory.py` — `_update_existing_versioned` (lines 1178-1198)

Wrap the `transact_write_safe(...)` call in the same way `_create_new_versioned` does:

```python
try:
    transact_write_safe(self.dynamodb_client, [...unchanged...])
except VersionConflictError as e:
    # The only conditions here are "this version slot is free" and "v0 is still at
    # previous_version", so a condition failure means another writer got there first.
    # Re-raise with context; keep the message so text matchers are unaffected.
    raise VersionConflictError(
        str(e),
        cancellation_reasons=e.cancellation_reasons,
        operation_indexes=e.operation_indexes,
        resource_type=resource.__class__.__name__,
        resource_id=resource.resource_id,
        expected_version=previous_version,
        actual_version=None,
    ) from e
```

Do not change `transact_write_safe` itself. It has no resource context.

### 3.3 `local_storage_memory.py`

- `update_existing`, versioned branch (around line 497): make the same change as 3.1, using
  `self.get_existing(existing_resource.resource_id, data_class, consistent_read=True)`.
  `consistent_read` is accepted and ignored locally, but pass it anyway for symmetry. Raise the same
  `VersionConflictError` when the result is `None`.
- `_update_existing_versioned` (line 564-565): replace `raise ValueError("Resource does not exist")`
  with:
  ```python
  raise VersionConflictError(
      "Resource does not exist",
      resource_type=resource.__class__.__name__,
      resource_id=resource.resource_id,
      expected_version=previous_version,
      actual_version=None,
  )
  ```
  `VersionConflictError` is already imported in this module.

### 3.4 `extras/repository.py`

- Change the import to `from simplesingletable.exceptions import ConflictError, VersionConflictError`.
- **`_update` (G6):**
  ```python
  try:
      result = self.ddb.update_existing(existing_obj, obj_in, clear_fields=..., changed_by=..., audit_metadata=...)
  except ConflictError:
      # The pre-image was stale. If it came from the cache, evict it so a re-read
      # actually re-reads; otherwise every retry would conflict until TTL expiry.
      if self._cache:
          self._cache.invalidate(str(existing_obj.resource_id))
      raise
  ```
- **`save`**: implement section 2.1. The structure, top to bottom: model check → split on
  instance or id → negative check → `== 0` create → `> 0` update. Delete the old trailing
  `try/except VersionConflictError` (now handled by `_update`). Keep the cache eviction in the id
  form's pre-check mismatch branch, because that raise happens before `_update`.

`VersionedResourceRepository` inherits `save`/`_update` and needs no changes.

### 3.5 Lint

Run `inv lint` (black, isort, ruff). TRY004 is enforced, so wrong-type checks must raise
`TypeError`, as specified above.

---

## 4. Tests

Append to `tests/test_typed_conflicts_and_save.py`, reusing its `Doc`/`CreateDoc`/`UpdateDoc`/
`Plain` models and its `repo`, `local_storage` and `dynamodb_memory` fixtures. DynamoDB tests use
the `dynamodb_memory` fixture from `tests/conftest.py`, which needs DynamoDB Local via Docker.
Add a new section header comment, `# Gaps closed after 21.0.0`.

Write races are simulated by monkeypatching the **instance** method `_update_existing_versioned`
(`DynamoDbMemory` is a plain dataclass, so `monkeypatch.setattr(dynamodb_memory, ...)` works). The
patch fires a rival write exactly once, then delegates to the original:

```python
def _race_once(monkeypatch, memory, rival):
    """Run ``rival()`` once, between update_existing's version check and its write."""
    original = memory._update_existing_versioned
    fired = False

    def racing(resource, previous_version):
        nonlocal fired
        if not fired:
            fired = True
            rival()
        return original(resource, previous_version)

    monkeypatch.setattr(memory, "_update_existing_versioned", racing)
```

(The rival's own `update_existing` re-enters `racing`, sees `fired=True` and goes straight to the
original, which is correct.)

### DynamoDB — G1

1. `test_write_race_conflict_carries_context` creates `doc`, then calls
   `_race_once(..., rival=lambda: dynamodb_memory.update_existing(dynamodb_memory.read_existing(doc.resource_id, Doc, consistent_read=True), UpdateDoc(content="rival")))`
   followed by `dynamodb_memory.update_existing(doc, UpdateDoc(content="mine"))`. Assert:
   - `pytest.raises(VersionConflictError)`;
   - `exc.resource_type == "Doc"`, `exc.resource_id == doc.resource_id`, `exc.expected_version == 1`,
     `exc.actual_version is None`;
   - `exc.cancellation_reasons` is non-empty;
   - `"ConditionalCheckFailed" in str(exc)` (the legacy matcher still works);
   - `read_existing(..., consistent_read=True).content == "rival"` (the losing write did not land).
2. `test_write_race_with_a_delete_is_a_conflict`: the rival is
   `dynamodb_memory.delete_existing(doc)`. Expect `VersionConflictError` with the same context
   assertions, and `get_existing(doc.resource_id, Doc)` is `None` afterwards.
3. `test_save_write_race_is_a_conflict_with_context`: `repo.save("doc-1", {...}, 0)`, install the
   rival, then `repo.save("doc-1", {"content": "mine"}, expected_version=1)`. Expect
   `VersionConflictError` with `resource_id == "doc-1"` and `expected_version == 1`.

### DynamoDB — G2

4. `test_update_of_a_deleted_resource_is_a_conflict`: create `doc`, call
   `dynamodb_memory.delete_existing(doc)`, then `update_existing(doc, UpdateDoc(content="x"))`.
   Expect `VersionConflictError`, `expected_version == 1`, `actual_version is None`,
   `resource_type == "Doc"`, and also `pytest.raises(ValueError, match="No item found")` (the
   message and type are back-compatible). Also assert `isinstance(exc, ConflictError)`.
5. `test_save_after_delete_between_reads_is_a_conflict`: create via `repo.save(..., 0)`.
   Monkeypatch `repo.ddb.get_existing` so the **first** call returns the real result and then
   deletes the item (`dynamodb_memory.delete_existing(result)`). That reproduces a delete between
   `save`'s read and `update_existing`'s read. Then call `repo.save("doc-1", {...}, 1)`. Expect
   `VersionConflictError`, not a bare `ValueError`
   (`assert type(exc_info.value) is VersionConflictError`).

### DynamoDB — G3

6. `test_version_check_reads_are_strongly_consistent`: wrap `dynamodb_memory.get_existing` with a
   spy (`monkeypatch.setattr(dynamodb_memory, "get_existing", spy)`) that records
   `kwargs.get("consistent_read", False)` and delegates. Then:
   - Run `update_existing(doc, ...)` and assert every recorded call had `consistent_read=True`.
   - Clear the spy. Run `repo.save(doc.resource_id, {...}, expected_version=doc.version)` on a
     repo built on the same `dynamodb_memory`, and assert the same.

   DynamoDB Local is always consistent, so the flag is the observable contract.

### Repository — G4

7. `test_save_rejects_a_missing_id`: parametrize over `[None, ""]`.
   `pytest.raises(ValueError, match="resource id")` on `repo.save(bad, {"title": "t", "content": "c"}, expected_version=0)`.
   Then assert `repo.list() == []`, so nothing was created under a generated id.

### Repository — G5 (pre-image form)

8. `test_save_accepts_the_pre_read_object`: `doc = repo.save("doc-1", {...}, 0)`, then
   `updated = repo.save(doc, {"content": "v2"})` with no `expected_version`. Assert
   `updated.version == 2`, `updated.content == "v2"`, and `updated.title` is unchanged.
9. `test_save_with_a_stale_pre_read_object_conflicts`: `doc` at v1, `repo.save(doc, {"content": "v2"})`,
   then `repo.save(doc, {"content": "v3"})` again with the stale v1 object. Expect
   `VersionConflictError` with `expected_version == 1`, `actual_version == 2`, and
   `repo.read("doc-1").content == "v2"`.
10. `test_save_pre_read_object_with_matching_explicit_version`:
    `repo.save(doc, {...}, expected_version=doc.version)` succeeds.
11. `test_save_pre_read_object_with_contradicting_version`: `repo.save(doc, {...}, expected_version=5)`
    raises `ValueError` matching `"contradicts"`. Also `repo.save(doc, {...}, expected_version=0)`
    raises `ValueError`, and the stored item is unchanged (`version == 1`).
12. `test_save_pre_read_object_of_the_wrong_class`: pass an instance of a second versioned model
    `OtherDoc(DynamoDbVersionedResource)` (defined in the test module) and expect `TypeError`.
13. `test_save_by_id_still_requires_expected_version`: `repo.save("doc-1", {...})` raises
    `TypeError` matching `"expected_version"`.
14. `test_save_pre_read_object_does_not_reread`: spy on `repo.ddb.get_existing` (as in test 6) and
    call `repo.save(doc, {"content": "v2"})`. Assert that exactly one pre-write read happened
    (`update_existing`'s own check) plus the post-write `read_existing`. Equivalently: no call
    before `update_existing` is entered. A simpler form that is good enough: count calls and
    assert `== 2`, against `== 3` for the id form. Put a comment on why that number is expected.

### Repository — G6

15. `test_update_after_a_conflict_rereads_instead_of_serving_the_stale_cache`: build a
    `VersionedResourceRepository` with `cache_ttl_seconds=300`. Create via `cached_repo.create(...)`
    (which caches v1). Advance the item out of band with
    `dynamodb_memory.update_existing(doc, UpdateDoc(content="v2"))`. Then:
    - `cached_repo.update(doc.resource_id, {"content": "v3"})` raises `VersionConflictError`;
    - the immediate retry `cached_repo.update(doc.resource_id, {"content": "v3"})` **succeeds**
      with `version == 3`. Before the fix, this second call raises again.

### LocalStorageMemory parity — G2

16. `test_local_storage_update_of_a_deleted_resource_is_a_conflict` uses the `local_storage`
    fixture. It runs the same flow as test 4 and gets the same assertions (type, message, context).
17. `test_local_storage_write_race_with_a_delete_is_a_conflict`: monkeypatch
    `local_storage.get_existing` to return the real result on the first call and then delete the
    item. That way `update_existing`'s check passes and `_update_existing_versioned` finds no v0.
    Expect `VersionConflictError` with `message == "Resource does not exist"` and
    `resource_id`/`expected_version` populated.

### Existing tests that must still pass unchanged

- `tests/test_typed_conflicts_and_save.py`, all of it. `test_save_does_not_trust_a_cached_pre_image`
  and `test_save_of_a_missing_resource_conflicts_rather_than_creating` cover the moved eviction and
  the unchanged missing-resource message.
- `tests/test_local_storage.py::test_local_storage__read_nonexistent_resource` (`read_existing` is
  not changed).
- `tests/test_resource_config_inheritance.py` (the migration recipe that relies on
  `except ConflictError`).

Run:

```bash
pytest tests/test_typed_conflicts_and_save.py -v
pytest                       # full suite, Docker required
inv lint
```

---

## 5. CHANGELOG entry

Add these bullets to the **existing** subsections under `## [Unreleased]` in `CHANGELOG.md`. Do not
create a second `### Added` / `### Changed` / `### Fixed`, and do not add a version header. Put each
bullet at the end of its subsection.

Under `### Added`:

```markdown
* **`ResourceRepository.save()` accepts the pre-read object** (#13). `repo.save(doc, changes)`
  conditions the write on `doc.version` and applies `changes` on top of `doc`, so the guard is
  tied, in the signature, to the state the caller actually read. This is the shape #13
  proposed. `expected_version` is now optional in this form. If given, it must equal
  `doc.version` (`ValueError` otherwise). The id form, `save(id, changes, expected_version=N)`, is
  unchanged and still requires `expected_version` (`TypeError` if omitted). An instance of a
  different model class raises `TypeError`.
```

Under `### Changed`:

```markdown
* **Versioned `update_existing` (and so `repo.update` / `repo.save`) checks the current version
  with a strongly consistent read** (#13). An eventually-consistent read could report an older
  version, which made a caller holding the true latest version fail with a spurious
  `VersionConflictError`, or could miss a newer one. Each versioned update now costs one strongly
  consistent `GetItem` (2× the read capacity of the previous eventually-consistent one).
  `LocalStorageMemory` is unaffected.
```

Under `### Fixed`:

```markdown
* **Every lost race on the non-transactional path is now a `ConflictError` with context** (#13).
  Three cases still escaped the 21.0.0 hierarchy:
  - When another writer landed between `update_existing`'s version check and its write, the
    `VersionConflictError` had `resource_type`, `resource_id` and `expected_version` all `None`.
    They are now populated. `actual_version` stays `None`, because DynamoDB does not report it.
  - Updating a versioned resource that was deleted after the caller read it raised a bare
    `ValueError("No item found with the provided key.")`, which an app mapping `ValueError -> 400`
    reported as bad input. It now raises `VersionConflictError` with `actual_version=None`.
  - The same applied to `LocalStorageMemory`'s `"Resource does not exist"`.

  Messages are unchanged and both types are `ValueError`s, so existing `except ValueError` and
  message-matching handlers keep working.

* **`repo.update()` no longer keeps failing on a stale cache entry** (#13). With
  `cache_ttl_seconds` set, an update whose cached pre-image had been overtaken raised
  `VersionConflictError` but left the entry cached, so every re-read-and-retry re-served it and
  conflicted again until the TTL expired. A conflict now evicts the entry.

* **`repo.save()` rejects a missing id** (#13). `save(None, ..., expected_version=0)` (or `""`)
  silently created the resource under a newly generated id, which defeats a "must not exist yet"
  guard. It now raises `ValueError`.
```

---

## 6. Docs

- **`README.md`**: after the `### Version Management` block (ends just before
  `## Internal Library Resources`, around line 436), add `### Optimistic concurrency`. In it:
  - A short hierarchy, copied from the 21.0.0 changelog block: `ValueError` → `ConflictError` →
    {`ResourceExistsError`, `TransactionConditionFailedError` → `VersionConflictError`}.
  - The sentence "catch `ConflictError` to map a lost race to HTTP 409".
  - Both `save` forms:
    ```python
    from simplesingletable import ConflictError

    try:
        doc = repo.save(doc_id, changes, expected_version=if_match)   # 0 = must not exist yet
        # or, holding the object you read:
        doc = repo.save(doc, changes)
    except ConflictError:
        ...  # re-read and retry, or return 409
    ```
- **`AGENT_KNOWLEDGE.md`**: the `### Exception hierarchy` block (around line 1770) still shows
  `VersionConflictError` as a "Back-compat alias" under `TransactionError`, with no
  `ConflictError`. Replace it with the full hierarchy (both roots: `ConflictError(ValueError)` and
  `TransactionError`, with `TransactionConditionFailedError` under both). Add one line: "On the
  non-transactional path `update_existing`/`create_new`/`repo.save` raise the same types; catch
  `ConflictError` for both."

---

## 7. Explicitly out of scope

- **Version bump or release.** `inv fullrelease` stamps the changelog and bumps the version.
- **Non-versioned conditional writes.** `save()` on a `DynamoDbResource` still raises `TypeError`,
  and `_put_nonversioned_resource` stays unconditional. The 21.0.0 changelog already says this is
  not covered. It needs a different guard (`updated_at`, like `txn.put(optimistic=True)`) and
  deserves its own issue.
- **`txn.create()` of a taken id** raises `VersionConflictError`, not `ResourceExistsError`. It is
  still a `ConflictError`, so the 409 mapping works. Refining it means changing what the
  transactional path raises, which is outside #13's non-transactional scope.
- **`transact_write_safe`'s non-condition cancellations** (throttling, validation) stay a bare
  `ValueError`, as 21.0.0 decided.
