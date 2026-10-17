//! Project-local names select existing suite YAML; filters select tests before setup begins.
use opendhu_core::{Error, Result};
use opendhu_test::TestFile;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Project {
    suites: BTreeMap<String, PathBuf>,
}

#[derive(Clone, serde::Serialize)]
pub(crate) struct NamedSuite {
    pub name: String,
    pub path: PathBuf,
}

fn project() -> Result<Option<(PathBuf, Project)>> {
    let cwd = std::env::current_dir().map_err(|e| Error::Other(e.to_string()))?;
    for base in cwd.ancestors() {
        let index = base.join("opendhu.yaml");
        let text = match std::fs::read_to_string(&index) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                return Err(Error::InvalidArgs(format!(
                    "cannot read {}: {e}",
                    index.display()
                )));
            }
        };
        let project: Project = serde_norway::from_str(&text)
            .map_err(|e| Error::InvalidArgs(format!("invalid {}: {e}", index.display())))?;
        for (name, path) in &project.suites {
            if name.is_empty()
                || name == "."
                || name == ".."
                || !name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
            {
                return Err(Error::InvalidArgs(format!(
                    "invalid suite name {name:?} in {}",
                    index.display()
                )));
            }
            if path.as_os_str().is_empty() {
                return Err(Error::InvalidArgs(format!(
                    "named suite {name:?} has an empty path in {}",
                    index.display()
                )));
            }
        }
        return Ok(Some((index, project)));
    }
    Ok(None)
}

fn missing_registry() -> Error {
    Error::InvalidArgs("no opendhu.yaml found in this directory or its parents; create one with a `suites` mapping, or pass a YAML path directly".into())
}

pub(crate) fn registered() -> Result<Vec<NamedSuite>> {
    let (index, project) = project()?.ok_or_else(missing_registry)?;
    let base = index.parent().expect("registry parent");
    Ok(project
        .suites
        .into_iter()
        .map(|(name, path)| NamedSuite {
            name,
            path: base.join(path),
        })
        .collect())
}

pub(crate) fn is_all(paths: &[PathBuf]) -> bool {
    paths == [PathBuf::from("all")] && !paths[0].exists()
}

pub(crate) fn resolve(paths: Vec<PathBuf>) -> Result<Vec<PathBuf>> {
    if paths.len() != 1 || paths[0].exists() || paths[0].components().count() != 1 {
        return Ok(paths);
    }
    let name = paths[0].to_string_lossy();
    let Some((index, project)) = project()? else {
        return if paths[0].extension().is_none() {
            Err(missing_registry())
        } else {
            Ok(paths)
        };
    };
    if let Some(path) = project.suites.get(name.as_ref()) {
        return Ok(vec![index.parent().expect("registry parent").join(path)]);
    }
    Err(Error::InvalidArgs(format!(
        "unknown suite {name:?} in {}; available names: {}. Use `aa test --list` to list suites",
        index.display(),
        project
            .suites
            .keys()
            .cloned()
            .collect::<Vec<_>>()
            .join(", "),
    )))
}

pub(crate) fn matching(
    tests: Vec<(String, TestFile)>,
    query: Option<&str>,
) -> Result<Vec<(String, TestFile)>> {
    let Some(query) = query else {
        return Ok(tests);
    };
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return Err(Error::InvalidArgs("--filter must not be empty".into()));
    }
    let selected: Vec<_> = tests
        .into_iter()
        .filter(|(file, test)| {
            test.name.to_lowercase().contains(&needle)
                || Path::new(file)
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().to_lowercase().contains(&needle))
        })
        .collect();
    Ok(selected)
}

pub(crate) fn filter(
    tests: Vec<(String, TestFile)>,
    query: Option<&str>,
) -> Result<Vec<(String, TestFile)>> {
    let selected = matching(tests, query)?;
    if selected.is_empty()
        && let Some(query) = query
    {
        return Err(Error::InvalidArgs(format!(
            "--filter {query:?} matched no tests"
        )));
    }
    Ok(selected)
}
