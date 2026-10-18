//! Resolve one parent per suite, retaining paths relative to their declaring YAML file.
use opendhu_core::{Error, Result};
use serde_norway::Value;
use std::path::{Path, PathBuf};

pub fn resolve(path: &Path) -> Result<Value> {
    resolve_inner(path, &mut Vec::new())
}

fn resolve_inner(path: &Path, stack: &mut Vec<PathBuf>) -> Result<Value> {
    let identity = std::fs::canonicalize(path)
        .map_err(|e| Error::InvalidArgs(format!("cannot read suite {}: {e}", path.display())))?;
    if stack.contains(&identity) {
        return Err(Error::InvalidArgs(format!(
            "suite extends cycle at {}",
            path.display()
        )));
    }
    if stack.len() >= 32 {
        return Err(Error::InvalidArgs("suite extends exceeds 32 files".into()));
    }
    stack.push(identity);
    let text = std::fs::read_to_string(path)
        .map_err(|e| Error::InvalidArgs(format!("cannot read suite {}: {e}", path.display())))?;
    let mut value: Value = serde_norway::from_str(&text)
        .map_err(|e| Error::InvalidArgs(format!("invalid suite {}: {e}", path.display())))?;
    let map = value
        .as_mapping_mut()
        .ok_or_else(|| Error::InvalidArgs(format!("suite {} must be a mapping", path.display())))?;
    let base = std::path::absolute(path).map_err(|e| Error::InvalidArgs(e.to_string()))?;
    let base = base.parent().expect("absolute file parent");
    let parent = map.remove(Value::String("extends".into()));
    let mut inherited = match parent {
        None => Value::Mapping(Default::default()),
        Some(Value::String(parent)) if !parent.trim().is_empty() => {
            resolve_inner(&base.join(parent), stack)?
        }
        Some(_) => {
            return Err(Error::InvalidArgs(
                "suite extends must be a nonempty file path".into(),
            ));
        }
    };
    // Normalize before merging, so inherited paths never drift into the child's directory.
    if let Some(apk) = value.get_mut("environment").and_then(|e| e.get_mut("apk")) {
        resolve_path(apk, base);
    }
    if let Some(dir) = value.get_mut("artifactsDir") {
        resolve_path(dir, base);
    }
    if let Some(tests) = value.get_mut("tests").and_then(Value::as_sequence_mut) {
        for pattern in tests {
            resolve_path(pattern, base);
        }
    }
    if let Some(services) = value.get_mut("services").and_then(Value::as_sequence_mut) {
        for service in services {
            if let Some(map) = service.as_mapping_mut() {
                let cwd = map
                    .entry(Value::String("cwd".into()))
                    .or_insert_with(|| Value::String(base.to_string_lossy().into_owned()));
                if cwd.is_null() || cwd.as_str() == Some("") {
                    *cwd = Value::String(base.to_string_lossy().into_owned());
                }
                resolve_path(cwd, base);
            }
        }
    }
    let target = inherited.as_mapping_mut().expect("resolved suite mapping");
    for (key, value) in value.as_mapping().expect("suite mapping") {
        if key.as_str() == Some("environment")
            && let (Some(parent), Some(child)) = (
                target.get_mut(key).and_then(Value::as_mapping_mut),
                value.as_mapping(),
            )
        {
            parent.extend(child.clone());
        } else {
            target.insert(key.clone(), value.clone());
        }
    }
    stack.pop();
    Ok(inherited)
}

fn resolve_path(value: &mut Value, base: &Path) {
    if let Some(text) = value.as_str()
        && !text.is_empty()
        && !Path::new(text).is_absolute()
    {
        *value = Value::String(base.join(text).to_string_lossy().into_owned());
    }
}
