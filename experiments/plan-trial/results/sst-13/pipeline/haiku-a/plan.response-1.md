# Response to plan.feedback-1.md

Edited copy: `/Users/sbgragent/Library/Application Support/Dispatch/tickets/ce7eaf93/review/1/repo/plan.md`. I re-read it after the edits and confirmed each change below is in the file. The original plan at `plan/1/repo/plan.md` is unchanged.

1. **AGENT_KNOWLEDGE.md not updated. Accepted.**
   - Section 2 has a new subsection, `### File: AGENT_KNOWLEDGE.md`. It replaces the "Key behavior" bullet at about line 966 so it names `VersionConflictError`, `ConflictError`, and `ValueError`.
   - Section 6 has a checklist item for it.
   - The Exception hierarchy block (about line 1770) is not extended. It documents only the transaction types imported from `simplesingletable.transactions`. Covering the whole conflict family there is a separate docs task, and the plan says so.

2. **"Type-only, not breaking" overstated. Accepted.**
   - Section 4 now says the change is not breaking for `except ValueError`, but it does broaden `except TransactionError`. It gives the inheritance chain as the reason, and says the same condition already raises `VersionConflictError` in `update_existing` (`dynamodb_memory.py:576`).
   - The CHANGELOG bullet itself now says that a caller catching `TransactionError` will also catch this case.
   - I confirmed with grep that nothing in `src/` catches `TransactionError`.

3. **LocalStorageMemory test was optional. Accepted.**
   - Section 3, test 5 is now required: `test_local_storage_stale_singleton_raises_version_conflict(local_storage)`. It uses the `local_storage` fixture from the backend-parity test at line 209.
   - The plan cites `get_existing` accepting `consistent_read` (`local_storage_memory.py:221`) and `create_new` accepting `override_id` (`:343`).
   - The skip-or-shim language is gone, and the plan says not to add one.

4. **Close-out contradicts the code change. Accepted, with the first option.**
   - Section 1, item 4: #13 closes after this branch merges, not as delivered in 21.0.0 alone.
   - Section 5: the close-out comment covers both the 21.0.0 delivery and the singleton fix, which is reported as merged in the next release. Closing waits until after the merge.
   - Section 5, last paragraph: the singleton fix ships on this branch, so #13 stays open until it merges.
   - Section 6: the closing checklist item now says closure follows the merge and user confirmation.

5. **New tests lack the file's docstring convention. Accepted, with one correction.**
   - Section 3 now requires a one-line docstring on every new test, in the file's style.
   - Test 2's docstring must record the back-compat intent: a caller on `except ValueError` or `except ConflictError` keeps catching a stale singleton write.
   - Correction: the feedback says every test in the file has a docstring. Most do, but `test_create_with_taken_id_raises_resource_exists` does not, so I did not repeat the "every test" claim. The rule applies to the new tests only.
