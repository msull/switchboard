# Response to plan.feedback-1.md

All five points were accepted. Every edit was made in `plan.md` in this directory, and I re-checked the file
afterwards (headings, the test count and the summary table all show the new content).

1. **Test style – accepted. Changed: section 2 and the section 6 table.** `TestSingletonConflict` is gone. The
   tests are now module-level functions, e.g. `test_stale_singleton_save_raises_version_conflict(dynamodb_memory)`.
   They go after `test_local_storage_raises_the_same_types_as_dynamodb`, under a banner comment in the file's style.

2. **LocalStorage parity – accepted. Changed: section 2 (new test 4) and the section 6 table.** Added
   `test_local_storage_singleton_stale_save_raises_version_conflict(local_storage)`, mirroring the test at line 209.
   I ran a scratch script first. `SingletonVersionedResource.ensure_exists` and `update_existing` work on
   `LocalStorageMemory`, and a stale `saved_updated_singleton` there currently raises the bare
   `ValueError("Cannot update from non-latest version")`. So the new test fails before the fix and passes after.

3. **Fixture isolation – accepted. Changed: section 2 ("Fixture isolation (checked)").** `dynamodb_memory`
   (`tests/conftest.py:85`) is function-scoped and truncates the table before every test. `local_storage` is a fresh
   temp directory per test. `ensure_exists` therefore always starts at version 1, so I kept the absolute
   `version` asserts and wrote the reasoning into the plan.

4. **Stale hierarchy tree – accepted, and you were right. Changed: section 3(b) and the section 6 table.**
   `AGENT_KNOWLEDGE.md` (~lines 1770-1785) labels `VersionConflictError` a "Back-compat alias". In
   `exceptions.py` it is a real subclass, `VersionConflictError(TransactionConditionFailedError)` (line 127), under
   `ConflictError(ValueError)` (line 43). The plan now has a replacement tree. It also fixes the import path and the
   "All three classes carry …" sentence, which wrongly gives all three classes the same attributes. It tells the
   implementer to re-read `exceptions.py` lines 43-160 before writing, so the docs only claim what the classes
   define.

5. **`repo.save` docs scope – accepted. Changed: section 3 and section 4.** Section 3 is now split into (a), the
   singleton note that belongs with the fix, and (b), a docs catch-up for the already-shipped #13 that can go in its
   own PR. Item (b) covers the `repo.save` subsection and the hierarchy-tree fix. Section 4 adds a
   `### Internal` bullet for (b) next to the `### Fixed` bullet. `### Internal` is the heading 22.0.0 uses for
   non-library changes, and `## [Unreleased]` has no such subsection yet. The plan says to move that bullet with (b)
   if it is split out.

Nothing was left unchanged. The code change in section 1 is as before.
