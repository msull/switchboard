//! `.env` files: `KEY=value` lines with comments, an optional `export`
//! prefix, and single or double quotes (double quotes honor `\n` and
//! `\"`). Later lines win. Only ever read on the user's opt-in.

/// Parse the text of a `.env` file into ordered pairs.
#[must_use]
pub fn parse(text: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").map_or(line, str::trim_start);
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }
        let value = unquote(value.trim());
        out.retain(|(k, _)| k != key);
        out.push((key.to_owned(), value));
    }
    out
}

fn unquote(value: &str) -> String {
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        let inner = &value[1..value.len() - 1];
        let mut out = String::with_capacity(inner.len());
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some(other) => out.push(other),
                    None => out.push('\\'),
                }
            } else {
                out.push(c);
            }
        }
        return out;
    }
    if value.len() >= 2 && value.starts_with('\'') && value.ends_with('\'') {
        return value[1..value.len() - 1].to_owned();
    }
    // Unquoted: a trailing comment ends the value.
    value
        .split_once(" #")
        .map_or(value, |(v, _)| v)
        .trim()
        .to_owned()
}

/// The variable names a `.env.example` declares, in order.
#[must_use]
pub fn names(text: &str) -> Vec<String> {
    parse(text).into_iter().map(|(k, _)| k).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_usual_shapes() {
        let text = "# comment\nA=1\nexport B = two\nC=\"with \\\"quotes\\\" and\\nnewline\"\nD='single # not comment'\nE=plain # comment\nBAD KEY=x\nA=override\n";
        assert_eq!(
            parse(text),
            vec![
                ("B".into(), "two".into()),
                ("C".into(), "with \"quotes\" and\nnewline".into()),
                ("D".into(), "single # not comment".into()),
                ("E".into(), "plain".into()),
                ("A".into(), "override".into()),
            ]
        );
        assert_eq!(names("X=\nY=1\n"), vec!["X", "Y"]);
    }
}
