//! File paths an agent names in its prose, turned into links. The
//! message's Markdown is scanned for path-shaped tokens, the caller says
//! which of them name a file, and those are rewritten as links to a
//! private scheme that `ui::markdown::show_linked` reports when clicked.
//!
//! The scan and the existence checks run once per message text: the
//! result is kept in the caller's cache under a hash of the text, for
//! as long as that text is still in the conversation.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::path::PathBuf;

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

use super::RecordId;
use crate::ports::transcript::{Activity, ActivityKind, Conversation};

/// The link scheme of a rewritten path; the rest is an index into
/// [`Linked::targets`].
pub const SCHEME: &str = "switchboard-file:";

/// A message rewritten for drawing, and where each of its links goes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Linked {
    /// The message with each path that names a file made a link.
    pub markdown: String,
    /// The file and line of link `i`, by the `i` in its target.
    pub targets: Vec<(PathBuf, Option<u32>)>,
}

/// Linked messages by record and [`text_key`] of the message.
pub type Cache = HashMap<(RecordId, u64), Linked>;

/// Keep only `record`'s entries whose text is still in `conversation`;
/// with none, drop them all. Hashing is cheap next to the existence
/// checks a fresh scan would make, so a live agent's earlier messages
/// are not scanned again on every write to its transcript.
pub fn keep_only(cache: &mut Cache, record: RecordId, conversation: Option<&Conversation>) {
    let keep: HashSet<u64> = conversation
        .into_iter()
        .flat_map(prose_texts)
        .map(text_key)
        .collect();
    cache.retain(|(r, k), _| *r != record || keep.contains(k));
}

/// A path-shaped token found in a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRef {
    /// Bytes of the message it covers; for a code span, the backticks too.
    pub range: Range<usize>,
    /// The path as typed, without its line.
    pub path: String,
    /// The line it named (from 1), if any.
    pub line: Option<u32>,
}

/// The text a conversation draws for one activity row of kind `Text`:
/// the whole message, or its excerpt when that is all there is.
#[must_use]
pub fn message_text(a: &Activity) -> &str {
    a.text.as_deref().unwrap_or(&a.line)
}

/// Every piece of agent prose the conversation view links: the
/// messages written along the way and each turn's final answer.
pub fn prose_texts(c: &Conversation) -> impl Iterator<Item = &str> {
    c.turns.iter().flat_map(|t| {
        t.activity
            .iter()
            .filter(|a| a.kind == ActivityKind::Text)
            .map(message_text)
            .chain((!t.final_text.is_empty()).then_some(t.final_text.as_str()))
    })
}

/// The cache key of a message text.
#[must_use]
pub fn text_key(text: &str) -> u64 {
    let mut h = DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

/// The characters a path token may hold. Spaces, brackets and quotes
/// are left out, so a link made of one needs no escaping.
fn path_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/' | '~' | '@' | '+')
}

/// `token` as a path with an optional `:line` or `:line:col`, when it
/// looks like one. A line range (`:12-20`) counts as its first line; a
/// column is accepted but not kept, since the view scrolls to lines.
fn parse_token(token: &str) -> Option<(String, Option<u32>)> {
    if token.contains("://") {
        return None;
    }
    let mut parts = token.split(':');
    let path = parts.next()?;
    let mut numbers = Vec::new();
    for part in parts {
        let first = part.split_once('-').map_or(part, |(a, b)| {
            if b.chars().all(|c| c.is_ascii_digit()) {
                a
            } else {
                ""
            }
        });
        numbers.push(first.parse::<u32>().ok()?);
    }
    if numbers.len() > 2 || !path_shaped(path) {
        return None;
    }
    Some((path.to_owned(), numbers.first().copied()))
}

fn path_shaped(p: &str) -> bool {
    if !p.chars().all(path_char) || !p.chars().any(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    if ["/", "~/", "./", "../"].iter().any(|s| p.starts_with(s)) || p.contains('/') {
        return true;
    }
    // A bare `name.ext` with a short alphanumeric extension.
    p.rsplit_once('.').is_some_and(|(name, ext)| {
        !name.is_empty()
            && (1..=8).contains(&ext.len())
            && ext.chars().all(|c| c.is_ascii_alphanumeric())
    })
}

/// The path-shaped tokens of a prose run at `offset` in the message:
/// runs of path characters and `:`, less the punctuation a sentence
/// hangs on their end.
fn prose_refs(run: &str, offset: usize, out: &mut Vec<FileRef>) {
    let mut start = None;
    for (i, c) in run.char_indices().chain([(run.len(), ' ')]) {
        if path_char(c) || c == ':' {
            start.get_or_insert(i);
            continue;
        }
        let Some(s) = start.take() else {
            continue;
        };
        let token = run[s..i].trim_end_matches(['.', ',', ';', ':', '!', '?']);
        if let Some((path, line)) = parse_token(token) {
            out.push(FileRef {
                range: offset + s..offset + s + token.len(),
                path,
                line,
            });
        }
    }
}

/// Every path-shaped token in `markdown`'s prose and code spans. Code
/// blocks, links, images and headings are left alone.
#[must_use]
pub fn file_refs(markdown: &str) -> Vec<FileRef> {
    let mut out = Vec::new();
    // Inside a block whose text must not link, how deep.
    let mut skip = 0usize;
    // Pulldown cuts text at `_`, `*` and `[`; touching pieces are joined
    // before scanning so a name with an underscore stays whole.
    let mut run: Option<Range<usize>> = None;
    let flush = |run: &mut Option<Range<usize>>, out: &mut Vec<FileRef>| {
        if let Some(r) = run.take() {
            prose_refs(&markdown[r.clone()], r.start, out);
        }
    };
    let parser = Parser::new_ext(markdown, Options::ENABLE_TABLES);
    for (event, range) in parser.into_offset_iter() {
        match event {
            Event::Start(
                Tag::CodeBlock(_) | Tag::Link { .. } | Tag::Image { .. } | Tag::Heading { .. },
            ) => {
                flush(&mut run, &mut out);
                skip += 1;
            }
            Event::End(TagEnd::CodeBlock | TagEnd::Link | TagEnd::Image | TagEnd::Heading(_)) => {
                skip = skip.saturating_sub(1);
            }
            Event::Text(_) if skip == 0 => match &mut run {
                Some(r) if r.end == range.start => r.end = range.end,
                _ => {
                    flush(&mut run, &mut out);
                    run = Some(range);
                }
            },
            Event::Code(content) if skip == 0 => {
                flush(&mut run, &mut out);
                let token = content.trim();
                if token.chars().all(|c| path_char(c) || c == ':')
                    && let Some((path, line)) = parse_token(token)
                {
                    out.push(FileRef { range, path, line });
                }
            }
            _ => flush(&mut run, &mut out),
        }
    }
    flush(&mut run, &mut out);
    out
}

/// `text` with every token for which `find` gives a file made a link
/// to it. A token right after `!` is left as it is: the link would read
/// as Markdown image syntax and draw a broken image.
#[must_use]
pub fn link(text: &str, mut find: impl FnMut(&FileRef) -> Option<PathBuf>) -> Linked {
    let found: Vec<(FileRef, PathBuf)> = file_refs(text)
        .into_iter()
        .filter(|r| !text[..r.range.start].ends_with('!'))
        .filter_map(|r| find(&r).map(|p| (r, p)))
        .collect();
    let mut markdown = text.to_owned();
    // From the end back, so the ranges still ahead stay where they were.
    for (i, (r, _)) in found.iter().enumerate().rev() {
        let shown = &text[r.range.clone()];
        markdown.replace_range(r.range.clone(), &format!("[{shown}]({SCHEME}{i})"));
    }
    Linked {
        markdown,
        targets: found.into_iter().map(|(r, p)| (p, r.line)).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::transcript::Turn;

    fn one(text: &str) -> FileRef {
        let refs = file_refs(text);
        assert_eq!(refs.len(), 1, "{refs:?}");
        refs.into_iter().next().unwrap()
    }

    #[test]
    fn a_code_span_with_a_line() {
        let text = "See `src/core/action.rs:1629` there.";
        let r = one(text);
        assert_eq!(r.path, "src/core/action.rs");
        assert_eq!(r.line, Some(1629));
        assert_eq!(&text[r.range], "`src/core/action.rs:1629`");
    }

    #[test]
    fn prose_paths_of_each_shape() {
        assert_eq!(
            one("Edited src/core/action.rs today").path,
            "src/core/action.rs"
        );
        assert_eq!(
            one("in /Users/me/notes/plan.md").path,
            "/Users/me/notes/plan.md"
        );
        assert_eq!(one("see ~/x.md").path, "~/x.md");
        assert_eq!(one("the README.md file").path, "README.md");
        assert_eq!(one("my_long_name.rs").path, "my_long_name.rs");
        let r = one("at a.rs:12:4 now");
        assert_eq!((r.path.as_str(), r.line), ("a.rs", Some(12)));
        assert_eq!(one("lines a.rs:12-20").line, Some(12));
    }

    #[test]
    fn trailing_punctuation_is_the_sentence_s() {
        let text = "Read a/b.rs. Then c/d.rs, and (e/f.rs) or g.rs:3:";
        let paths: Vec<_> = file_refs(text)
            .into_iter()
            .map(|r| (r.path, r.line, text[r.range].to_owned()))
            .collect();
        assert_eq!(
            paths,
            vec![
                ("a/b.rs".into(), None, "a/b.rs".into()),
                ("c/d.rs".into(), None, "c/d.rs".into()),
                ("e/f.rs".into(), None, "e/f.rs".into()),
                ("g.rs".into(), Some(3), "g.rs:3".into()),
            ]
        );
    }

    #[test]
    fn code_blocks_links_urls_and_numbers_are_left_alone() {
        assert!(file_refs("```\nsrc/a.rs\n```\n").is_empty());
        assert!(file_refs("[src/a.rs](src/a.rs)").is_empty());
        assert!(file_refs("see https://example.com/a/b.html").is_empty());
        assert!(file_refs("version 1.2.3 and 10/20").is_empty());
        assert!(file_refs("# src/a.rs").is_empty());
        assert!(file_refs("`cargo test --locked`").is_empty());
    }

    #[test]
    fn the_rewrite_links_only_what_find_finds() {
        let file = PathBuf::from("/p/src/a.rs");
        let find = |r: &FileRef| (r.path == "src/a.rs").then(|| file.clone());
        let linked = link("See `src/a.rs:3` and src/a.rs, not src/b.rs.", find);
        assert_eq!(
            linked.markdown,
            format!("See [`src/a.rs:3`]({SCHEME}0) and [src/a.rs]({SCHEME}1), not src/b.rs.")
        );
        assert_eq!(linked.targets, vec![(file.clone(), Some(3)), (file, None)]);
    }

    #[test]
    fn a_path_after_a_bang_is_not_made_an_image() {
        let find = |r: &FileRef| Some(PathBuf::from("/p").join(&r.path));
        let linked = link("add !build/keep.txt, see `src/a.rs`", find);
        assert_eq!(
            linked.markdown,
            format!("add !build/keep.txt, see [`src/a.rs`]({SCHEME}0)")
        );
        assert_eq!(linked.targets.len(), 1);
    }

    #[test]
    fn prose_texts_are_what_the_view_draws() {
        let row = |kind, line: &str, text: Option<&str>| Activity {
            kind,
            line: line.into(),
            at: None,
            error: false,
            detail: None,
            text: text.map(Into::into),
        };
        let c = Conversation {
            turns: vec![
                Turn {
                    activity: vec![
                        row(ActivityKind::Text, "excerpt", None),
                        row(ActivityKind::Tool, "Read a.rs", None),
                        row(ActivityKind::Text, "short", Some("whole")),
                    ],
                    final_text: "answer".into(),
                    ..Turn::default()
                },
                Turn::default(),
            ],
            ..Conversation::default()
        };
        assert_eq!(
            prose_texts(&c).collect::<Vec<_>>(),
            vec!["excerpt", "whole", "answer"]
        );
    }
}
