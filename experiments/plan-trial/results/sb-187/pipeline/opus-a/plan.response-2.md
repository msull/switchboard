# Response to round 2

1. Resolved. The plan did not change for this point.
2. Resolved. The plan did not change for this point.
3. Resolved. The plan did not change for this point.
4. Accepted as the minimum. The plan did not change for this point.

5. Resolved. The plan did not change for this point.

6. **`unanswered` is never cleared.** Agreed. Changed:
   - "The actions and effects", step 4: a new helper,
     `still_unanswered(a, source)`, returns `None` once the attempt being
     asked about has a `revisions` entry of its own, and returns
     `source.unanswered` otherwise. I chose the `revisions` test over a
     response file. The owner's resend is a revision of this attempt, so
     the sentence goes as soon as they act. It stays gone whether or not
     the planner has answered yet, and the "never answered" advice was
     only ever about resending. The `finalize` question and `ticket_paths`
     both read the helper, so they stop together.

     Steps 2 and 3 don't need the helper, and step 4 says why. They
     describe a launch, and the new attempt has no revisions yet. If an
     attempt the owner revised then fails, the walk reaches its
     revisions first, so `unanswered` comes from that attempt's own
     rounds.
   - "The UI", the `ticket_paths` bullet: `LaneFile::unanswered` is now
     filled through `still_unanswered`.
   - "The tests":
     - `a_resent_note_is_no_longer_named_as_unanswered` resends with
       `revise --file`, converges, and checks that the second `finalize`
       question no longer names the file and that `ticket_paths` gives
       `unanswered: None`.
     - The `serve.rs` `ticket_paths` test also checks `None` when the
       open attempt has a revision.
