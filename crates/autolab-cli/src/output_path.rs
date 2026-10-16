//! Validation of user-supplied output paths (PRD section 21: validate file output paths).

use std::path::{Component, Path, PathBuf};

use autolab_core::{Error, Result};

/// Turn a user-supplied screenshot path into a clean absolute path, or explain why not.
/// Symlinks are not resolved: the target usually does not exist yet.
pub fn resolve_png(raw: &Path, cwd: &Path) -> Result<PathBuf> {
    let text = raw.to_string_lossy();
    if text.is_empty() {
        return Err(Error::InvalidArgs("output path is empty".into()));
    }
    if text.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(Error::InvalidArgs(format!(
            "output path {text:?} must not contain whitespace or control characters (the DHU console cannot take them safely)"
        )));
    }
    let is_png = raw
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("png"));
    if !is_png {
        return Err(Error::InvalidArgs(format!(
            "output path {text} must end in .png"
        )));
    }
    let abs = normalize(&if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        cwd.join(raw)
    });
    let parent = abs.parent().unwrap_or(Path::new("/"));
    if !parent.is_dir() {
        return Err(Error::InvalidArgs(format!(
            "directory {} does not exist",
            parent.display()
        )));
    }
    if abs.is_dir() {
        return Err(Error::InvalidArgs(format!(
            "{} is a directory, not a file",
            abs.display()
        )));
    }
    Ok(abs)
}

/// An absolute, lexically clean version of `raw`. The daemon's working directory is not the
/// caller's, so relative paths must be resolved here, before they are sent.
pub fn absolute(raw: &Path, cwd: &Path) -> PathBuf {
    normalize(&if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        cwd.join(raw)
    })
}

/// Collapse `.` and `..` lexically.
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cwd() -> PathBuf {
        std::env::temp_dir()
    }

    #[test]
    fn relative_paths_become_absolute_and_clean() {
        let got = resolve_png(Path::new("./a/../shot.png"), &cwd()).unwrap();
        assert_eq!(got, normalize(&cwd().join("shot.png")));
        assert!(got.is_absolute());
    }

    #[test]
    fn absolute_resolves_against_the_callers_directory() {
        let cwd = Path::new("/work/proj");
        assert_eq!(
            absolute(Path::new("cfg/a.ini"), cwd),
            Path::new("/work/proj/cfg/a.ini")
        );
        assert_eq!(
            absolute(Path::new("../x.ini"), cwd),
            Path::new("/work/x.ini")
        );
        assert_eq!(
            absolute(Path::new("/abs/y.ini"), cwd),
            Path::new("/abs/y.ini")
        );
    }

    #[test]
    fn extension_is_case_insensitive_but_required() {
        assert!(resolve_png(Path::new("a.PNG"), &cwd()).is_ok());
        assert!(matches!(
            resolve_png(Path::new("a.jpg"), &cwd()),
            Err(Error::InvalidArgs(_))
        ));
        assert!(matches!(
            resolve_png(Path::new("png"), &cwd()),
            Err(Error::InvalidArgs(_))
        ));
    }

    #[test]
    fn rejects_whitespace_empty_missing_dir_and_directories() {
        for bad in ["", "has space.png", "tab\t.png", "new\nline.png"] {
            assert!(
                matches!(
                    resolve_png(Path::new(bad), &cwd()),
                    Err(Error::InvalidArgs(_))
                ),
                "{bad:?}"
            );
        }
        let missing = resolve_png(Path::new("/definitely/not/here/x.png"), &cwd()).unwrap_err();
        assert!(missing.to_string().contains("does not exist"));
        let dir = cwd().join("looks-like-file.png");
        std::fs::create_dir_all(&dir).unwrap();
        let err = resolve_png(&dir, &cwd()).unwrap_err();
        assert!(err.to_string().contains("is a directory"));
        let _ = std::fs::remove_dir(&dir);
    }
}
