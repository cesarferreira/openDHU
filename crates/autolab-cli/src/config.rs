//! The optional user config file (TOML). Only the CLI reads it: the daemon keeps receiving explicit,
//! absolute values in each request, so the API stays self-describing.
//!
//! Precedence, highest first: command-line flag, environment variable, config file, built-in default.
//! A relative path inside the file is relative to the file's own directory, not the current one.

use std::path::{Path, PathBuf};

use autolab_core::{Error, Result};
use serde::Deserialize;

use crate::BackendArg;
use crate::test_cmd::ReportFormat;

/// Names the config file, overriding the default location. A missing file here is an error.
pub const ENV_VAR: &str = "AUTOLAB_CONFIG";

#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Raw {
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
    /// The DHU's own `.ini` file (what `aa start --config` takes).
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
    /// The file these came from, if one was loaded.
    pub source: Option<PathBuf>,
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
    pub autolab_config: Option<PathBuf>,
    pub xdg_config_home: Option<PathBuf>,
    pub home: Option<PathBuf>,
}

impl Locator {
    pub fn from_process() -> Self {
        let var = |k: &str| {
            std::env::var_os(k)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        Self {
            autolab_config: var(ENV_VAR),
            xdg_config_home: var("XDG_CONFIG_HOME"),
            home: var("HOME"),
        }
    }

    /// `(path, must_exist)`. A file the user named must exist; the default location may be absent.
    fn path(&self) -> Option<(PathBuf, bool)> {
        if let Some(p) = &self.autolab_config {
            return Some((p.clone(), true));
        }
        let base = self
            .xdg_config_home
            .clone()
            .filter(|p| p.is_absolute())
            .or_else(|| self.home.as_ref().map(|h| h.join(".config")))?;
        Some((base.join("autolab").join("config.toml"), false))
    }
}

/// Load the config file if there is one. No file is not an error and gives the defaults.
pub fn load(loc: &Locator, cwd: &Path) -> Result<Config> {
    let Some((path, must_exist)) = loc.path() else {
        return Ok(Config::default());
    };
    let path = if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && !must_exist => {
            return Ok(Config::default());
        }
        Err(e) => {
            return Err(Error::InvalidArgs(format!(
                "cannot read config file {}: {e}",
                path.display()
            )));
        }
    };
    parse(&text, &path)
}

pub fn parse(text: &str, path: &Path) -> Result<Config> {
    let raw: Raw = toml::from_str(text)
        .map_err(|e| Error::InvalidArgs(format!("invalid config file {}:\n{e}", path.display())))?;
    let base = path.parent().unwrap_or(Path::new("/"));
    let abs = |p: PathBuf| if p.is_absolute() { p } else { base.join(p) };
    if raw.session.reverse.as_ref().is_some_and(|r| r.contains(&0)) {
        return Err(Error::InvalidArgs(format!(
            "invalid config file {}: session.reverse ports must be 1-65535",
            path.display()
        )));
    }
    Ok(Config {
        source: Some(path.to_path_buf()),
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

/// The DHU path to send to the daemon. A non-empty `AUTOLAB_DHU_PATH` outranks the file, and the
/// daemon already reads that variable, so in that case nothing is sent.
pub fn dhu_path(flag: Option<PathBuf>, file: Option<PathBuf>, env_set: bool) -> Option<PathBuf> {
    flag.or(if env_set { None } else { file })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Config {
        parse(s, Path::new("/home/u/.config/autolab/config.toml")).unwrap()
    }

    #[test]
    fn empty_file_gives_defaults_but_records_its_source() {
        let c = p("");
        assert_eq!(
            c.source.as_deref(),
            Some(Path::new("/home/u/.config/autolab/config.toml"))
        );
        assert!(c.dhu_path.is_none() && c.reverse.is_empty() && !c.headless);
    }

    #[test]
    fn relative_paths_resolve_against_the_file_directory() {
        let c =
            p("[dhu]\npath = \"bin/dhu\"\nini = \"/abs/x.ini\"\n[test]\nartifacts_dir = \"out\"");
        assert_eq!(
            c.dhu_path.unwrap(),
            Path::new("/home/u/.config/autolab/bin/dhu")
        );
        assert_eq!(c.dhu_ini.unwrap(), Path::new("/abs/x.ini"));
        assert_eq!(
            c.artifacts_dir.unwrap(),
            Path::new("/home/u/.config/autolab/out")
        );
    }

    #[test]
    fn full_file_parses() {
        let c = p(
            "[dhu]\nheadless = true\n[session]\nserial = \"ABC\"\nreverse = [10101, 8080]\n[test]\nreport = [\"json\", \"junit\", \"html\"]\nskip_unsupported = true\n[daemon]\nport = 9000",
        );
        assert!(c.headless && c.skip_unsupported);
        assert_eq!(c.serial.as_deref(), Some("ABC"));
        assert_eq!(c.reverse, vec![10101, 8080]);
        assert_eq!(
            c.report,
            vec![ReportFormat::Json, ReportFormat::Junit, ReportFormat::Html]
        );
        assert_eq!(c.port, Some(9000));
    }

    #[test]
    fn unknown_keys_and_sections_are_errors_that_name_the_file() {
        for bad in ["[dhu]\npth = \"x\"", "[nope]\na = 1", "top = 1"] {
            let e = parse(bad, Path::new("/c.toml")).unwrap_err();
            assert!(matches!(e, Error::InvalidArgs(_)), "{bad}");
            assert!(e.to_string().contains("/c.toml"), "{e}");
        }
    }

    #[test]
    fn wrong_types_and_bad_ports_are_errors() {
        for bad in [
            "[session]\nreverse = \"10101\"",
            "[session]\nreverse = [0]",
            "[session]\nreverse = [70000]",
            "[test]\nreport = [\"csv\"]",
        ] {
            assert!(parse(bad, Path::new("/c.toml")).is_err(), "{bad}");
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
    fn default_location_may_be_absent_but_a_named_file_may_not() {
        let dir = std::env::temp_dir().join(format!("autolab-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let none = Locator {
            autolab_config: None,
            xdg_config_home: Some(dir.clone()),
            home: None,
        };
        assert_eq!(load(&none, &dir).unwrap(), Config::default());
        let named = Locator {
            autolab_config: Some(dir.join("missing.toml")),
            xdg_config_home: None,
            home: None,
        };
        assert!(load(&named, &dir).is_err());
        let sub = dir.join("autolab");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("config.toml"), "[session]\nserial = \"Z\"").unwrap();
        assert_eq!(load(&none, &dir).unwrap().serial.as_deref(), Some("Z"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
