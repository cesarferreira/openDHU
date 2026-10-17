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

pub(crate) fn resolve(paths: Vec<PathBuf>) -> Result<Vec<PathBuf>> {
    if paths.len() != 1 || paths[0].exists() || paths[0].components().count() != 1 {
        return Ok(paths);
    }
    let cwd = std::env::current_dir().map_err(|e| Error::Other(e.to_string()))?;
    let name = paths[0].to_string_lossy();
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
        if let Some(path) = project.suites.get(name.as_ref()) {
            if path.as_os_str().is_empty() {
                return Err(Error::InvalidArgs(format!(
                    "named suite {name:?} has an empty path"
                )));
            }
            return Ok(vec![base.join(path)]);
        }
        return Err(Error::InvalidArgs(format!(
            "unknown suite {name:?} in {}; available names: {}",
            index.display(),
            project
                .suites
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    Ok(paths) // The ordinary path diagnostic still applies outside a configured project.
}

pub(crate) fn filter(
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
    if selected.is_empty() {
        return Err(Error::InvalidArgs(format!(
            "--filter {query:?} matched no tests"
        )));
    }
    Ok(selected)
}
