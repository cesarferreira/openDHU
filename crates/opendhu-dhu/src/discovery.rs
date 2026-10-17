use std::path::{Path, PathBuf};

const DHU_REL: &str = "extras/google/auto/desktop-head-unit";

/// Where the lookup inputs come from. Injected so tests never read the real machine.
pub struct Env {
    pub explicit: Option<PathBuf>,
    pub opendhu_dhu_path: Option<PathBuf>,
    pub android_home: Option<PathBuf>,
    pub android_sdk_root: Option<PathBuf>,
    pub home: Option<PathBuf>,
}

impl Env {
    pub fn from_process(explicit: Option<PathBuf>) -> Self {
        let var = |k: &str| {
            std::env::var_os(k)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        Self {
            explicit,
            opendhu_dhu_path: var("OPENDHU_DHU_PATH"),
            android_home: var("ANDROID_HOME"),
            android_sdk_root: var("ANDROID_SDK_ROOT"),
            home: var("HOME"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Explicit,
    EnvVar,
    AndroidHome,
    AndroidSdkRoot,
    KnownLocation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub path: PathBuf,
    pub source: Source,
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    p.metadata()
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// Resolve the DHU executable. Precedence follows PRD 11.1: explicit config, env var,
/// SDK env vars, then known platform locations. An explicit path that does not exist is
/// an error to the caller, not a silent fall-through.
pub fn find_dhu(env: &Env) -> Result<Found, Vec<PathBuf>> {
    let mut tried = Vec::new();
    // An explicitly requested path is a statement of intent: if it is unusable, say so rather
    // than quietly launching a different DHU.
    for (requested, source) in [
        (&env.explicit, Source::Explicit),
        (&env.opendhu_dhu_path, Source::EnvVar),
    ] {
        if let Some(path) = requested {
            return if is_executable(path) {
                Ok(Found {
                    path: path.clone(),
                    source,
                })
            } else {
                Err(vec![path.clone()])
            };
        }
    }
    let mut candidates: Vec<(PathBuf, Source)> = Vec::new();
    if let Some(r) = &env.android_home {
        candidates.push((r.join(DHU_REL), Source::AndroidHome));
    }
    if let Some(r) = &env.android_sdk_root {
        candidates.push((r.join(DHU_REL), Source::AndroidSdkRoot));
    }
    if let Some(h) = &env.home {
        for rel in ["Library/Android/sdk", "Android/Sdk"] {
            candidates.push((h.join(rel).join(DHU_REL), Source::KnownLocation));
        }
    }
    for (path, source) in candidates {
        if is_executable(&path) {
            return Ok(Found { path, source });
        }
        tried.push(path);
    }
    Err(tried)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn fake_dhu(root: &Path) -> PathBuf {
        let p = root.join(DHU_REL);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    fn empty_env() -> Env {
        Env {
            explicit: None,
            opendhu_dhu_path: None,
            android_home: None,
            android_sdk_root: None,
            home: None,
        }
    }

    #[test]
    fn explicit_beats_env_and_sdk() {
        let a = tempdir("a");
        let b = tempdir("b");
        let explicit = fake_dhu(&a);
        let sdk = fake_dhu(&b);
        let _ = sdk;
        let env = Env {
            explicit: Some(explicit.clone()),
            android_home: Some(b),
            ..empty_env()
        };
        let f = find_dhu(&env).unwrap();
        assert_eq!(
            f,
            Found {
                path: explicit,
                source: Source::Explicit
            }
        );
    }

    #[test]
    fn falls_back_to_android_home_then_known_location() {
        let sdk = tempdir("sdk");
        let p = fake_dhu(&sdk);
        let env = Env {
            android_home: Some(sdk),
            ..empty_env()
        };
        assert_eq!(find_dhu(&env).unwrap().source, Source::AndroidHome);
        assert_eq!(find_dhu(&env).unwrap().path, p);

        let home = tempdir("home");
        fake_dhu(&home.join("Library/Android/sdk"));
        let env = Env {
            home: Some(home),
            ..empty_env()
        };
        assert_eq!(find_dhu(&env).unwrap().source, Source::KnownLocation);
    }

    #[test]
    fn non_executable_is_rejected_and_reported() {
        let sdk = tempdir("noexec");
        let p = fake_dhu(&sdk);
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        let env = Env {
            android_home: Some(sdk),
            ..empty_env()
        };
        assert_eq!(find_dhu(&env).unwrap_err(), vec![p]);
    }

    #[test]
    fn unusable_explicit_path_never_falls_through_to_the_sdk() {
        let sdk = tempdir("fallthrough");
        fake_dhu(&sdk);
        let env = Env {
            explicit: Some("/nope/dhu".into()),
            android_home: Some(sdk.clone()),
            ..empty_env()
        };
        assert_eq!(
            find_dhu(&env).unwrap_err(),
            vec![PathBuf::from("/nope/dhu")]
        );
        let env = Env {
            opendhu_dhu_path: Some("/nope/dhu".into()),
            android_home: Some(sdk),
            ..empty_env()
        };
        assert_eq!(
            find_dhu(&env).unwrap_err(),
            vec![PathBuf::from("/nope/dhu")]
        );
    }

    #[test]
    fn env_var_beats_sdk_locations() {
        let a = tempdir("envvar");
        let b = tempdir("envsdk");
        let chosen = fake_dhu(&a);
        fake_dhu(&b);
        let env = Env {
            opendhu_dhu_path: Some(chosen.clone()),
            android_home: Some(b),
            ..empty_env()
        };
        assert_eq!(
            find_dhu(&env).unwrap(),
            Found {
                path: chosen,
                source: Source::EnvVar
            }
        );
    }

    #[test]
    fn missing_everywhere_lists_tried_paths() {
        let env = Env {
            explicit: Some("/nope/dhu".into()),
            ..empty_env()
        };
        assert_eq!(
            find_dhu(&env).unwrap_err(),
            vec![PathBuf::from("/nope/dhu")]
        );
    }

    fn tempdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "opendhu-test-{}-{}-{}",
            tag,
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }
}
