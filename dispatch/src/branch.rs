//! A ticket's branch name from the pipeline's template.
//!
//! `[project] branch` (or a lane's own `branch`, on a lane with a
//! repository of its own) is a template of four fields: `{number}`,
//! `{slug}`, `{key}` and `{type}`. Absent, the name is
//! `dispatch/{number}-{slug}` as [`crate::git::branch_name`] makes it.
//!
//! This is not [`crate::template::Vars::render`], which leaves an unset
//! field as written: a branch name must never carry a literal `{key}`,
//! so every field here either renders or refuses. A take renders every
//! context's name once and refuses on any error, so the cut, which
//! renders them again from the ticket's own copy of the file, cannot
//! fail later. There is no fallback to the default name: a repository
//! that wants a convention may refuse a push of any other.

use anyhow::{Result, anyhow, bail};
use regex::Regex;

use crate::git::{branch_name, slug};
use crate::pipeline::{Lane, Pipeline, Source};
use crate::ticket::SourceSnapshot;

/// One of a template's four fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Number,
    Slug,
    Key,
    Type,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Part {
    Text(String),
    Field(Field),
}

/// The template's parts, or why it is not one: a `{` or `}` that does
/// not belong to one of the four fields.
fn parse(template: &str) -> Result<Vec<Part>, String> {
    let mut parts = Vec::new();
    let mut text = String::new();
    let mut rest = template;
    while let Some(c) = rest.chars().next() {
        match c {
            '{' => {
                let Some(end) = rest.find('}') else {
                    return Err("a `{` with no `}`".to_owned());
                };
                let name = &rest[1..end];
                let field = match name {
                    "number" => Field::Number,
                    "slug" => Field::Slug,
                    "key" => Field::Key,
                    "type" => Field::Type,
                    _ => {
                        return Err(format!(
                            "unknown field {{{name}}} (the fields are {{number}}, {{slug}}, {{key}} and {{type}})"
                        ));
                    }
                };
                if !text.is_empty() {
                    parts.push(Part::Text(std::mem::take(&mut text)));
                }
                parts.push(Part::Field(field));
                rest = &rest[end + 1..];
            }
            '}' => return Err("a `}` with no `{`".to_owned()),
            _ => {
                text.push(c);
                rest = &rest[c.len_utf8()..];
            }
        }
    }
    if !text.is_empty() {
        parts.push(Part::Text(text));
    }
    Ok(parts)
}

/// The template `lane`'s branch is named by: its own on a lane with a
/// repository, else the project's. `None` for the default name.
fn template_for<'a>(p: &'a Pipeline, lane: Option<&'a Lane>) -> Option<&'a str> {
    lane.filter(|l| l.repo.is_some())
        .and_then(|l| l.branch.as_deref())
        .or(p.project.branch.as_deref())
}

/// The name of the ticket's tree: the project's template, rendered.
pub fn tree(p: &Pipeline, s: &SourceSnapshot) -> Result<String> {
    render(p, None, s)
}

/// The name of `lane`'s branch. A lane with a repository of its own
/// takes its own template over the project's; any other shares the
/// ticket's tree and so its name.
pub fn lane_branch(p: &Pipeline, lane: &Lane, s: &SourceSnapshot) -> Result<String> {
    render(p, Some(lane), s)
}

/// Every name a cut would record, by lane: the tree first (lane `None`,
/// since the tree belongs to no lane), then each lane with a repository
/// of its own (`Some(name)`). Rendered at take and at
/// restart so a template that cannot name a branch is refused then.
pub fn names<'a>(p: &'a Pipeline, s: &SourceSnapshot) -> Result<Vec<(Option<&'a str>, String)>> {
    let mut out = vec![(None, tree(p, s)?)];
    for lane in p.lanes.iter().filter(|l| l.repo.is_some()) {
        out.push((Some(lane.name.as_str()), lane_branch(p, lane, s)?));
    }
    Ok(out)
}

fn render(p: &Pipeline, lane: Option<&Lane>, s: &SourceSnapshot) -> Result<String> {
    let number = s.number.unwrap_or(0);
    let Some(template) = template_for(p, lane) else {
        return Ok(branch_name(number, &s.title));
    };
    let fail = |why: String| anyhow!("branch template {template:?}: {why}");
    let parts = parse(template).map_err(fail)?;
    let uses = |f: Field| parts.contains(&Part::Field(f));
    let mut title = s.title.clone();
    let key = if uses(Field::Key) {
        let pattern = p
            .project
            .branch_key
            .as_deref()
            .ok_or_else(|| fail("{key} needs [project] branch_key".to_owned()))?;
        let re = Regex::new(pattern).map_err(|e| fail(format!("branch_key: {e}")))?;
        let caps = re.captures(&s.title).ok_or_else(|| {
            fail(format!(
                "no key matches {pattern} in the title {:?}",
                s.title
            ))
        })?;
        // A group named `key` is the key even when it did not take part in
        // the match; only a pattern without one falls back to group 1.
        let named = re.capture_names().any(|n| n == Some("key"));
        let found = if named { caps.name("key") } else { caps.get(1) }.ok_or_else(|| {
            fail(format!(
                "branch_key {pattern} matched without its key group"
            ))
        })?;
        // The key leads the name, so it is not repeated in the slug.
        let whole = caps.get(0).expect("group 0 is the match");
        title.replace_range(whole.range(), " ");
        let key = sanitize(found.as_str());
        if key.is_empty() {
            return Err(fail(format!(
                "the key {:?} has no character a branch name may hold",
                found.as_str()
            )));
        }
        key
    } else {
        String::new()
    };
    let kind = if uses(Field::Type) {
        let raw = s
            .labels
            .iter()
            .find_map(|l| l.strip_prefix("type:"))
            .map(str::trim)
            .or(p.project.branch_type.as_deref())
            .ok_or_else(|| {
                fail("{type} needs a type: label on the issue or [project] branch_type".to_owned())
            })?;
        let kind = sanitize(raw);
        if kind.is_empty() {
            return Err(fail(format!(
                "the type {raw:?} has no character a branch name may hold"
            )));
        }
        kind
    } else {
        String::new()
    };
    let slug_text = slug(&title);
    let mut name = String::new();
    for part in &parts {
        match part {
            Part::Text(t) => name.push_str(t),
            Part::Field(Field::Number) => name.push_str(&number.to_string()),
            Part::Field(Field::Slug) => name.push_str(&slug_text),
            Part::Field(Field::Key) => name.push_str(&key),
            Part::Field(Field::Type) => name.push_str(&kind),
        }
    }
    // An empty slug leaves the hyphen before it.
    let name = name.trim_end_matches('-');
    check_ref_name(name).map_err(fail)?;
    Ok(name.to_owned())
}

/// `value` reduced to `[A-Za-z0-9._-]`: every other character a hyphen,
/// runs of hyphens as one, no leading `-` or `.`, no trailing `.`.
fn sanitize(value: &str) -> String {
    let mut out = String::new();
    for c in value.chars() {
        let c = if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
            c
        } else {
            '-'
        };
        if c == '-' && out.ends_with('-') {
            continue;
        }
        out.push(c);
    }
    out.trim_start_matches(['-', '.'])
        .trim_end_matches('.')
        .to_owned()
}

/// Whether git takes `name` as a branch: the rules of
/// `git check-ref-format --branch`, with the one broken as the error.
fn check_ref_name(name: &str) -> Result<(), String> {
    let refuse = |rule: &str| Err(format!("{name:?} is not a branch name: {rule}"));
    if name.is_empty() {
        return refuse("it is empty");
    }
    if name == "@" {
        return refuse("it is `@`");
    }
    if name.starts_with('-') {
        return refuse("it starts with `-`");
    }
    if name.starts_with('/') || name.ends_with('/') {
        return refuse("it starts or ends with `/`");
    }
    if name.contains("//") {
        return refuse("it has `//`");
    }
    if name.ends_with('.') {
        return refuse("it ends with `.`");
    }
    if name.contains("..") {
        return refuse("it has `..`");
    }
    if name.contains("@{") {
        return refuse("it has `@{`");
    }
    if let Some(c) = name.chars().find(|c| {
        c.is_ascii_control() || matches!(c, ' ' | '~' | '^' | ':' | '?' | '*' | '[' | '\\')
    }) {
        return refuse(&format!("it has {c:?}"));
    }
    for part in name.split('/') {
        if part.starts_with('.') {
            return refuse("a component starts with `.`");
        }
        // Git's rule is exact: `x.LOCK` is a valid name.
        if part.strip_suffix(".lock").is_some() {
            return refuse("a component ends with `.lock`");
        }
    }
    Ok(())
}

/// The checks a pipeline's branch keys need before any ticket: each
/// template's fields, a `branch_key` for `{key}` with one capture group
/// or a group named `key`, a lane's `branch` only on a lane with its
/// own repository, and none of them on a pull-request pipeline, whose
/// tickets keep the pull requests' branches.
pub(crate) fn validate(p: &Pipeline) -> Result<()> {
    let project = &p.project;
    let any = project.branch.is_some()
        || project.branch_key.is_some()
        || project.branch_type.is_some()
        || p.lanes.iter().any(|l| l.branch.is_some());
    if any && p.source == Source::PullRequest {
        bail!(
            "a pull-request pipeline keeps the pull requests' branches: branch, branch_key and branch_type do nothing"
        );
    }
    let mut templates = Vec::new();
    if let Some(t) = &project.branch {
        templates.push(("[project] branch", t));
    }
    for lane in &p.lanes {
        if let Some(t) = &lane.branch {
            if lane.repo.is_none() {
                bail!(
                    "lane {}: branch is only for a lane with its own repo; the others share the tree's branch",
                    lane.name
                );
            }
            templates.push(("a lane's branch", t));
        }
    }
    let mut uses_key = false;
    for (what, t) in templates {
        let parts = parse(t).map_err(|why| anyhow!("{what} {t:?}: {why}"))?;
        uses_key |= parts.contains(&Part::Field(Field::Key));
    }
    if let Some(pattern) = &project.branch_key {
        let re = Regex::new(pattern).map_err(|e| {
            anyhow!("[project] branch_key {pattern:?} is not a regular expression: {e}")
        })?;
        let named = re.capture_names().any(|n| n == Some("key"));
        if !named && re.captures_len() != 2 {
            bail!(
                "[project] branch_key {pattern:?} needs exactly one capture group, or one named `key`"
            );
        }
    } else if uses_key {
        bail!("a branch template uses {{key}}, which needs [project] branch_key");
    }
    if let Some(kind) = &project.branch_type
        && sanitize(kind).is_empty()
    {
        bail!("[project] branch_type {kind:?} has no character a branch name may hold");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = r#"
version = 1
source = { kind = "manual" }

[project]
name = "demo"
repo = "https://example.invalid/demo.git"
space = "Demo"
"#;

    const LANE: &str = r#"
[[lanes]]
name = "app"
path = "."
"#;

    fn pipeline(project_extra: &str, lanes: &str) -> Result<Pipeline> {
        let text = format!("{BASE}{project_extra}\n{lanes}");
        Pipeline::parse(&text)
    }

    fn templated(extra: &str) -> Pipeline {
        pipeline(
            &format!(
                "branch = \"{{type}}/{{key}}-{{slug}}\"\nbranch_key = '\\b([A-Z]+-[0-9]+)\\b'\n{extra}"
            ),
            LANE,
        )
        .unwrap()
    }

    fn snapshot(title: &str, labels: &[&str]) -> SourceSnapshot {
        SourceSnapshot {
            kind: "manual".into(),
            identity: "x".into(),
            number: Some(7),
            title: title.into(),
            body: String::new(),
            url: None,
            labels: labels.iter().map(|&l| l.to_owned()).collect(),
            taken_at_ms: 0,
            pull_requests: Vec::new(),
            taken_by: None,
        }
    }

    #[test]
    fn no_template_is_the_default_name() {
        let p = pipeline("", LANE).unwrap();
        for title in [
            "Escape leaves the field, not the set!",
            "???",
            "",
            &"word ".repeat(30),
        ] {
            assert_eq!(
                tree(&p, &snapshot(title, &[])).unwrap(),
                branch_name(7, title)
            );
        }
    }

    #[test]
    fn the_key_leads_and_leaves_the_slug() {
        let p = templated("branch_type = \"feat\"");
        for title in [
            "ABC-123 Fix the thing",
            "Fix ABC-123 the thing",
            "Fix the thing ABC-123",
        ] {
            assert_eq!(
                tree(&p, &snapshot(title, &[])).unwrap(),
                "feat/ABC-123-fix-the-thing",
                "{title}"
            );
        }
        // Nothing but the key: the hyphen before the empty slug goes.
        assert_eq!(tree(&p, &snapshot("ABC-9", &[])).unwrap(), "feat/ABC-9");
    }

    #[test]
    fn a_type_label_wins_over_the_default() {
        let p = templated("branch_type = \"feat\"");
        let s = snapshot("ABC-1 Crash on start", &["bug", "type:fix"]);
        assert_eq!(tree(&p, &s).unwrap(), "fix/ABC-1-crash-on-start");
        let none = templated("");
        let e = tree(&none, &snapshot("ABC-1 Crash", &[])).unwrap_err();
        assert!(e.to_string().contains("{type} needs"), "{e}");
        assert_eq!(
            tree(&none, &snapshot("ABC-1 Crash", &["type: chore"])).unwrap(),
            "chore/ABC-1-crash"
        );
    }

    #[test]
    fn no_key_refuses_with_no_fallback() {
        let p = templated("branch_type = \"feat\"");
        let e = tree(&p, &snapshot("Fix the thing", &[])).unwrap_err();
        let text = e.to_string();
        assert!(
            text.starts_with("branch template \"{type}/{key}-{slug}\""),
            "{text}"
        );
        assert!(text.contains("no key matches"), "{text}");
        assert!(text.contains("\"Fix the thing\""), "{text}");
    }

    #[test]
    fn a_named_key_group_is_the_key() {
        let p = pipeline(
            "branch = \"{key}-{slug}\"\nbranch_key = '\\[(?P<key>[A-Z]+-[0-9]+)\\]( x)?'",
            LANE,
        )
        .unwrap();
        assert_eq!(
            tree(&p, &snapshot("[AB-4] Do it", &[])).unwrap(),
            "AB-4-do-it"
        );
    }

    #[test]
    fn an_unset_named_key_group_is_refused_not_replaced_by_another() {
        let p = pipeline(
            "branch = \"{key}-{slug}\"\nbranch_key = '(?P<key>[A-Z]+-[0-9]+)?|(\\w+)'",
            LANE,
        )
        .unwrap();
        let text = format!("{:#}", tree(&p, &snapshot("fix thing", &[])).unwrap_err());
        assert!(text.contains("matched without its key group"), "{text}");
    }

    #[test]
    fn labels_are_sanitised() {
        let p = templated("");
        let s = snapshot("ABC-1 Go", &["type:.big new/thing"]);
        assert_eq!(tree(&p, &s).unwrap(), "big-new-thing/ABC-1-go");
        let s = snapshot("ABC-1 Go", &["type:???"]);
        let e = tree(&p, &s).unwrap_err();
        assert!(e.to_string().contains("no character"), "{e}");
        assert_eq!(sanitize("..-a b..c."), "a-b..c");
    }

    #[test]
    fn ref_names_follow_git() {
        for (name, rule) in [
            ("", "empty"),
            ("@", "`@`"),
            ("-x", "starts with `-`"),
            ("/x", "starts or ends"),
            ("x/", "starts or ends"),
            ("a//b", "`//`"),
            ("a.", "ends with `.`"),
            ("a..b", "`..`"),
            ("a@{b", "`@{`"),
            ("a b", "' '"),
            ("a~b", "'~'"),
            ("a^b", "'^'"),
            ("a:b", "':'"),
            ("a?b", "'?'"),
            ("a*b", "'*'"),
            ("a[b", "'['"),
            ("a\\b", "'\\\\'"),
            ("a\u{7}b", "'\\u{7}'"),
            ("a/.b", "starts with `.`"),
            ("a.lock/b", "`.lock`"),
        ] {
            let e = check_ref_name(name).unwrap_err();
            assert!(e.contains(rule), "{name:?}: {e}");
        }
        check_ref_name("feat/ABC-1-go").unwrap();
        check_ref_name("dispatch/3").unwrap();
    }

    #[test]
    fn a_lane_with_its_own_repo_takes_its_own_template() {
        let lanes = r#"
[[lanes]]
name = "app"
path = "."

[[lanes]]
name = "infra"
path = "infra"
repo = "https://example.invalid/infra.git"
branch = "{type}/{number}"

[[lanes]]
name = "docs"
path = "docs"
repo = "https://example.invalid/docs.git"
"#;
        let p = pipeline(
            "branch = \"{type}/{key}-{slug}\"\nbranch_key = '([A-Z]+-[0-9]+)'\nbranch_type = \"feat\"",
            lanes,
        )
        .unwrap();
        let s = snapshot("ABC-1 Go", &[]);
        let names = names(&p, &s).unwrap();
        assert_eq!(
            names,
            vec![
                (None, "feat/ABC-1-go".to_owned()),
                (Some("infra"), "feat/7".to_owned()),
                (Some("docs"), "feat/ABC-1-go".to_owned()),
            ]
        );
        assert_eq!(lane_branch(&p, &p.lanes[0], &s).unwrap(), "feat/ABC-1-go");
    }

    #[test]
    fn parse_refuses_what_cannot_name_a_branch() {
        let refused = |extra: &str, lanes: &str, why: &str| {
            let e = pipeline(extra, lanes).unwrap_err();
            assert!(format!("{e:#}").contains(why), "{extra}: {e:#}");
        };
        refused("branch = \"{kind}/{slug}\"", LANE, "unknown field {kind}");
        refused("branch = \"{type/{slug}\"", LANE, "unknown field");
        refused("branch = \"x-{slug\"", LANE, "a `{` with no `}`");
        refused("branch = \"x}-{slug}\"", LANE, "a `}` with no `{`");
        refused("branch = \"{key}\"", LANE, "needs [project] branch_key");
        refused(
            "branch = \"{key}\"\nbranch_key = '([A-Z'",
            LANE,
            "is not a regular expression",
        );
        refused(
            "branch = \"{key}\"\nbranch_key = '[A-Z]+-[0-9]+'",
            LANE,
            "exactly one capture group",
        );
        refused(
            "branch = \"{key}\"\nbranch_key = '([A-Z]+)-([0-9]+)'",
            LANE,
            "exactly one capture group",
        );
        refused(
            "",
            "[[lanes]]\nname = \"app\"\npath = \".\"\nbranch = \"x/{slug}\"",
            "only for a lane with its own repo",
        );
        let pr = BASE.replace("{ kind = \"manual\" }", "{ kind = \"pull-request\" }");
        let e = Pipeline::parse(&format!("{pr}branch_type = \"feat\"\n{LANE}")).unwrap_err();
        assert!(format!("{e:#}").contains("pull-request pipeline"), "{e:#}");
    }
}
