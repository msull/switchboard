//! Prompt templates: `{name}` and `{a.b}` fields filled from a map, the
//! rest left as written. Commands never go through this; their values
//! travel as environment variables.

use std::collections::BTreeMap;

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
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_')
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
}
