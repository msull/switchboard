//! Prompt templates: `{name}` and `{a.b}` fields filled from a map, the
//! rest left as written. Commands never go through this; their values
//! travel as environment variables.

use std::collections::{BTreeMap, BTreeSet};

/// The values a prompt may name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Vars(pub BTreeMap<String, String>);

impl Vars {
    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) -> &mut Self {
        self.0.insert(key.into(), value.into());
        self
    }

    /// Fill `template`. A field with no value stays as written, so a
    /// missing input is visible in the prompt rather than silently blank.
    #[must_use]
    pub fn render(&self, template: &str) -> String {
        let mut out = String::with_capacity(template.len());
        let mut rest = template;
        while let Some(start) = rest.find('{') {
            out.push_str(&rest[..start]);
            let after = &rest[start + 1..];
            match after.find('}') {
                Some(end) if is_field(&after[..end]) => {
                    let key = &after[..end];
                    if let Some(value) = self.0.get(key) {
                        out.push_str(value);
                    } else {
                        out.push('{');
                        out.push_str(key);
                        out.push('}');
                    }
                    rest = &after[end + 1..];
                }
                _ => {
                    out.push('{');
                    rest = after;
                }
            }
        }
        out.push_str(rest);
        out
    }
}

fn is_field(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Each input `text` names, as `{inputs.X}` (no stage) or
/// `{inputs.S.X}` (stage `S`), leaving out `{inputs.S.commit}`, which
/// is a commit and not a file.
#[must_use]
pub fn input_names(text: &str) -> BTreeSet<(Option<String>, String)> {
    input_fields(text)
        .filter_map(|field| match field.split_once('.') {
            None => Some((None, field.to_owned())),
            Some((stage, name)) if !name.contains('.') && name != "commit" => {
                Some((Some(stage.to_owned()), name.to_owned()))
            }
            Some(_) => None,
        })
        .collect()
}

/// The stage of each `{inputs.S.commit}` `text` names.
#[must_use]
pub fn commit_stages(text: &str) -> BTreeSet<String> {
    input_fields(text)
        .filter_map(|field| field.strip_suffix(".commit"))
        .filter(|stage| !stage.is_empty() && !stage.contains('.'))
        .map(str::to_owned)
        .collect()
}

/// What follows `inputs.` in each well-formed `{inputs.…}` field.
fn input_fields(text: &str) -> impl Iterator<Item = &str> {
    let mut rest = text;
    std::iter::from_fn(move || {
        loop {
            let start = rest.find("{inputs.")?;
            let after = &rest[start + "{inputs.".len()..];
            let end = after.find('}')?;
            let field = &after[..end];
            rest = &after[end..];
            if is_field(field) {
                return Some(field);
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_fill_and_the_rest_stays() {
        let mut vars = Vars::default();
        vars.set("issue.number", "7").set("notes", "/n.md");
        assert_eq!(
            vars.render(
                "Issue #{issue.number}: write to {notes}; keep {unknown} and { spaced } and {}"
            ),
            "Issue #7: write to /n.md; keep {unknown} and { spaced } and {}"
        );
        assert_eq!(vars.render("no fields"), "no fields");
        assert_eq!(vars.render("{notes"), "{notes");
    }

    #[test]
    fn input_names_finds_both_forms_and_skips_commits() {
        let names = input_names(
            "Use {inputs.personas} and {inputs.try-setup.personas}; at {inputs.deploy.commit}; {inputs.} {issue.title} {inputs.bad name}",
        );
        assert_eq!(
            names,
            BTreeSet::from([
                (None, "personas".to_owned()),
                (Some("try-setup".to_owned()), "personas".to_owned()),
            ])
        );
    }

    #[test]
    fn commit_stages_finds_only_commits() {
        assert_eq!(
            commit_stages(
                "Use {inputs.personas} and {inputs.try-setup.personas}; at {inputs.deploy.commit}; {inputs.} {issue.title} {inputs.bad name}",
            ),
            BTreeSet::from(["deploy".to_owned()])
        );
    }
}
