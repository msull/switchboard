# Response to plan feedback 1

Edited file: `/Users/sbgragent/Library/Application Support/Dispatch/tickets/c3a45d04/review/1/repo/plan.md`. The earlier plan at `tickets/c3a45d04/plan/1/repo/plan.md` is unchanged.

## 1. `update(id)` keeps the false conflict. Accepted.

Confirmed in the code: `update(id)` calls `self.read(id)` (`extras/repository.py:178`), which calls `get()` (`:128-133`), which calls `_get()` (`:351`). `_get` uses the cache, then a default read.

Changed: **§2** adds path C (`update(id)`) to the list of stale reads and says all three must change. **§4.1 change 2** adds a `consistent_read` keyword to `_get` and makes `update()`'s id branch call `_get(id, consistent_read=True)`, with the same not-found `ValueError` that `read()` raises.

Decision on the cache: I did **not** bypass it. A cache hit can only be stale if another process wrote the record after it was cached. That is a real conflict and the behaviour the library already had, not replica lag. Bypassing the cache would also remove every cache hit from `update(id)`. This is recorded in **§2** ("Cache on the `update(id)` path"). **§9** lists the `_get` and `update()` changes.

## 2. T4 cannot pass after the fix as written. Accepted.

Confirmed: with the old `read()` path, T4 would still get v1 and fail. With point 1 fixed, that is no longer true.

Changed: **§6 T4** now clears the cache before the update, so the test exercises the miss path that point 1 fixes. Its docstring says why. **§7 step 2** now says T4 passes after **§4.1 change 2**. I checked the sequence: before the fix, the pre-image read returns v1 and the conditional write fails. After it, the pre-image read is consistent and the call returns v3.

## 3. The lagging-read fake in T3 and T4 is too broad. Accepted.

Changed: **§6** replaces the two inline fakes with one module-level helper, `_lag_latest_reads`. It lags only default (`consistent_read=False`) reads with `version=0`, for this record id and this model class. Every other read passes through unchanged. The docstring says DynamoDB Local is always strongly consistent and that the lag is simulated. The fake also drops the note about `self`, which was wrong for an instance-level patch.

I did not add call-site assertions inside T3 and T4. **§6 T1** already asserts the `consistent_read` flag directly on each call site. T3 and T4 check the observable result.

## 4. Section 8 lists outward-facing actions with no confirmation gate. Accepted.

Changed: **§8** is retitled as outward-facing. Each of the four actions is marked "Needs go-ahead". The section says the implementer does not run any of them without the user's explicit approval for that specific action.

## 5. Nits: CHANGELOG lines and test count. Accepted, both verified.

- **CHANGELOG position.** Changed: **§5** gives the exact lines. The `### Fixed` heading is at `CHANGELOG.md:88`, the last line of the #18 entry is `:106` (`find them.`), line 107 is blank, and `## [22.0.0]` is at line 108. The new bullet goes after line 106. The §5 entry text also now names `update(id)`.
- **Test count.** The file has 23 `def test_` functions. Counted by fixture: 18 use `repo` or `dynamodb_memory`, 1 uses `local_storage`, and 4 use no fixture. Changed: **§1** table row says 23 (18 / 1 / 4), replacing "20+". **§6** states the same split. **§7 step 1** keeps "18 Docker-backed", which is now the verified figure.

## Other changes

I also checked the subclasses before editing `repository.py`. `VersionedResourceRepository` inherits `_get`, `get`, `read` and `update` without overriding them. `ReadOnlyResourceRepository` is a separate class with no `update`. This is noted in **§4**. **§9** is the summary table, updated for the `update(id)` change.

I did not run any tests. Docker is still not available here, so the test results in **§7** are not verified.
