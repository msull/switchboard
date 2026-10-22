Issue #13: Typed conflict errors on the non-transactional write path, and a save(obj, expected_version) primitive

Split out of #10. These two are grouped because the typed exceptions are the prerequisite
for the `save` primitive having anything meaningful to raise — shipping them together
means callers delete their message-matching workaround once instead of twice.

Verified on `main` at **19.0.0**.

---

## 1. Conflicts are typed in the transaction path and bare `ValueError` everywhere else

`TransactionConditionFailedError` and `VersionConflictError` exist
(`transactions.py:91,101`), and 18.1.0 added `exceptions.py` for blobs. The
non-transactional write path did not get the same treatment:

- `transact_write_safe` turns every `TransactionCanceledException` — including
  `ConditionalCheckFailed` — into `raise ValueError(f"Transaction failed: {...}")`
  (`dynamodb_memory.py:190`).
- `update_existing` raises `ValueError("Cannot update from non-latest version")`
  (`dynamodb_memory.py:442`) for precisely the condition `VersionConflictError` names.

`ResourceRepository.create/update` route through those, so a repository caller cannot
distinguish a lost optimistic write from a programming error, and ends up doing this:

```python
@staticmethod
def _is_lost_race(exc: ValueError) -> bool:
    message = str(exc)
    return "ConditionalCheckFailed" in message or "non-latest version" in message
```

Matching exception *message text* to choose between HTTP 409 and HTTP 400 is not somewhere
callers should be — it breaks silently if the wording ever changes. It is sharper for any
app that maps `ValueError -> 400` globally, where an untranslated conflict reaches the
operator as "bad request".

Both types already exist and `exceptions.py` now sets the precedent for where they live.
Keeping them as `ValueError` subclasses preserves every existing handler, the same way the
blob exceptions in `exceptions.py` do.

## 2. No "save with expected version" primitive

The most common shape is *save this, fail if it changed since I read it, where version 0
means it must not exist yet*. Each half is easy; the combination is not:

- `_build_versioned_update_items` reads the current state and raises
  `ResourceNotFoundError` when absent (`transactions.py:809`), so the update path cannot
  create.
- `get_or_create` erases the distinction, which defeats the guard.
- Passing the **pre-read object** rather than the id is what makes the library condition
  `v0` on that exact version — the pre-read alone is TOCTOU, and nothing in the signature
  says so.

So every caller hand-rolls the branch, and it is the part most likely to be got subtly
wrong by the next person.

**Proposal:** `repo.save(obj_in, expected_version: int)` — `0` creates conditionally,
`>0` updates from that exact pre-image, `VersionConflictError` either way. That removes
the branch, the string-matching above, and the TOCTOU footgun together.

Per #10 this is the single feature most wanted from that round of feedback.


Write a plan a second engineer could implement without asking questions: the files to change, the new or changed public API with signatures, the tests to add in tests/ (they run against DynamoDB Local), and the CHANGELOG entry under ## [Unreleased]. Do not plan a version bump or a release; inv fullrelease does both.

Read the repository first. Then write the plan to plan.md in the current directory, and write nothing else.
