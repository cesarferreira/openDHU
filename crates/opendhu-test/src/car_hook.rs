//! Reads the template an app reports through its debug-only openDHU hook.
//!
//! Android Auto draws the car UI where neither accessibility nor logcat can read it, so an app we
//! own can publish what it showed. The contract is a content provider at
//! `content://<package>.opendhu/template` with one row and one `json` column, holding the template
//! as JSON. The shape is the app's own; openDHU only matches the strings in it.

use opendhu_core::{Error, Result};

/// Every string value in the hook's JSON (titles, row text), from `adb shell content query` output.
pub fn strings(content_query_output: &str) -> Result<Vec<String>> {
    let json = content_query_output
        .lines()
        .find_map(|line| line.split_once(" json=").map(|(_, json)| json))
        .ok_or_else(|| {
            Error::InvalidArgs(format!(
                "the app has no openDHU template hook (expected a content provider returning a \
                 `json` column); content query said: {}",
                content_query_output.trim()
            ))
        })?;
    let value: serde_json::Value = serde_json::from_str(json.trim()).map_err(|e| {
        Error::InvalidArgs(format!("the app's template hook returned bad JSON: {e}"))
    })?;
    let mut out = Vec::new();
    collect(&value, &mut out);
    Ok(out)
}

fn collect(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::String(s) if !s.trim().is_empty() => out.push(s.clone()),
        serde_json::Value::Array(items) => items.iter().for_each(|v| collect(v, out)),
        serde_json::Value::Object(map) => map.values().for_each(|v| collect(v, out)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_nested_strings_and_skips_flags() {
        let out = r#"Row: 0 json={"type":"ListTemplate","title":"openDHU Demo","rows":[{"title":"Browse places","clickable":true},{"title":"Saved place: Harbour Cafe","clickable":true}]}"#;
        assert_eq!(
            strings(out).unwrap(),
            [
                "ListTemplate",
                "openDHU Demo",
                "Browse places",
                "Saved place: Harbour Cafe"
            ]
        );
    }

    #[test]
    fn a_missing_provider_is_explained() {
        let error = strings("No result found.\n").unwrap_err();
        assert!(
            error.message().contains("no openDHU template hook"),
            "{}",
            error.message()
        );
        assert!(strings("Row: 0 json={oops").is_err());
    }
}
