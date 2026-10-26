//! The changelog the binary was built with, and a found release's
//! notes, as sections of titled groups of plain-text entries for the
//! What's new view. Pure: the text comes in as a string, and the
//! built-in one is compiled in.

use std::sync::OnceLock;

use super::action::AppCore;
use super::updates::UpdateState;

/// `CHANGELOG.md` as it stood when this binary was built. Fragments
/// under `changelog.d/` are not in it until a release folds them.
pub const BUILT_IN: &str = include_str!("../../CHANGELOG.md");

/// One `## [version] - date` section of the changelog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// `0.7.3`, or `Unreleased`.
    pub version: String,
    /// `2026-10-26`; `None` for Unreleased.
    pub date: Option<String>,
    /// The section's titled groups; one with no entries is dropped.
    pub groups: Vec<Group>,
}

/// One `### Title` group: `Added`, `Changed`, `Fixed`. Entries before
/// any heading land in a group with an empty title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    /// `Added`, `Changed`, `Fixed`; empty for entries under no heading.
    pub title: String,
    /// Each entry's text, its wrapped lines joined by a space.
    pub items: Vec<String>,
}

/// What a line of a changelog is to the parser.
enum Line<'a> {
    Blank,
    Group(&'a str),
    Item(&'a str),
    Continuation(&'a str),
    Other,
}

fn classify(line: &str) -> Line<'_> {
    if line.trim().is_empty() {
        Line::Blank
    } else if let Some(title) = line.strip_prefix("### ") {
        Line::Group(title.trim())
    } else if let Some(text) = line.strip_prefix("- ") {
        Line::Item(text.trim())
    } else if line.starts_with([' ', '\t']) {
        Line::Continuation(line.trim())
    } else {
        Line::Other
    }
}

/// Groups being built: the open entry takes continuation lines until a
/// blank line or anything else closes it.
#[derive(Default)]
struct Groups {
    groups: Vec<Group>,
    open: bool,
}

impl Groups {
    /// Take one line; false when it is none of a group's lines.
    fn push(&mut self, line: &str) -> bool {
        match classify(line) {
            Line::Blank => self.open = false,
            Line::Group(title) => {
                self.groups.push(Group {
                    title: title.to_owned(),
                    items: Vec::new(),
                });
                self.open = false;
            }
            Line::Item(text) => {
                if self.groups.is_empty() {
                    self.groups.push(Group {
                        title: String::new(),
                        items: Vec::new(),
                    });
                }
                if let Some(group) = self.groups.last_mut() {
                    group.items.push(text.to_owned());
                }
                self.open = true;
            }
            Line::Continuation(text) if self.open => {
                if let Some(item) = self.groups.last_mut().and_then(|g| g.items.last_mut()) {
                    item.push(' ');
                    item.push_str(text);
                }
            }
            Line::Continuation(_) | Line::Other => {
                self.open = false;
                return false;
            }
        }
        true
    }

    fn finish(self) -> Vec<Group> {
        self.groups
            .into_iter()
            .filter(|g| !g.items.is_empty())
            .collect()
    }
}

/// A section being built: its heading, then its lines.
struct Building {
    version: String,
    date: Option<String>,
    groups: Groups,
}

impl Building {
    /// `[0.7.3] - 2026-10-26` into its version and date.
    fn new(heading: &str) -> Self {
        let heading = heading.trim();
        let (version, rest) = match heading.strip_prefix('[').and_then(|h| h.split_once(']')) {
            Some((version, rest)) => (version.trim(), rest),
            None => (heading, ""),
        };
        let date = rest.trim().trim_start_matches('-').trim().to_owned();
        Self {
            version: version.to_owned(),
            date: (!date.is_empty()).then_some(date),
            groups: Groups::default(),
        }
    }

    fn finish(self) -> Section {
        Section {
            version: self.version,
            date: self.date,
            groups: self.groups.finish(),
        }
    }
}

/// The sections of a Keep a Changelog file, in file order (newest
/// first). The preamble before the first `##` is skipped, as is any
/// line inside a section that is not a heading or an entry.
#[must_use]
pub fn parse_changelog(text: &str) -> Vec<Section> {
    let mut sections = Vec::new();
    let mut current: Option<Building> = None;
    for line in text.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            sections.extend(
                current
                    .replace(Building::new(heading))
                    .map(Building::finish),
            );
        } else if let Some(building) = current.as_mut() {
            building.groups.push(line);
        }
    }
    sections.extend(current.map(Building::finish));
    sections
}

/// How many lines of a body that is not changelog-shaped are shown.
const PLAIN_LINES: usize = 12;

/// A found release's notes as groups. The body is the release's
/// changelog section without its `##` line, then the footer
/// `scripts/release.sh` writes; reading stops at the first line that
/// is not part of a group, and at `Built from`, so the footer never
/// shows. A body with no entries at all (one edited by hand) comes
/// back as its first lines in one untitled group, so the notes are
/// never silently empty.
#[must_use]
pub fn parse_notes(body: &str) -> Vec<Group> {
    let mut groups = Groups::default();
    for line in body.lines() {
        if line.starts_with("Built from") || !groups.push(line) {
            break;
        }
    }
    let groups = groups.finish();
    if !groups.is_empty() {
        return groups;
    }
    let plain = body
        .lines()
        .take_while(|l| !l.starts_with("Built from"))
        .take(PLAIN_LINES)
        .collect::<Vec<_>>()
        .join("\n");
    let plain = plain.trim();
    if plain.is_empty() {
        return Vec::new();
    }
    vec![Group {
        title: String::new(),
        items: vec![plain.to_owned()],
    }]
}

fn built_in() -> &'static [Section] {
    static SECTIONS: OnceLock<Vec<Section>> = OnceLock::new();
    SECTIONS.get_or_init(|| parse_changelog(BUILT_IN))
}

/// What a section of the What's new view says about itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotesTag {
    /// A found release that is not downloaded or not yet installed.
    NotInstalled,
    /// A found release that is downloaded and verified.
    Ready,
    /// The version that is running.
    Running,
}

impl NotesTag {
    /// The words shown beside the heading.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::NotInstalled => "Not installed",
            Self::Ready => "Ready to install",
            Self::Running => "Running",
        }
    }

    /// Whether the section is a release found online, not yet running.
    #[must_use]
    pub fn is_found(self) -> bool {
        self != Self::Running
    }
}

/// One section of the What's new view, ready to draw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotesView {
    /// `0.7.3`, or a found release's version.
    pub version: String,
    /// `2026-10-26`, when the section has a date.
    pub date: Option<String>,
    /// `None` for a released version below the running one.
    pub tag: Option<NotesTag>,
    /// The entries to show.
    pub groups: Vec<Group>,
}

impl AppCore {
    /// The built-in changelog's released sections at or below the
    /// running version, newest first. Unreleased, and any section the
    /// running version does not reach, are left out, so nothing is
    /// shown as installed that is not.
    #[must_use]
    pub fn changelog(&self) -> Vec<&'static Section> {
        sections_up_to(built_in(), self.version())
    }

    /// The What's new view: a found release's notes first, then the
    /// changelog sections that have entries.
    #[must_use]
    pub fn whats_new(&self) -> Vec<NotesView> {
        let found = match self.update() {
            UpdateState::Available(release) | UpdateState::Downloading(release) => {
                Some((release, NotesTag::NotInstalled))
            }
            UpdateState::Ready { release, .. } => Some((release, NotesTag::Ready)),
            _ => None,
        };
        let found = found.map(|(release, tag)| NotesView {
            version: release.version.clone(),
            date: release
                .published_at
                .as_deref()
                .and_then(|d| d.get(..10))
                .map(str::to_owned),
            tag: Some(tag),
            groups: parse_notes(&release.notes),
        });
        let running = self.version();
        let released = self
            .changelog()
            .into_iter()
            .filter(|s| !s.groups.is_empty())
            .map(|s| NotesView {
                version: s.version.clone(),
                date: s.date.clone(),
                tag: (s.version == running).then_some(NotesTag::Running),
                groups: s.groups.clone(),
            });
        found.into_iter().chain(released).collect()
    }
}

/// `sections` whose version parses and is at or below `current`.
#[must_use]
pub fn sections_up_to<'a>(sections: &'a [Section], current: &str) -> Vec<&'a Section> {
    let Ok(current) = semver::Version::parse(current.trim_start_matches('v')) else {
        return Vec::new();
    };
    sections
        .iter()
        .filter(|s| {
            semver::Version::parse(s.version.trim_start_matches('v')).is_ok_and(|v| v <= current)
        })
        .collect()
}
