//! The agent skill that ships inside the binary: `aa skill install` writes it where agents look.

use std::path::{Path, PathBuf};

use opendhu_core::{Error, Result};

/// Embedded so the installed `aa` is enough; the repo checkout is not needed.
pub const SKILL: &str = include_str!("../skill/SKILL.md");
const NAME: &str = "opendhu";

/// Default destinations: Claude Code and the shared `~/.agents` convention, or the project's copies.
pub fn default_roots(project: bool, cwd: &Path) -> Result<Vec<PathBuf>> {
    let base = if project {
        cwd.to_path_buf()
    } else {
        PathBuf::from(std::env::var_os("HOME").ok_or_else(|| {
            Error::InvalidArgs("HOME is not set; pass --dir to choose where to install".into())
        })?)
    };
    Ok(vec![
        base.join(".claude").join("skills"),
        base.join(".agents").join("skills"),
    ])
}

/// Write `<root>/opendhu/SKILL.md` into each root. Returns what was written.
/// An existing different file is only replaced with `force`; an identical one is left alone.
pub fn install(roots: &[PathBuf], force: bool) -> Result<Vec<(PathBuf, &'static str)>> {
    let mut done = Vec::new();
    for root in roots {
        let dir = root.join(NAME);
        let file = dir.join("SKILL.md");
        let state = match std::fs::read_to_string(&file) {
            Ok(old) if old == SKILL => "unchanged",
            Ok(_) if !force => {
                return Err(Error::InvalidArgs(format!(
                    "{} exists and differs; pass --force to replace it",
                    file.display()
                )));
            }
            Ok(_) => "updated",
            Err(_) => "installed",
        };
        if state != "unchanged" {
            // A symlinked skill dir (`make install-skill`) would write into the repo; replace the link.
            if dir
                .symlink_metadata()
                .is_ok_and(|m| m.file_type().is_symlink())
            {
                if !force {
                    return Err(Error::InvalidArgs(format!(
                        "{} is a symlink; pass --force to replace it",
                        dir.display()
                    )));
                }
                std::fs::remove_file(&dir).map_err(|e| io(&dir, e))?;
            }
            std::fs::create_dir_all(&dir).map_err(|e| io(&dir, e))?;
            std::fs::write(&file, SKILL).map_err(|e| io(&file, e))?;
        }
        done.push((file, state));
    }
    Ok(done)
}

fn io(p: &Path, e: std::io::Error) -> Error {
    Error::Other(format!("{}: {e}", p.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("opendhu-skill-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn skill_has_frontmatter_name() {
        assert!(SKILL.starts_with("---\nname: opendhu\n"));
    }

    #[test]
    fn installs_then_is_idempotent_and_guards_edits() {
        let root = tmp("a");
        let r = vec![root.clone()];
        assert_eq!(install(&r, false).unwrap()[0].1, "installed");
        assert_eq!(install(&r, false).unwrap()[0].1, "unchanged");
        let f = root.join("opendhu/SKILL.md");
        std::fs::write(&f, "mine").unwrap();
        assert!(install(&r, false).is_err());
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "mine");
        assert_eq!(install(&r, true).unwrap()[0].1, "updated");
        assert_eq!(std::fs::read_to_string(&f).unwrap(), SKILL);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_dir_is_not_written_through() {
        let root = tmp("b");
        let target = tmp("b-target");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        std::os::unix::fs::symlink(&target, root.join("opendhu")).unwrap();
        assert!(install(std::slice::from_ref(&root), false).is_err());
        assert!(!target.join("SKILL.md").exists());
        install(std::slice::from_ref(&root), true).unwrap();
        assert!(!target.join("SKILL.md").exists());
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&target);
    }
}
