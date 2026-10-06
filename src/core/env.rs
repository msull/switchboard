//! Environment resolution for new sessions: global variables, then the
//! project's `.env` files (only when the project opted in), then the
//! project's own variables. Secret values are looked up by account name;
//! the records never hold them.
//!
//! Environment sets are resolved separately, by `resolve_sets`, only for
//! `switchboard-env`: their values never go into a pane's environment.

use std::collections::HashSet;

pub use switchboard_control::valid_set_name;

use crate::core::model::{AwsMethod, EnvSet, EnvVar, ProjectEnv, ProjectId};

/// Where a secret lives; decides its Keychain account name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretScope {
    Global,
    Project(ProjectId),
    /// An environment set, by name.
    Set(String),
}

impl SecretScope {
    /// `global/NAME`, `project/<id>/NAME` or `set/<name>/NAME`.
    #[must_use]
    pub fn account(&self, name: &str) -> String {
        match self {
            Self::Global => format!("global/{name}"),
            Self::Project(id) => format!("project/{}/{name}", id.0),
            Self::Set(set) => format!("set/{set}/{name}"),
        }
    }
}

/// A variable name is `[A-Z_][A-Z0-9_]*`.
#[must_use]
pub fn valid_var_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_uppercase() || c == '_')
        && chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// SHA-256 of a launch token, in hex: what a record keeps of it.
#[must_use]
pub fn token_hash(token: &str) -> String {
    use sha2::{Digest, Sha256};
    use std::fmt::Write;
    Sha256::digest(token.as_bytes())
        .iter()
        .fold(String::new(), |mut hex, b| {
            let _ = write!(hex, "{b:02x}");
            hex
        })
}

/// What a session's granted sets resolve to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SetsResolved {
    /// Name/value pairs for the child; missing secrets are left out.
    pub pairs: Vec<(String, String)>,
    /// Each secret with no stored value, as `<set>/VAR`.
    pub missing: Vec<String>,
    /// The AWS method of the one granted set that gives one.
    pub aws: Option<AwsMethod>,
}

/// Resolve the project's grants, then the session's, each in listed
/// order with repeats dropped; a later variable replaces an earlier one
/// of the same name. An unknown set is an error naming it, and so are
/// two sets that both give an AWS method: picking one could deploy to
/// the wrong account.
pub fn resolve_sets(
    sets: &[EnvSet],
    project: &[String],
    session: &[String],
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Result<SetsResolved, String> {
    let mut names: Vec<&str> = Vec::new();
    for name in project.iter().chain(session) {
        if !names.contains(&name.as_str()) {
            names.push(name);
        }
    }
    let mut out = SetsResolved::default();
    let mut aws_from: Option<&str> = None;
    for name in names {
        let set = sets
            .iter()
            .find(|s| s.name == name)
            .ok_or_else(|| format!("no environment set named {name}"))?;
        if let Some(method) = &set.aws {
            if let Some(first) = aws_from {
                return Err(format!(
                    "environment sets {first} and {name} both give AWS credentials"
                ));
            }
            aws_from = Some(name);
            out.aws = Some(method.clone());
        }
        let scope = SecretScope::Set(set.name.clone());
        for var in &set.vars {
            out.pairs.retain(|(n, _)| *n != var.name);
            out.missing
                .retain(|m| m.rsplit_once('/').is_none_or(|(_, v)| v != var.name));
            if var.secret {
                match lookup(&scope.account(&var.name)) {
                    Some(value) => out.pairs.push((var.name.clone(), value)),
                    None => out.missing.push(format!("{name}/{}", var.name)),
                }
            } else {
                out.pairs.push((var.name.clone(), var.value.clone()));
            }
        }
    }
    Ok(out)
}

/// Which layer supplied a value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Global,
    DotEnv(String),
    Project,
}

impl Source {
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Global => "global".into(),
            Self::DotEnv(file) => file.clone(),
            Self::Project => "project".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedVar {
    pub name: String,
    /// `None`: a secret whose value is not in the store.
    pub value: Option<String>,
    pub secret: bool,
    pub source: Source,
}

/// The environment a session in the project gets.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Resolved {
    pub vars: Vec<ResolvedVar>,
    /// Names `.env.example` declares that nothing defines.
    pub example_missing: Vec<String>,
}

impl Resolved {
    /// Name/value pairs to inject; missing secrets are left out.
    #[must_use]
    pub fn pairs(&self) -> Vec<(String, String)> {
        self.vars
            .iter()
            .filter_map(|v| v.value.clone().map(|value| (v.name.clone(), value)))
            .collect()
    }

    /// Secret names with no stored value.
    #[must_use]
    pub fn missing(&self) -> Vec<&str> {
        self.vars
            .iter()
            .filter(|v| v.value.is_none())
            .map(|v| v.name.as_str())
            .collect()
    }
}

/// Resolve the layers. `dotenv` is the parsed content of each opted-in
/// file, in the order they were listed; `lookup` reads a secret by
/// account name.
#[must_use]
pub fn resolve(
    global: &[EnvVar],
    project_id: ProjectId,
    project: &ProjectEnv,
    dotenv: &[(String, Vec<(String, String)>)],
    example_names: &[String],
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Resolved {
    let mut vars: Vec<ResolvedVar> = Vec::new();
    let mut put = |var: ResolvedVar| {
        vars.retain(|v| v.name != var.name);
        vars.push(var);
    };
    for var in global {
        put(layer_var(var, &SecretScope::Global, Source::Global, lookup));
    }
    for (file, pairs) in dotenv {
        for (name, value) in pairs {
            put(ResolvedVar {
                name: name.clone(),
                value: Some(value.clone()),
                secret: false,
                source: Source::DotEnv(file.clone()),
            });
        }
    }
    for var in &project.vars {
        put(layer_var(
            var,
            &SecretScope::Project(project_id),
            Source::Project,
            lookup,
        ));
    }
    let defined: HashSet<&str> = vars.iter().map(|v| v.name.as_str()).collect();
    let example_missing = example_names
        .iter()
        .filter(|n| !defined.contains(n.as_str()))
        .cloned()
        .collect();
    Resolved {
        vars,
        example_missing,
    }
}

fn layer_var(
    var: &EnvVar,
    scope: &SecretScope,
    source: Source,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> ResolvedVar {
    let value = if var.secret {
        lookup(&scope.account(&var.name))
    } else {
        Some(var.value.clone())
    };
    ResolvedVar {
        name: var.name.clone(),
        value,
        secret: var.secret,
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn var(name: &str, value: &str, secret: bool) -> EnvVar {
        EnvVar {
            name: name.into(),
            value: value.into(),
            secret,
        }
    }

    #[test]
    fn layers_override_in_order_and_secrets_come_from_the_store() {
        let pid = ProjectId::new();
        let global = vec![var("A", "g", false), var("TOKEN", "", true)];
        let project = ProjectEnv {
            vars: vec![var("A", "p", false), var("KEY", "", true)],
            load_dotenv: true,
            dotenv_files: vec![".env".into()],
        };
        let dotenv = vec![(
            ".env".to_string(),
            vec![
                ("A".to_string(), "d".to_string()),
                ("B".to_string(), "b".to_string()),
            ],
        )];
        let lookup = move |account: &str| match account {
            "global/TOKEN" => Some("t".to_string()),
            _ => None,
        };
        let r = resolve(
            &global,
            pid,
            &project,
            &dotenv,
            &["A".to_string(), "C".to_string()],
            &lookup,
        );
        let by_name = |n: &str| r.vars.iter().find(|v| v.name == n).unwrap().clone();
        assert_eq!(by_name("A").value.as_deref(), Some("p"));
        assert_eq!(by_name("A").source, Source::Project);
        assert_eq!(by_name("B").source, Source::DotEnv(".env".into()));
        assert_eq!(by_name("TOKEN").value.as_deref(), Some("t"));
        assert!(by_name("TOKEN").secret);
        assert_eq!(by_name("KEY").value, None);
        assert_eq!(r.missing(), vec!["KEY"]);
        assert_eq!(r.example_missing, vec!["C".to_string()]);
        assert_eq!(r.pairs().len(), 3);
        assert_eq!(
            SecretScope::Project(pid).account("KEY"),
            format!("project/{}/KEY", pid.0)
        );
    }
}
