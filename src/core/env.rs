//! Environment resolution for new sessions: global variables, then the
//! project's `.env` files (only when the project opted in), then the
//! project's own variables. Secret values are looked up by account name;
//! the records never hold them.

use std::collections::HashSet;

use crate::core::model::{EnvVar, ProjectEnv, ProjectId};

/// Where a secret lives; decides its Keychain account name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretScope {
    Global,
    Project(ProjectId),
}

impl SecretScope {
    /// `global/NAME` or `project/<id>/NAME`.
    #[must_use]
    pub fn account(self, name: &str) -> String {
        match self {
            Self::Global => format!("global/{name}"),
            Self::Project(id) => format!("project/{}/{name}", id.0),
        }
    }
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
        put(layer_var(var, SecretScope::Global, Source::Global, lookup));
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
            SecretScope::Project(project_id),
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
    scope: SecretScope,
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
