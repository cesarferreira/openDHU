//! Project YAML configuration. Only the CLI reads it: the daemon receives explicit, absolute
//! values in each request, so the API stays self-describing. Relative paths are based on the file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use opendhu_core::{Error, Result};
use serde::Deserialize;

use crate::BackendArg;
use crate::test_cmd::ReportFormat;

/// Names a YAML config file instead of searching for `opendhu.yaml` in the current tree.
pub const ENV_VAR: &str = "OPENDHU_CONFIG";

#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Raw {
    #[serde(default)]
    suites: BTreeMap<String, PathBuf>,
    #[serde(default)]
    dhu: RawDhu,
    #[serde(default)]
    session: RawSession,
    #[serde(default)]
    test: RawTest,
    #[serde(default)]
    daemon: RawDaemon,
}

#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct RawDhu {
    path: Option<PathBuf>,
    /// The DHU's own `.ini` file (what `aa session start --config` takes).
    ini: Option<PathBuf>,
    headless: Option<bool>,
}

#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct RawSession {
    backend: Option<BackendArg>,
    serial: Option<String>,
    reverse: Option<Vec<u16>>,
}

#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct RawTest {
    artifacts_dir: Option<PathBuf>,
    report: Option<Vec<ReportFormat>>,
    skip_unsupported: Option<bool>,
}

#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct RawDaemon {
    port: Option<u16>,
}

/// Parsed settings with every path already absolute.
#[derive(Debug, Default, PartialEq)]
pub struct Config {
    /// The loaded YAML file, if any.
    pub sources: Vec<PathBuf>,
    pub suites: BTreeMap<String, PathBuf>,
    pub dhu_path: Option<PathBuf>,
    pub dhu_ini: Option<PathBuf>,
    pub headless: bool,
    pub backend: Option<BackendArg>,
    pub serial: Option<String>,
    pub reverse: Vec<u16>,
    pub artifacts_dir: Option<PathBuf>,
    pub report: Vec<ReportFormat>,
    pub skip_unsupported: bool,
    pub port: Option<u16>,
}

/// Where the lookup inputs come from. Injected so tests never read the real machine.
pub struct Locator {
    pub opendhu_config: Option<PathBuf>,
}

impl Locator {
    pub fn from_process() -> Self {
        Self {
            opendhu_config: std::env::var_os(ENV_VAR)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from),
        }
    }
}

/// Load the nearest project file, or the explicit file. No project file gives the defaults.
pub fn load(loc: &Locator, cwd: &Path) -> Result<Config> {
    if let Some(path) = &loc.opendhu_config {
        let path = if path.is_absolute() {
            path.clone()
        } else {
            cwd.join(path)
        };
        return read(&path, true).map(Option::unwrap);
    }
    for base in cwd.ancestors() {
        let path = base.join("opendhu.yaml");
        if let Some(config) = read(&path, false)? {
            return Ok(config);
        }
    }
    Ok(Config::default())
}

fn read(path: &Path, must_exist: bool) -> Result<Option<Config>> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && !must_exist => {
            return Ok(None);
        }
        Err(e) => {
            return Err(Error::InvalidArgs(format!(
                "cannot read config file {}: {e}",
                path.display()
            )));
        }
    };
    parse(&text, path).map(Some)
}

fn parse(text: &str, path: &Path) -> Result<Config> {
    let raw: Raw = serde_norway::from_str(text)
        .map_err(|e| Error::InvalidArgs(format!("invalid config file {}:\n{e}", path.display())))?;
    let base = path.parent().unwrap_or(Path::new("/"));
    let abs = |p: PathBuf| if p.is_absolute() { p } else { base.join(p) };
    if raw.session.reverse.as_ref().is_some_and(|r| r.contains(&0)) {
        return Err(Error::InvalidArgs(format!(
            "invalid config file {}: session.reverse ports must be 1-65535",
            path.display()
        )));
    }
    for (name, suite) in &raw.suites {
        if name.is_empty()
            || name == "."
            || name == ".."
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
            || suite.as_os_str().is_empty()
        {
            return Err(Error::InvalidArgs(format!(
                "invalid suite {name:?} in {}",
                path.display()
            )));
        }
    }
    Ok(Config {
        sources: vec![path.to_path_buf()],
        suites: raw
            .suites
            .into_iter()
            .map(|(name, p)| (name, abs(p)))
            .collect(),
        dhu_path: raw.dhu.path.map(abs),
        dhu_ini: raw.dhu.ini.map(abs),
        headless: raw.dhu.headless.unwrap_or(false),
        backend: raw.session.backend,
        serial: raw.session.serial,
        reverse: raw.session.reverse.unwrap_or_default(),
        artifacts_dir: raw.test.artifacts_dir.map(abs),
        report: raw.test.report.unwrap_or_default(),
        skip_unsupported: raw.test.skip_unsupported.unwrap_or(false),
        port: raw.daemon.port,
    })
}

/// Flag first, then the config file's value.
pub fn first<T>(flag: Option<T>, file: Option<T>) -> Option<T> {
    flag.or(file)
}

/// The DHU path to send to the daemon. A non-empty `OPENDHU_DHU_PATH` outranks the file, and the
/// daemon already reads that variable, so in that case nothing is sent.
pub fn dhu_path(flag: Option<PathBuf>, file: Option<PathBuf>, env_set: bool) -> Option<PathBuf> {
    flag.or(if env_set { None } else { file })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Config {
        parse(s, Path::new("/home/u/project/opendhu.yaml")).unwrap()
    }

    #[test]
    fn empty_file_gives_defaults_but_records_its_source() {
        let c = p("suites: {}\n");
        assert_eq!(
            c.sources,
            vec![PathBuf::from("/home/u/project/opendhu.yaml")]
        );
        assert!(c.dhu_path.is_none() && c.reverse.is_empty() && !c.headless);
    }

    #[test]
    fn relative_paths_resolve_against_the_file_directory() {
        let c = p("dhu:\n  path: bin/dhu\n  ini: /abs/x.ini\ntest:\n  artifacts_dir: out\n");
        assert_eq!(c.dhu_path.unwrap(), Path::new("/home/u/project/bin/dhu"));
        assert_eq!(c.dhu_ini.unwrap(), Path::new("/abs/x.ini"));
        assert_eq!(c.artifacts_dir.unwrap(), Path::new("/home/u/project/out"));
    }

    #[test]
    fn full_file_parses() {
        let c = p(
            "suites: {sample: e2e/sample.yaml}\ndhu:\n  headless: true\nsession:\n  serial: ABC\n  reverse: [10101, 8080]\ntest:\n  report: [json, junit, html]\n  skip_unsupported: true\ndaemon:\n  port: 9000\n",
        );
        assert!(c.headless && c.skip_unsupported);
        assert_eq!(c.serial.as_deref(), Some("ABC"));
        assert_eq!(c.reverse, vec![10101, 8080]);
        assert_eq!(
            c.report,
            vec![ReportFormat::Json, ReportFormat::Junit, ReportFormat::Html]
        );
        assert_eq!(c.port, Some(9000));
        assert_eq!(
            c.suites["sample"],
            Path::new("/home/u/project/e2e/sample.yaml")
        );
    }

    #[test]
    fn unknown_keys_and_sections_are_errors_that_name_the_file() {
        for bad in ["dhu:\n  pth: x", "nope:\n  a: 1", "top: 1"] {
            let e = parse(bad, Path::new("/c.yaml")).unwrap_err();
            assert!(matches!(e, Error::InvalidArgs(_)), "{bad}");
            assert!(e.to_string().contains("/c.yaml"), "{e}");
        }
    }

    #[test]
    fn wrong_types_and_bad_ports_are_errors() {
        for bad in [
            "session:\n  reverse: '10101'",
            "session:\n  reverse: [0]",
            "session:\n  reverse: [70000]",
            "test:\n  report: [csv]",
        ] {
            assert!(parse(bad, Path::new("/c.yaml")).is_err(), "{bad}");
        }
    }

    #[test]
    fn precedence_helpers() {
        assert_eq!(first(Some(1), Some(2)), Some(1));
        assert_eq!(first(None, Some(2)), Some(2));
        let (f, c) = (Some(PathBuf::from("/flag")), Some(PathBuf::from("/file")));
        assert_eq!(dhu_path(f.clone(), c.clone(), true), f);
        assert_eq!(dhu_path(None, c.clone(), false), c);
        assert_eq!(dhu_path(None, c, true), None, "env outranks the file");
    }

    #[test]
    fn missing_project_file_gives_defaults_and_named_files_must_exist() {
        let dir = std::env::temp_dir().join(format!("opendhu-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let none = Locator {
            opendhu_config: None,
        };
        assert_eq!(load(&none, &dir).unwrap(), Config::default());
        let named = Locator {
            opendhu_config: Some(dir.join("missing.yaml")),
        };
        assert!(load(&named, &dir).is_err());
        std::fs::write(dir.join("opendhu.yaml"), "session:\n  serial: Z\n").unwrap();
        assert_eq!(load(&none, &dir).unwrap().serial.as_deref(), Some("Z"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn nearest_project_file_supplies_session_and_suite_settings() {
        let dir = std::env::temp_dir().join(format!(
            "opendhu-project-cfg-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let project = dir.join("project");
        let nested = project.join("app/src");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(
            project.join("opendhu.yaml"),
            "suites: {robotaxi: e2e/robotaxi.yaml}\ndhu:\n  ini: head-unit/nissan_leaf.ini\nsession:\n  backend: dhu\n  serial: FAKE0001\n",
        )
        .unwrap();
        let locator = Locator {
            opendhu_config: None,
        };

        let cfg = load(&locator, &nested).unwrap();
        assert_eq!(cfg.backend, Some(BackendArg::Dhu));
        assert_eq!(cfg.serial.as_deref(), Some("FAKE0001"));
        assert_eq!(
            cfg.dhu_ini.as_deref(),
            Some(project.join("head-unit/nissan_leaf.ini").as_path())
        );
        assert_eq!(cfg.suites["robotaxi"], project.join("e2e/robotaxi.yaml"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn explicit_config_skips_project_discovery() {
        let dir = std::env::temp_dir().join(format!(
            "opendhu-explicit-cfg-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let project = dir.join("project");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join("opendhu.yaml"), "session:\n  backend: dhu\n").unwrap();
        std::fs::write(
            project.join("chosen.yaml"),
            "session:\n  serial: EXPLICIT\n",
        )
        .unwrap();
        let locator = Locator {
            opendhu_config: Some(PathBuf::from("chosen.yaml")),
        };

        let cfg = load(&locator, &project).unwrap();
        assert_eq!(cfg.backend, None);
        assert_eq!(cfg.serial.as_deref(), Some("EXPLICIT"));
        assert_eq!(cfg.sources, vec![project.join("chosen.yaml")]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn closest_project_file_wins() {
        let dir = std::env::temp_dir().join(format!(
            "opendhu-nearest-cfg-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let child = dir.join("child");
        std::fs::create_dir_all(&child).unwrap();
        std::fs::write(dir.join("opendhu.yaml"), "dhu:\n  ini: parent.ini\n").unwrap();
        std::fs::write(child.join("opendhu.yaml"), "dhu:\n  ini: child.ini\n").unwrap();
        let locator = Locator {
            opendhu_config: None,
        };

        let cfg = load(&locator, &child).unwrap();
        assert_eq!(cfg.dhu_ini, Some(child.join("child.ini")));
        assert_eq!(cfg.sources, vec![child.join("opendhu.yaml")]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
