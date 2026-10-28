//! Android Auto (Gearhead) on the phone or emulator: detecting it and installing it on request.
//! Nothing here runs implicitly. A missing app is reported with a hint, and `aa setup gearhead`
//! installs one the user supplies, or one fetched by apkeep when asked, pinned and checksummed.

use std::path::{Path, PathBuf};
use std::time::Duration;

use opendhu_core::{Error, Result};

use crate::device::{adb, command};
use crate::doctor::{Check, Level, check};

pub(crate) const PACKAGE: &str = "com.google.android.projection.gearhead";

pub(crate) const MISSING_HINT: &str = "Android Auto (com.google.android.projection.gearhead) is not installed on this device; \
     use an emulator image with Google Play, or run `aa setup gearhead --apk <file>`";

fn lists_package(output: &str) -> bool {
    output
        .lines()
        .any(|l| l.trim() == format!("package:{PACKAGE}"))
}

/// Whether Android Auto is installed. An adb failure is an error, not "missing".
pub(crate) fn is_installed(program: &Path, serial: &str) -> Result<bool> {
    adb(
        program,
        serial,
        &["shell", "pm", "list", "packages", PACKAGE],
    )
    .map(|o| lists_package(&o))
}

/// The installed version, such as `17.9.664044-release`, when adb can tell.
fn version(program: &Path, serial: &str) -> Option<String> {
    adb(program, serial, &["shell", "dumpsys", "package", PACKAGE])
        .ok()?
        .lines()
        .find_map(|l| l.trim().strip_prefix("versionName=").map(str::to_string))
}

/// A doctor line for one device. `missing` is the level to report when the app is absent.
pub(crate) fn check_device(program: &Path, serial: &str, missing: Level) -> Check {
    match is_installed(program, serial) {
        Ok(true) => check(
            Level::Ok,
            "android auto",
            match version(program, serial) {
                Some(v) => format!("{serial}: installed ({v})"),
                None => format!("{serial}: installed"),
            },
        ),
        Ok(false) => check(missing, "android auto", format!("{serial}: {MISSING_HINT}")),
        Err(e) => check(Level::Warn, "android auto", format!("{serial}: {e}")),
    }
}

/// The APK files an `--apk` argument stands for: a single file, or a directory of split APKs.
/// `.xapk` and `.apks` bundles are unzipped into `scratch` first.
fn apk_files(source: &Path, scratch: &Path) -> Result<Vec<PathBuf>> {
    let extension = source
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let dir = if source.is_dir() {
        source.to_path_buf()
    } else if extension == "xapk" || extension == "apks" {
        let to = scratch.join("bundle");
        std::fs::create_dir_all(&to).map_err(|e| Error::Other(e.to_string()))?;
        let (src, dst) = (to_str(source)?, to_str(&to)?);
        command(
            Path::new("unzip"),
            &["-qo", src, "*.apk", "-d", dst],
            Duration::from_secs(120),
        )
        .map_err(|e| Error::InvalidArgs(format!("could not unzip {}: {e}", source.display())))?;
        to
    } else if source.is_file() {
        return Ok(vec![source.to_path_buf()]);
    } else {
        return Err(Error::InvalidArgs(format!(
            "{} does not exist",
            source.display()
        )));
    };
    let mut found = Vec::new();
    collect_apks(&dir, &mut found);
    found.sort();
    if found.is_empty() {
        return Err(Error::InvalidArgs(format!(
            "no .apk files found in {}",
            source.display()
        )));
    }
    Ok(found)
}

fn collect_apks(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for path in entries.flatten().map(|e| e.path()) {
        if path.is_dir() {
            collect_apks(&path, out);
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("apk"))
        {
            out.push(path);
        }
    }
}

fn to_str(path: &Path) -> Result<&str> {
    path.to_str()
        .ok_or_else(|| Error::InvalidArgs("path is not UTF-8".into()))
}

/// Install Android Auto from `source`; split APKs go through `install-multiple`.
pub(crate) fn install(
    program: &Path,
    serial: &str,
    source: &Path,
    scratch: &Path,
) -> Result<usize> {
    let files = apk_files(source, scratch)?;
    let mut args = vec![
        if files.len() == 1 {
            "install"
        } else {
            "install-multiple"
        },
        "-r",
    ];
    for file in &files {
        args.push(to_str(file)?);
    }
    // Installs of a large APK outlast the 30 second default of `adb`.
    let mut scoped = vec!["-s", serial];
    scoped.extend(args);
    command(program, &scoped, Duration::from_secs(300))?;
    Ok(files.len())
}

fn sha256(file: &Path) -> Result<String> {
    let path = to_str(file)?;
    let output = command(Path::new("sha256sum"), &[path], Duration::from_secs(60))
        .or_else(|_| {
            command(
                Path::new("shasum"),
                &["-a", "256", path],
                Duration::from_secs(60),
            )
        })
        .map_err(|_| {
            Error::BackendUnavailable("sha256sum or shasum is needed to verify the download".into())
        })?;
    Ok(output
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase())
}

/// Fetch a pinned Android Auto version with apkeep into `dir` and verify it against `expected`.
/// Returns the downloaded file. The source is apkeep's APKPure, which is not Google; the checksum is
/// what makes the result trustworthy, so it is required.
pub(crate) fn download(version: &str, expected: &str, dir: &Path) -> Result<PathBuf> {
    // APKPure names versions like `17.9.664044-release`, and apkeep exits cleanly with no file for a
    // version it does not know, so accept the short form.
    let version = if version.ends_with("-release") {
        version.to_string()
    } else {
        format!("{version}-release")
    };
    let app = format!("{PACKAGE}@{version}");
    command(
        Path::new("apkeep"),
        &["-a", &app, "-d", "apk-pure", to_str(dir)?],
        Duration::from_secs(600),
    )
    .map_err(|e| {
        Error::BackendUnavailable(format!(
            "apkeep failed ({e}); install it with `cargo install apkeep` or pass --apk <file>"
        ))
    })?;
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir)
        .map_err(|e| Error::Other(e.to_string()))?
        .flatten()
    {
        let path = entry.path();
        if path.extension().is_some_and(|e| {
            ["apk", "xapk", "apks"]
                .iter()
                .any(|x| e.eq_ignore_ascii_case(x))
        }) {
            found.push(path);
        }
    }
    if found.is_empty() {
        return Err(Error::Other(format!(
            "apkeep downloaded nothing for {version}; list what APKPure has with `apkeep -a {PACKAGE} -d apk-pure -l`"
        )));
    }
    let [file] = found.as_slice() else {
        return Err(Error::Other(format!(
            "expected apkeep to download one file into {}, found {}",
            dir.display(),
            found.len()
        )));
    };
    let actual = sha256(file)?;
    if actual != expected.to_ascii_lowercase() {
        return Err(Error::Other(format!(
            "SHA-256 of the download is {actual}, not {expected}; refusing to install it"
        )));
    }
    Ok(file.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_exact_package_only() {
        assert!(lists_package(&format!("package:{PACKAGE}\n")));
        assert!(lists_package(&format!(
            "package:other\r\npackage:{PACKAGE}\r\n"
        )));
        assert!(!lists_package(""));
        assert!(!lists_package(&format!("package:{PACKAGE}.extra\n")));
    }

    #[test]
    fn apk_files_rejects_a_missing_source() {
        let dir = std::env::temp_dir();
        assert!(apk_files(&dir.join("opendhu-no-such.apk"), &dir).is_err());
    }
}
