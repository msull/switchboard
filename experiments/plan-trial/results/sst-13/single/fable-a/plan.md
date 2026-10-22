# Plan: Issue #13 — typed conflict errors and `save(obj, expected_version)`

## 0. Status: the issue is already implemented. This plan covers what is left.

Issue #13 was written against 19.0.0. It was implemented in commit `7bfdeb0`
("Typed conflict errors and a save(expected_version) primitive (#13)") and released in
**21.0.0** (2026-08-24). `main` is now at 22.0.0. Do **not** re-implement any of the
following; it is all on `main` today and covered by `tests/test_typed_conflicts_and_save.py`
(379 lines, 26 tests):

| Issue ask | Where it lives now |
|---|---|
| Typed conflict hierarchy, all `ValueError` subclasses | `src/simplesingletable/exceptions.py:43-159` — `ConflictError(ValueError)` → `ResourceExistsError`, `TransactionConditionFailedError(TransactionError, ConflictError)` → `VersionConflictError` |
| `transact_write_safe` raises typed error on `ConditionalCheckFailed` | `src/simplesingletable/dynamodb_memory.py:296-329` raises `VersionConflictError`; other cancellations stay bare `ValueError` |
| `update_existing` "Cannot update from non-latest version" typed | `dynamodb_memory.py:575-582` raises `VersionConflictError` with `expected_version`/`actual_version`; `local_storage_memory.py:501-507` mirrors it |
| Versioned create with a taken id typed | `dynamodb_memory.py:1124-1131` raises `ResourceExistsError`; `local_storage_memory.py:434` mirrors it |
| Transaction exception types have one home | moved to `exceptions.py`; `transactions.py:19-42` re-exports the identical objects |
| Package-root exports | `src/simplesingletable/__init__.py:15-21` |
| `save(..., expected_version)` primitive | `src/simplesingletable/extras/repository.py:183-265` — `ResourceRepository.save(id, obj_in, expected_version, clear_fields=None, changed_by=None, audit_metadata=None)`; `0` creates via `_create` (conditional put → `ResourceExistsError`), `>0` pre-reads bypassing the cache then `_update` (conditioned write → `VersionConflictError`); requires a versioned model (`TypeError` otherwise); inherited by `VersionedResourceRepository` |
| CHANGELOG | `CHANGELOG.md:219-310` (the `## [21.0.0]` section) |

One deliberate deviation from the issue text: the shipped signature is `save(id, obj_in,
expected_version)`, not `save(obj_in, expected_version)`. The id is required because the create
branch (`expected_version=0`) has no stored object to take an id from, and the issue's own
rationale ("pass the pre-read object, not the id") is satisfied by the `expected_version`
argument carrying the pre-image's version. Keep this signature; do not add an overload.

Reading the code against the issue's claim that conflicts are typed "everywhere else" turned up
three places where a lost race still surfaces as a bare `ValueError`, plus a documentation gap.
Those are the whole of this plan. Everything below is additive and non-breaking (every new raise
is a `ValueError` subclass with an unchanged message).

---

## 1. Files to change

| File | Change |
|---|---|
| `src/simplesingletable/dynamodb_memory.py` | §2.1 — versioned `update_existing`: resource deleted between caller's read and this call raises `VersionConflictError`, not `ValueError("No item found with the provided key.")` |
| `src/simplesingletable/local_storage_memory.py` | §2.1 mirror in `update_existing`; §2.2 — `_update_existing_versioned` `ValueError("Resource does not exist")` → `VersionConflictError` |
| `src/simplesingletable/extras/singleton.py` | §2.3 — `SingletonVersionedResource.saved_updated_singleton` raises `VersionConflictError` with context |
| `tests/test_typed_conflicts_and_save.py` | §3 — add tests for the three cases above (append to the existing "Non-transactional write path" and a new "Singleton" section) |
| `README.md` | §4 — short "Conflicts and `save()`" subsection; the README has no mention of `ConflictError`, `save(` or `expected_version` today |
| `CHANGELOG.md` | §5 — entry under `## [Unreleased]` |

No changes to `exceptions.py`, `transactions.py`, `__init__.py`, `repository.py`,
`pyproject.toml`, or the version. No release steps; `inv fullrelease` handles stamping and bump.

---

## 2. Code changes

### 2.1 `update_existing` on a concurrently deleted versioned resource

**Problem.** `DynamoDbMemory.update_existing` (`dynamodb_memory.py:570-574`) and
`LocalStorageMemory.update_existing` (`local_storage_memory.py:496-500`) fetch the latest version
with `self.read_existing(...)`, which raises bare `ValueError("No item found with the provided
key.")` (`dynamodb_memory.py:545`, `local_storage_memory.py:336`) when the resource is gone. A
caller who read a resource, then had another writer delete it, then calls `update_existing` (or
`repo.update` / `repo.save`) therefore gets an untyped `ValueError` for what is a lost race.
`ResourceRepository.save` pre-checks with `get_existing` and raises `VersionConflictError` when
absent (`repository.py:246-257`), but a delete that lands between that pre-check and `_update`
falls through to this bare `ValueError`.

**Change (both backends, identical shape).** Replace the `read_existing` call in the versioned
branch with `get_existing` and raise on `None`:

```python
elif issubclass(data_class, DynamoDbVersionedResource):
    latest_resource = self.get_existing(
        existing_resource.resource_id,
        data_class=data_class,
    )
    if latest_resource is None:
        raise VersionConflictError(
            f"{data_class.__name__} {existing_resource.resource_id} no longer exists",
            resource_type=data_class.__name__,
            resource_id=existing_resource.resource_id,
            expected_version=existing_resource.version,
            actual_version=None,
        )
    if existing_resource.version != latest_resource.version:
        raise VersionConflictError(
            "Cannot update from non-latest version",   # unchanged
            ...
        )
```

`get_existing` is already imported/defined in both classes; in `DynamoDbMemory` pass the same
default `consistent_read` that `read_existing` was using (it did not pass one, so leave it at the
default). `actual_version=None` matches the convention `save()` already uses for "it does not
exist" (`repository.py:256`) and the `VersionConflictError` docstring.

Do **not** change `read_existing` itself; its `ValueError` is a lookup failure, not a conflict,
and `tests/test_simplesingletable.py` / others assert on it.

### 2.2 `LocalStorageMemory._update_existing_versioned` missing-v0 branch

**Problem.** `local_storage_memory.py:564-565`:

```python
if v0_key not in data:
    raise ValueError("Resource does not exist")
```

On DynamoDB the same situation fails the `attribute_exists(pk) and attribute_exists(sk) and
#version = :version` condition (`dynamodb_memory.py:1192`) and surfaces as `VersionConflictError`
via `transact_write_safe`. The local backend diverges, breaking the "identical types on both
backends" guarantee stated in the 21.0.0 changelog.

**Change.**

```python
if v0_key not in data:
    raise VersionConflictError(
        "Resource does not exist",   # keep message
        resource_type=resource.__class__.__name__,
        resource_id=resource.resource_id,
        expected_version=previous_version,
        actual_version=None,
    )
```

### 2.3 `SingletonVersionedResource.saved_updated_singleton`

**Problem.** `extras/singleton.py:40-41` raises `ValueError("Cannot update from non-latest
version")` — the exact string the issue calls out — before delegating to `update_existing`.

**Change.**

```python
from ..exceptions import VersionConflictError

def saved_updated_singleton(self, memory: "DynamoDbMemory") -> Self:
    """..."""
    existing = self.ensure_exists(memory)
    if existing.version != self.version:
        raise VersionConflictError(
            "Cannot update from non-latest version",
            resource_type=type(self).__name__,
            resource_id=existing.resource_id,
            expected_version=self.version,
            actual_version=existing.version,
        )
    return memory.update_existing(existing, self)
```

Import from `..exceptions`, not from `..` (the package root), to avoid widening the existing
circular-import surface in `extras/`.

### 2.4 Public API after this change

No new names. Signatures unchanged. Behavioural contract additions, to state in docstrings:

- `DynamoDbMemory.update_existing` / `LocalStorageMemory.update_existing` — Raises
  `VersionConflictError` (with `actual_version=None`) when a versioned `existing_resource` no
  longer exists. Add a `Raises:` block to the `DynamoDbMemory.update_existing` docstring (it has
  none today) listing `VersionConflictError`, `ResourceExistsError` is not applicable here, and
  `TypeError` for a non-resource class.
- `SingletonVersionedResource.saved_updated_singleton` — Raises `VersionConflictError` when
  `self.version` is not the stored version. Add to its docstring.
- `ResourceRepository.save` docstring (`repository.py:220-224`): the `VersionConflictError` line
  already says "The resource is absent, or at a different version"; no change needed, it is now
  true on every path.

---

## 3. Tests — `tests/test_typed_conflicts_and_save.py`

All use the existing `dynamodb_memory` fixture (DynamoDB Local via Docker, `tests/conftest.py:86`)
and the module's own `local_storage` fixture and `Doc`/`CreateDoc`/`UpdateDoc` models. Add:

**In the "Non-transactional write path" section:**

```python
def test_update_of_a_concurrently_deleted_resource_is_a_conflict(dynamodb_memory):
    """A delete that lands after the caller's read is a lost race, not a lookup error."""
    doc = dynamodb_memory.create_new(Doc, CreateDoc(title="t", content="v1"))
    dynamodb_memory.delete_all_versions(doc.resource_id, Doc)

    with pytest.raises(VersionConflictError) as exc_info:
        dynamodb_memory.update_existing(doc, UpdateDoc(content="v2"))

    assert exc_info.value.expected_version == 1
    assert exc_info.value.actual_version is None
    assert exc_info.value.resource_id == doc.resource_id
    assert isinstance(exc_info.value, ValueError)  # back-compat


def test_local_storage_update_of_a_deleted_resource_is_a_conflict(local_storage):
    doc = local_storage.create_new(Doc, CreateDoc(title="t", content="v1"))
    local_storage.delete_all_versions(doc.resource_id, Doc)

    with pytest.raises(VersionConflictError) as exc_info:
        local_storage.update_existing(doc, UpdateDoc(content="v2"))
    assert exc_info.value.actual_version is None


def test_local_storage_versioned_write_with_missing_v0_matches_dynamodb(local_storage):
    """Backend parity for the write itself (not the pre-check): missing v0 is a conflict."""
    doc = local_storage.create_new(Doc, CreateDoc(title="t", content="v1"))
    next_version = doc.update_existing(UpdateDoc(content="v2"))
    local_storage.delete_all_versions(doc.resource_id, Doc)

    with pytest.raises(VersionConflictError, match="Resource does not exist"):
        local_storage._update_existing_versioned(next_version, previous_version=1)
```

**In the "save()" section:**

```python
def test_save_of_a_resource_deleted_after_the_precheck_is_a_conflict(repo, dynamodb_memory, monkeypatch):
    """The pre-read is advisory; the write is what guards. Simulate a delete between them."""
    doc = repo.save("doc-1", {"title": "t", "content": "v1"}, expected_version=0)

    real_update = dynamodb_memory.update_existing

    def delete_then_update(existing, *args, **kwargs):
        dynamodb_memory.delete_all_versions(existing.resource_id, Doc)
        return real_update(existing, *args, **kwargs)

    monkeypatch.setattr(dynamodb_memory, "update_existing", delete_then_update)

    with pytest.raises(VersionConflictError) as exc_info:
        repo.save("doc-1", {"content": "v2"}, expected_version=doc.version)
    assert exc_info.value.actual_version is None
```

Note: `monkeypatch.setattr` on the instance works because `ResourceRepository._update` calls
`self.ddb.update_existing(...)` (`repository.py:369`). The patched function receives the same
object `save` just fetched, so `existing.resource_id` is correct.

**New section "Singleton":**

```python
from simplesingletable.extras.singleton import SingletonVersionedResource


class Settings(SingletonVersionedResource):
    theme: str = "light"


def test_singleton_stale_save_raises_version_conflict(dynamodb_memory):
    first = Settings.ensure_exists(dynamodb_memory)
    first.theme = "dark"
    dynamodb_memory.update_existing(Settings.ensure_exists(dynamodb_memory), first)  # moves to v2

    stale = first.model_copy()  # still claims version 1
    stale.theme = "sepia"
    with pytest.raises(VersionConflictError) as exc_info:
        stale.saved_updated_singleton(dynamodb_memory)

    assert (exc_info.value.expected_version, exc_info.value.actual_version) == (1, 2)
    assert exc_info.value.resource_type == "Settings"
    assert exc_info.value.resource_id == "Settings"
    with pytest.raises(ValueError, match="Cannot update from non-latest version"):
        stale.saved_updated_singleton(dynamodb_memory)  # old handlers still match


def test_singleton_save_from_current_version_succeeds(dynamodb_memory):
    current = Settings.ensure_exists(dynamodb_memory)
    current.theme = "dark"
    saved = current.saved_updated_singleton(dynamodb_memory)
    assert saved.theme == "dark"
    assert saved.version == current.version + 1
```

If `Settings.ensure_exists` collides with another test's data in the shared session table, name
the class something unique to this module (e.g. `ConflictTestSettings`); the singleton id is the
class name.

Also extend the existing `test_passing_the_wrong_class_raises_type_error_not_value_error` — no
change needed, listed here only so the implementer does not duplicate it.

Run: `pytest tests/test_typed_conflicts_and_save.py tests/test_memory_versioning_methods.py
tests/test_local_storage.py tests/test_form_data.py` (the last exercises `FormConfig`, a
singleton), then the full `pytest`, then `inv lint`.

---

## 4. README

Add a `### Conflicts and save()` subsection under `## Important Patterns and Conventions`, directly
after `### Version Management` (README.md:423). Content, roughly 25 lines:

- The hierarchy diagram from `CHANGELOG.md:248-253`.
- One sentence: every conflict is a `ValueError` subclass, so old handlers work; map
  `ConflictError` → HTTP 409.
- The `save` example from `repository.py:201-204` with a one-line gloss of `expected_version=0`
  vs `>0`, and that `save` needs a versioned model.
- The structured attributes (`resource_type`, `resource_id`, `expected_version`, `actual_version`)
  and that `actual_version is None` means the resource was absent.

---

## 5. CHANGELOG entry

Add under `## [Unreleased]`, in a `### Fixed` subsection (create it after the existing
`### Changed` in that section). Do not add a version header.

```markdown
### Fixed

* **Three remaining untyped conflicts now raise `VersionConflictError`** (#13 follow-up). The
  21.0.0 conflict hierarchy missed three paths where losing a race still surfaced as a bare
  `ValueError`:

  * `update_existing` on a versioned resource that another writer **deleted** after the caller
    read it raised `ValueError("No item found with the provided key.")` from the internal
    re-read. It now raises `VersionConflictError` with `actual_version=None`, on both
    `DynamoDbMemory` and `LocalStorageMemory`. This also closes the window in
    `ResourceRepository.save` between its pre-check and the conditioned write.
  * `LocalStorageMemory._update_existing_versioned` raised `ValueError("Resource does not
    exist")` where DynamoDB's `attribute_exists` condition produces `VersionConflictError`.
    The local backend now matches.
  * `SingletonVersionedResource.saved_updated_singleton` raised
    `ValueError("Cannot update from non-latest version")`; it now raises `VersionConflictError`
    carrying `expected_version`/`actual_version`.

  Messages are unchanged and every new type is a `ValueError`, so existing handlers keep working.
  `README.md` gains a "Conflicts and save()" section documenting the hierarchy and
  `ResourceRepository.save(id, obj_in, expected_version)`.
```

---

## 6. Explicitly out of scope

- **`save()` for non-versioned resources.** `_put_nonversioned_resource`
  (`dynamodb_memory.py:1056-1063`) does an unconditional `put_item`, so `expected_version=0` has
  nothing to condition on, and there is no version for `>0`. The 21.0.0 changelog records this as
  "not covered yet". Supporting it means adding `ConditionExpression="attribute_not_exists(pk)"`
  to a new conditional-create path and mapping `ConditionalCheckFailedException` to
  `ResourceExistsError`. That is a feature with its own semantics decision (what does `>0` mean
  for a resource with no version?) and should be its own issue.
- **Changing `read_existing`'s `ValueError`** — a not-found on a plain read is not a conflict.
- **Other bare `ValueError`s** in `restore_version`, blob configuration, `increment_counter`,
  `form_data` — none are optimistic-concurrency failures.
