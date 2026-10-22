# Response to plan feedback 1

## 1. `save(expected_version=0)` raises `ResourceExistsError`, not `VersionConflictError`

**Agreed. The plan changed in two sections.**

I checked the claim against the code. `ResourceExistsError(ConflictError)` is at `exceptions.py:66`
and `VersionConflictError(TransactionConditionFailedError)` is at `:127`, so the two are siblings
under `ConflictError`. `test_resource_exists_is_a_conflict_but_not_a_transaction_error` pins this,
and `test_save_with_expected_version_zero_refuses_to_clobber` asserts that `save(..., 0)` on a taken
id raises `ResourceExistsError`. An `except VersionConflictError:` handler, as the issue proposes,
would miss a lost create race.

- **Evidence section, the paragraph after the table.** The heading is now "The shipped API differs
  from the issue's proposal in two intentional ways". Item 1 is the existing signature point (`id`
  instead of a pre-read object). Item 2 is new and covers the exception type. `0` raises
  `ResourceExistsError` and `>0` raises `VersionConflictError`. Both are `ConflictError`, so
  `except ConflictError:` is the handler that covers both modes, and `except VersionConflictError:`
  misses the create case. Item 2 cites the line numbers and both tests, and says the split is
  deliberate because a taken id is not a transaction condition failure.
- **Step 3: close #13, the draft comment.** I removed the misreadable "A lost race raises
  `ConflictError` in both cases". A new bullet says it outright: "`expected_version=0` raises
  `ResourceExistsError` and `>0` raises `VersionConflictError`. Both are `ConflictError`, so catch
  `ConflictError`, not `VersionConflictError`; `except VersionConflictError:` misses a lost create."

Nothing else in the plan changed. The point affects documentation only. It needs no code change,
test or CHANGELOG entry, because the behaviour is what shipped in 21.0.0 and tests already pin it.
