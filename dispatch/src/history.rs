//! Which commits a code review's fix rounds fold into, as plans: no
//! I/O, no git. A plan is a list of groups, oldest first; each group is
//! one commit of the rewritten branch, the commits it replays in order
//! and the message and author it carries. The `Repo` port replays a
//! plan; the scheduler decides whether to.

use std::collections::BTreeMap;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

/// What a code review stage does to the branch's history as it
/// completes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Commits {
    /// The commits stay as the implementer and the fixers made them.
    #[default]
    Keep,
    /// Each fix round's commits fold into the commits they amend.
    Fold,
    /// The whole branch becomes one commit.
    One,
}

impl Commits {
    /// The name the pipeline file and the records use.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Keep => "keep",
            Self::Fold => "fold",
            Self::One => "one",
        }
    }
}

/// A commit of the branch, as `git log` reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub sha: String,
    /// How many parents it has: one, except for a root or a merge.
    pub parents: u32,
    /// The whole message; its first line is the subject.
    pub message: String,
}

impl Commit {
    #[must_use]
    pub fn subject(&self) -> &str {
        self.message.lines().next().unwrap_or("").trim()
    }
}

/// One commit of the rewritten branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    /// The commits replayed into it, oldest first; the first is the one
    /// the others fold into.
    pub picks: Vec<String>,
    pub message: String,
    /// The commit whose author (and committer) it keeps.
    pub author_of: String,
}

/// Whether replaying `groups` would rebuild `commits` as they are: every
/// group one commit with its own message.
#[must_use]
pub fn is_identity(commits: &[Commit], groups: &[Group]) -> bool {
    groups.len() == commits.len()
        && groups
            .iter()
            .zip(commits)
            .all(|(g, c)| g.picks.len() == 1 && g.picks[0] == c.sha && g.message == c.message)
}

/// The fold plan: a `fixup!` or `squash!` commit folds into its target
/// as git's autosquash would, and any other commit a fix round made
/// folds into the tip of the already-folded history at the head that
/// round reviewed. `ranges` are the rounds' `(head, head_after)`; one
/// whose ends are not on the branch is dropped.
///
/// # Errors
/// A merge (or a root) among `commits`: it cannot be replayed.
pub fn fold_plan(
    commits: &[Commit],
    base: &str,
    ranges: &[(String, String)],
) -> Result<Vec<Group>> {
    refuse_merges(commits, base)?;
    let anchors = fix_anchors(commits, base, ranges);
    let mut leader: Vec<usize> = Vec::with_capacity(commits.len());
    let mut squashed = vec![false; commits.len()];
    for (i, c) in commits.iter().enumerate() {
        let own = if let Some((target, squash)) = autosquash_target(c.subject()) {
            squashed[i] = squash;
            find_target(&commits[..i], target).map(|j| leader[j])
        } else if let Some(Anchor::At(anchor)) = anchors[i] {
            (0..=anchor).rev().find(|&k| leader[k] == k)
        } else {
            None
        };
        leader.push(own.unwrap_or(i));
    }
    let mut groups: Vec<Group> = Vec::new();
    let mut slot: BTreeMap<usize, usize> = BTreeMap::new();
    for (i, c) in commits.iter().enumerate() {
        let l = leader[i];
        if l == i {
            slot.insert(i, groups.len());
            groups.push(Group {
                picks: vec![c.sha.clone()],
                message: c.message.clone(),
                author_of: c.sha.clone(),
            });
            continue;
        }
        let g = &mut groups[slot[&l]];
        g.picks.push(c.sha.clone());
        if squashed[i] {
            let body = body_of(&c.message);
            if !body.is_empty() {
                g.message.push_str("\n\n");
                g.message.push_str(body);
            }
        }
    }
    Ok(groups)
}

/// The one-commit plan: every commit in one group, with the message and
/// author of the first commit no fix round made (the first commit when
/// every one is a fix).
///
/// # Errors
/// A merge (or a root) among `commits`.
pub fn one_plan(commits: &[Commit], base: &str, ranges: &[(String, String)]) -> Result<Vec<Group>> {
    refuse_merges(commits, base)?;
    let Some(first) = commits.first() else {
        return Ok(Vec::new());
    };
    let anchors = fix_anchors(commits, base, ranges);
    let lead = commits
        .iter()
        .zip(&anchors)
        .find(|(_, a)| a.is_none())
        .map_or(first, |(c, _)| c);
    Ok(vec![Group {
        picks: commits.iter().map(|c| c.sha.clone()).collect(),
        message: lead.message.clone(),
        author_of: lead.sha.clone(),
    }])
}

fn refuse_merges(commits: &[Commit], base: &str) -> Result<()> {
    if let Some(c) = commits.iter().find(|c| c.parents != 1) {
        bail!(
            "commit {} is a merge; the commits since {base} cannot be folded",
            c.sha
        );
    }
    Ok(())
}

/// The head a fix round reviewed, as a place on the branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Anchor {
    /// The base: the round reviewed no commit of the branch.
    Base,
    /// The commit at this index.
    At(usize),
}

/// Per commit: `None` when no fix round made it; otherwise where the
/// head that round reviewed is.
fn fix_anchors(commits: &[Commit], base: &str, ranges: &[(String, String)]) -> Vec<Option<Anchor>> {
    let index = |sha: &str| commits.iter().position(|c| c.sha == sha);
    let mut anchors = vec![None; commits.len()];
    for (head, after) in ranges {
        let from = if head == base {
            Some(Anchor::Base)
        } else {
            index(head).map(Anchor::At)
        };
        let (Some(from), Some(to)) = (from, index(after)) else {
            continue;
        };
        let start = match from {
            Anchor::Base => 0,
            Anchor::At(f) => f + 1,
        };
        for slot in anchors.iter_mut().take(to + 1).skip(start) {
            slot.get_or_insert(from);
        }
    }
    anchors
}

/// The target of a `fixup!` or `squash!` subject, its prefixes
/// stripped as git strips them, and whether the outermost was `squash!`.
fn autosquash_target(subject: &str) -> Option<(&str, bool)> {
    let squash = if subject.starts_with("squash! ") {
        true
    } else if subject.starts_with("fixup! ") {
        false
    } else {
        return None;
    };
    let mut rest = subject;
    while let Some(r) = rest
        .strip_prefix("fixup! ")
        .or_else(|| rest.strip_prefix("squash! "))
    {
        rest = r.trim_start();
    }
    Some((rest, squash))
}

/// Git's autosquash match among the earlier commits: the first whose
/// subject is `target`, else whose sha starts with it, else whose
/// subject starts with it.
fn find_target(earlier: &[Commit], target: &str) -> Option<usize> {
    if target.is_empty() {
        return None;
    }
    let is_sha = target.len() >= 4 && target.chars().all(|c| c.is_ascii_hexdigit());
    earlier
        .iter()
        .position(|c| c.subject() == target)
        .or_else(|| {
            is_sha
                .then(|| earlier.iter().position(|c| c.sha.starts_with(target)))
                .flatten()
        })
        .or_else(|| earlier.iter().position(|c| c.subject().starts_with(target)))
}

/// A message without its subject line and the blank lines after it.
fn body_of(message: &str) -> &str {
    message.split_once('\n').map_or("", |(_, rest)| rest.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(sha: &str, message: &str) -> Commit {
        Commit {
            sha: sha.to_owned(),
            parents: 1,
            message: message.to_owned(),
        }
    }

    fn r(head: &str, after: &str) -> (String, String) {
        (head.to_owned(), after.to_owned())
    }

    fn picks(groups: &[Group]) -> Vec<Vec<&str>> {
        groups
            .iter()
            .map(|g| g.picks.iter().map(String::as_str).collect())
            .collect()
    }

    #[test]
    fn a_fixup_folds_into_its_target_and_a_plain_fix_into_the_tip_it_reviewed() {
        let commits = [
            c("aaaa1", "A\n\nwhy A"),
            c("bbbb2", "B"),
            c("ffff1", "fixup! A"),
            c("pppp1", "plain fix"),
        ];
        let ranges = [r("bbbb2", "ffff1"), r("ffff1", "pppp1")];
        let groups = fold_plan(&commits, "base", &ranges).unwrap();
        assert_eq!(
            picks(&groups),
            [vec!["aaaa1", "ffff1"], vec!["bbbb2", "pppp1"]]
        );
        assert_eq!(groups[0].message, "A\n\nwhy A");
        assert_eq!(groups[1].message, "B");
        assert_eq!(groups[0].author_of, "aaaa1");
        assert_eq!(groups[1].author_of, "bbbb2");
        assert!(!is_identity(&commits, &groups));
    }

    #[test]
    fn a_fixup_matches_by_subject_then_sha_then_prefix() {
        let commits = [
            c("aaaa1", "Add the parser"),
            c("bbbb2", "Add"),
            c("ffff1", "fixup! Add"),
            c("ffff2", "fixup! aaaa"),
            c("ffff3", "fixup! Add the"),
            c("ffff4", "fixup! fixup! Add the parser"),
        ];
        let groups = fold_plan(&commits, "base", &[]).unwrap();
        assert_eq!(
            picks(&groups),
            [
                vec!["aaaa1", "ffff2", "ffff3", "ffff4"],
                vec!["bbbb2", "ffff1"]
            ]
        );
    }

    #[test]
    fn a_squash_appends_its_body_and_a_fixup_does_not() {
        let commits = [
            c("aaaa1", "A\n\nwhy A"),
            c("ffff1", "fixup! A\n\nfixup body"),
            c("ssss1", "squash! A\n\nsquash body"),
        ];
        let groups = fold_plan(&commits, "base", &[]).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].message, "A\n\nwhy A\n\nsquash body");
    }

    #[test]
    fn a_fixup_with_no_target_and_a_fix_before_any_commit_stay_their_own() {
        let commits = [
            c("pppp1", "plain fix"),
            c("aaaa1", "A"),
            c("ffff1", "fixup! nothing like it"),
        ];
        let ranges = [r("base", "pppp1"), r("aaaa1", "ffff1")];
        let groups = fold_plan(&commits, "base", &ranges).unwrap();
        assert_eq!(
            picks(&groups),
            [vec!["pppp1"], vec!["aaaa1"], vec!["ffff1"]]
        );
        assert!(is_identity(&commits, &groups));
    }

    #[test]
    fn implementation_fixups_fold_too() {
        let commits = [c("aaaa1", "A"), c("bbbb2", "B"), c("cccc3", "fixup! A")];
        let groups = fold_plan(&commits, "base", &[]).unwrap();
        assert_eq!(picks(&groups), [vec!["aaaa1", "cccc3"], vec!["bbbb2"]]);
    }

    #[test]
    fn a_range_off_the_branch_drops_out() {
        let commits = [c("aaaa1", "A"), c("bbbb2", "B"), c("pppp1", "plain fix")];
        // An earlier attempt reviewed heads a refresh has since rebased.
        let ranges = [r("old00001", "old00002"), r("bbbb2", "gone0001")];
        let groups = fold_plan(&commits, "base", &ranges).unwrap();
        assert!(is_identity(&commits, &groups));
        // A range read backwards is dropped too.
        let groups = fold_plan(&commits, "base", &[r("pppp1", "bbbb2")]).unwrap();
        assert!(is_identity(&commits, &groups));
        let groups = fold_plan(&commits, "base", &[r("bbbb2", "pppp1")]).unwrap();
        assert_eq!(picks(&groups), [vec!["aaaa1"], vec!["bbbb2", "pppp1"]]);
    }

    #[test]
    fn a_merge_is_refused() {
        let mut merge = c("mmmm1", "Merge main");
        merge.parents = 2;
        let commits = [c("aaaa1", "A"), merge];
        for plan in [fold_plan, one_plan] {
            let e = plan(&commits, "base0000", &[]).unwrap_err().to_string();
            assert_eq!(
                e,
                "commit mmmm1 is a merge; the commits since base0000 cannot be folded"
            );
        }
    }

    #[test]
    fn one_takes_the_first_implementation_commits_message() {
        let commits = [
            c("pppp1", "plain fix"),
            c("aaaa1", "A\n\nwhy A"),
            c("bbbb2", "B"),
        ];
        let groups = one_plan(&commits, "base", &[r("base", "pppp1")]).unwrap();
        assert_eq!(picks(&groups), [vec!["pppp1", "aaaa1", "bbbb2"]]);
        assert_eq!(groups[0].message, "A\n\nwhy A");
        assert_eq!(groups[0].author_of, "aaaa1");
        // Every commit a fix: the first one's.
        let groups = one_plan(&commits, "base", &[r("base", "bbbb2")]).unwrap();
        assert_eq!(groups[0].author_of, "pppp1");
        // One commit, or none: nothing to replay.
        let single = [c("aaaa1", "A")];
        assert!(is_identity(
            &single,
            &one_plan(&single, "base", &[]).unwrap()
        ));
        assert!(one_plan(&[], "base", &[]).unwrap().is_empty());
        assert!(is_identity(&[], &[]));
    }
}
