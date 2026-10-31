//! `dispatch check`: one pipeline file parsed and validated as a take
//! would, every problem listed, and nothing written. The rules that need
//! to know where the file lives (its name, the project it is stored
//! under) are here; the rest are [`Pipeline::check_with`]'s.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

use crate::pipeline::{Pipeline, Report, Source};
use crate::store::DataDir;

/// One file to check, and the project it is stored under when it was
/// named by project rather than by path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub path: PathBuf,
    pub project: Option<String>,
}

/// The files `arg` names. A path (it holds a `/` or ends in `.toml`) is
/// one file, read where it is. A project name is its pipeline and its
/// pull-request pipeline in the data directory, whichever exist.
pub fn resolve(data: &DataDir, arg: &str) -> Result<Vec<Target>> {
    let toml = Path::new(arg)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("toml"));
    if arg.contains('/') || toml {
        return Ok(vec![Target {
            path: PathBuf::from(arg),
            project: None,
        }]);
    }
    let targets: Vec<Target> = [data.pipeline(arg), data.pr_pipeline(arg)]
        .into_iter()
        .filter(|p| p.exists())
        .map(|path| Target {
            path,
            project: Some(arg.to_owned()),
        })
        .collect();
    if targets.is_empty() {
        bail!(
            "no pipeline for {arg} at {} or {}",
            data.pipeline(arg).display(),
            data.pr_pipeline(arg).display()
        );
    }
    Ok(targets)
}

/// Reads the file at `target` and returns its report; a file that cannot
/// be read is the report's one error.
#[must_use]
pub fn check_path(target: &Target) -> Report {
    match std::fs::read_to_string(&target.path) {
        Ok(text) => check_file(target, &text),
        Err(e) => Report {
            errors: vec![format!("cannot read {}: {e}", target.path.display())],
            warnings: Vec::new(),
        },
    }
}

/// Returns the report for `text`, the contents of `target`: the
/// pipeline's own rules, then the ones a take of it adds for its name and
/// its project. A take
/// also refuses a worktree root its host's shells would split
/// (`shell_unsafe`); that depends on the host, so it is not checked here.
#[must_use]
pub fn check_file(target: &Target, text: &str) -> Report {
    Pipeline::check_with(text, |p| {
        let mut errors = Vec::new();
        let file = target
            .path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let by_name = file.ends_with(".pr.toml");
        let pull_request = p.source == Source::PullRequest;
        if by_name && !pull_request {
            errors.push(format!(
                "{file} is a pull-request pipeline by its name but its source is kind = {:?}",
                kind(&p.source)
            ));
        } else if !by_name && pull_request {
            errors.push(format!(
                "{file} is not a pull-request pipeline by its name (that is <project>.pr.toml) but its source is kind = \"pull-request\""
            ));
        }
        if let Some(project) = &target.project
            && p.project.name != *project
        {
            errors.push(format!(
                "the pipeline file names project {:?}, not {project:?}",
                p.project.name
            ));
        }
        errors
    })
}

/// A source's `kind` as the file spells it.
fn kind(source: &Source) -> &'static str {
    match source {
        Source::Github { .. } => "github",
        Source::TaskFile { .. } => "task-file",
        Source::Manual => "manual",
        Source::PullRequest => "pull-request",
    }
}

/// `report` for the file at `path`, as `dispatch check` prints it: the
/// path, each problem indented under it, and a count; `<path>: ok` when
/// there is nothing to say.
#[must_use]
pub fn render(path: &Path, report: &Report) -> String {
    if report.errors.is_empty() && report.warnings.is_empty() {
        return format!("{}: ok", path.display());
    }
    let mut out = vec![path.display().to_string()];
    for (label, items) in [("error", &report.errors), ("warning", &report.warnings)] {
        for item in items {
            let mut rows = item.lines();
            if let Some(first) = rows.next() {
                out.push(format!("  {label}: {first}"));
            }
            out.extend(rows.map(|row| format!("    {row}")));
        }
    }
    let warnings = count(report.warnings.len(), "warning");
    out.push(if report.errors.is_empty() {
        format!("ok, {warnings}")
    } else {
        format!("{}, {warnings}", count(report.errors.len(), "error"))
    });
    out.join("\n")
}

fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GITHUB: &str = "kind = \"github\"\nrepo = \"o/r\"\nlabel = \"dispatch\"\n";

    /// A small file whose `[source]` table is `source` and whose project
    /// is `project`.
    fn file(project: &str, source: &str) -> String {
        format!(
            "version = 1\n\n[project]\nname = \"{project}\"\nroot = \"/r\"\n\
             space = \"Dispatch\"\n\n[source]\n{source}\n[[lanes]]\nname = \"repo\"\n\
             path = \".\"\n\n[[stages]]\nname = \"inspect\"\n\
             gate = {{ kind = \"human\", decision = \"inspect\" }}\n"
        )
    }

    fn at(path: &str) -> Target {
        Target {
            path: PathBuf::from(path),
            project: None,
        }
    }

    #[test]
    fn the_file_name_and_the_source_kind_agree() {
        let github = file("Orchard", GITHUB);
        let pr = file("Orchard", "kind = \"pull-request\"\n");
        let manual = file("Orchard", "kind = \"manual\"\n");
        assert_eq!(check_file(&at("Orchard.toml"), &github), Report::default());
        assert_eq!(check_file(&at("Orchard.pr.toml"), &pr), Report::default());
        assert_eq!(
            check_file(&at("Orchard.held.toml"), &github),
            Report::default()
        );
        assert_eq!(check_file(&at("Orchard.toml"), &manual), Report::default());
        assert_eq!(
            check_file(&at("p/Orchard.pr.toml"), &github).errors,
            [
                "Orchard.pr.toml is a pull-request pipeline by its name but its source is kind = \"github\""
            ]
        );
        assert_eq!(
            check_file(&at("Orchard.toml"), &pr).errors,
            [
                "Orchard.toml is not a pull-request pipeline by its name (that is <project>.pr.toml) but its source is kind = \"pull-request\""
            ]
        );
    }

    #[test]
    fn the_project_name_is_checked_only_when_named_by_project() {
        let text = file("Pear", GITHUB);
        let by_project = Target {
            path: PathBuf::from("/d/pipelines/Orchard.toml"),
            project: Some("Orchard".into()),
        };
        assert_eq!(
            check_file(&by_project, &text).errors,
            ["the pipeline file names project \"Pear\", not \"Orchard\""]
        );
        assert_eq!(
            check_file(&at("/d/pipelines/Orchard.toml"), &text),
            Report::default()
        );
    }

    #[test]
    fn a_project_names_the_files_it_has() {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::new(dir.path());
        let e = resolve(&data, "Orchard").unwrap_err().to_string();
        assert!(
            e.contains(&data.pipeline("Orchard").display().to_string()),
            "{e}"
        );
        assert!(
            e.contains(&data.pr_pipeline("Orchard").display().to_string()),
            "{e}"
        );
        std::fs::create_dir_all(dir.path().join("pipelines")).unwrap();
        std::fs::write(data.pipeline("Orchard"), "").unwrap();
        let paths = |ts: Vec<Target>| ts.into_iter().map(|t| t.path).collect::<Vec<_>>();
        assert_eq!(
            paths(resolve(&data, "Orchard").unwrap()),
            [data.pipeline("Orchard")]
        );
        std::fs::write(data.pr_pipeline("Orchard"), "").unwrap();
        let targets = resolve(&data, "Orchard").unwrap();
        assert!(
            targets
                .iter()
                .all(|t| t.project.as_deref() == Some("Orchard"))
        );
        assert_eq!(
            paths(targets),
            [data.pipeline("Orchard"), data.pr_pipeline("Orchard")]
        );
        // A path is read where it is, whatever the data directory holds.
        assert_eq!(resolve(&data, "x/Orchard").unwrap(), [at("x/Orchard")]);
        assert_eq!(
            resolve(&data, "Orchard.toml").unwrap(),
            [at("Orchard.toml")]
        );
    }

    /// The examples are copied to other machines as they are, so each
    /// one is checked the way `dispatch check` checks it: no error and
    /// no key the parser drops. A new example is picked up by the glob.
    #[test]
    fn every_example_in_pipelines_examples_checks_clean() {
        let dir = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../pipelines/examples"
        ));
        let mut checked = 0;
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|e| e != "toml") {
                continue;
            }
            let report = check_path(&Target {
                path: path.clone(),
                project: None,
            });
            assert_eq!(report, Report::default(), "{}", path.display());
            checked += 1;
        }
        assert!(checked >= 3, "only {checked} examples found");
    }

    #[test]
    fn a_report_is_rendered_with_its_counts() {
        let report = Report {
            errors: vec!["one".into(), "two\nsecond line".into()],
            warnings: vec!["stages[0].gates is not a key the parser keeps".into()],
        };
        assert_eq!(
            render(Path::new("pipelines/P.toml"), &report),
            "pipelines/P.toml\n  error: one\n  error: two\n    second line\n  \
             warning: stages[0].gates is not a key the parser keeps\n2 errors, 1 warning"
        );
        assert_eq!(
            render(Path::new("P.toml"), &Report::default()),
            "P.toml: ok"
        );
        let warned = Report {
            errors: Vec::new(),
            warnings: vec!["x is not a key the parser keeps".into()],
        };
        assert_eq!(
            render(Path::new("P.toml"), &warned),
            "P.toml\n  warning: x is not a key the parser keeps\nok, 1 warning"
        );
    }
}
