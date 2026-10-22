# Response to plan feedback, round 1

All five points accepted. Every edit is in `review/1/repo/plan.md`. The earlier plan at
`plan/1/repo/plan.md` is unchanged.

## 1. CHANGELOG: no invented `### Documentation` heading

**Changed: §3 "CHANGELOG entry" and §4 Checklist item 4.** The entry is now a new bullet at the
end of the existing `### Fixed` under `## [Unreleased]` (`CHANGELOG.md:88`), placed just
before `## [22.0.0]`. The plan now says explicitly not to add a new heading, because the file
follows Keep a Changelog and has never used `### Documentation`. The bullet text and the "no
version header" instruction are unchanged. Checklist item 4 now reads "Add the CHANGELOG
bullet to the existing `### Fixed` under `## [Unreleased]`."

## 2. §14 attribute wording settled in the plan

**Changed: §1.1(c).** The instruction to "check exceptions.py before finalizing" is gone. The
plan now tells the implementer to replace "All three classes carry:" and its two bullets with
your wording, verbatim: "`TransactionError` and its subclasses carry `cancellation_reasons` /
`operation_indexes`. Both are filled on a transaction `commit()`. On the non-transactional
path they may be empty, so use `expected_version` / `actual_version` there." The redundant
"(and `VersionConflictError`)" phrasing is removed. Below it, marked as not for pasting into
the doc, is the per-path list of which attributes each path fills (`commit()`,
`transact_write_safe`, the `update_existing` and `save` pre-checks, `ResourceExistsError`),
so the implementer can see why the wording is correct.

## 3. Quick Reference comment and `ResourceExistsError` tree comment

**Changed: §1.1(d) and §1.1(c).** The Quick Reference comment is now
`# Errors — ConflictError and its subclasses are ValueErrors; TransactionError is not`. In
the §14 tree, the `ResourceExistsError` comment now reads "Versioned create_new with a taken
id / save(..., 0). Non-versioned create_new is an unconditioned put and never raises this."
That matches the plan's own out-of-scope note.

## 4. The new test's name, docstring, assertion and note

**Changed: §2 "Test to add".** I traced the calls and you are right. `update_existing` →
`read_existing` (`dynamodb_memory.py:571`) → `self.get_existing` (`:541`) hits the patched
function, which returns v2, so the non-latest check at `:575` fires every time and the
`ConditionExpression` is never reached. My note that "which of the two fires depends on that
internal read" was wrong. Changes:
- The intro sentence now says the eviction branch covers a conflict "raised inside
  `_update`, after `save`'s own pre-read passed", not one coming "from the write".
- The test is renamed to `test_save_conflict_from_the_update_itself_evicts_the_cache`, and
  its docstring now describes the `_update` conflict.
- `pytest.raises` now captures `exc_info`, and the test adds
  `assert exc_info.value.actual_version == 2` to pin which check fired.
- The old implementation note is replaced with the deterministic call trace. A new note says
  this test is not evidence for the write-condition race: that case is covered by
  `test_losing_the_write_race_itself_raises_version_conflict` and the condition in
  `_update_existing_versioned`, which are what the §5 doc bullet rests on.

## 5. README bullet is required

**Changed: §1.2 and §4 Checklist item 2.** The heading is now "`README.md`: one bullet
(required)", with a sentence explaining why: `save` has shipped in 21.0.0 and 22.0.0, and
the feature list at `README.md:128-129` doesn't mention it. Checklist item 2 now reads "Add
the README bullet (1.2)", with "Optionally" removed.
