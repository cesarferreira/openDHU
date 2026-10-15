//! JSON and JUnit reports plus the artifacts directory layout from PRD section 12.6.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::model::{Outcome, StepResult, TestResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Json,
    Junit,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendMeta {
    pub name: String,
    pub version: Option<String>,
    pub build: Option<String>,
    pub serial: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub tests: usize,
    pub passed: usize,
    pub failed: usize,
    pub errors: usize,
    pub skipped: usize,
}

impl Summary {
    pub fn of(tests: &[TestResult]) -> Self {
        let count = |o: Outcome| tests.iter().filter(|t| t.outcome == o).count();
        Self {
            tests: tests.len(),
            passed: count(Outcome::Passed),
            failed: count(Outcome::Failed),
            errors: count(Outcome::Error),
            skipped: count(Outcome::Skipped),
        }
    }
}

/// Only values that were actually observed. Everything the backend trait cannot tell us is an
/// explicit `null`, never a guess.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Metadata {
    pub autolab_version: String,
    pub started_at: String,
    pub started_at_unix: u64,
    pub duration_ms: u64,
    pub backend: Option<BackendMeta>,
    pub device_model: Option<String>,
    pub android_version: Option<String>,
    pub android_auto_version: Option<String>,
    pub display: Option<String>,
    pub summary: Summary,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunReport {
    pub metadata: Metadata,
    pub exit_code: u8,
    /// The run was stopped by Ctrl+C or SIGTERM, so some tests or steps did not run.
    pub interrupted: bool,
    pub tests: Vec<TestResult>,
}

/// One exit code for a whole run. Order is fixed so the result is deterministic: invalid input
/// first, then environment problems, then timeouts, then assertion failures.
pub fn exit_code_for(tests: &[TestResult]) -> u8 {
    const ORDER: [u8; 9] = [crate::runner::INTERRUPTED, 2, 3, 4, 5, 6, 7, 8, 1];
    let present = |c: u8| tests.iter().any(|t| t.exit_code == c);
    ORDER
        .into_iter()
        .find(|&c| present(c))
        .or_else(|| tests.iter().map(|t| t.exit_code).find(|&c| c != 0))
        .unwrap_or(0)
}

pub fn to_json(report: &RunReport) -> String {
    serde_json::to_string_pretty(report).expect("report serializes")
}

/// Plain-text log of one test, used for `logs/` and JUnit `system-out`.
pub fn step_log(t: &TestResult) -> String {
    let mut out = format!("Test: {}\nFile: {}\n\n", t.name, t.file);
    for s in &t.steps {
        let tag = match s.outcome {
            Outcome::Passed => "PASS ",
            Outcome::Failed => "FAIL ",
            Outcome::Error => "ERROR",
            Outcome::Skipped => "SKIP ",
        };
        let _ = writeln!(
            out,
            "{tag} {}  {}  ({} ms)",
            label(s),
            s.detail,
            s.duration_ms
        );
    }
    let _ = writeln!(out, "\nResult: {:?}  ({} ms)", t.outcome, t.duration_ms);
    if let Some(m) = &t.message {
        let _ = writeln!(out, "Reason: {m}");
    }
    out
}

fn label(s: &StepResult) -> String {
    if s.index == 0 {
        s.action.clone()
    } else {
        format!("{}. {}", s.index, s.action)
    }
}

/// Strip characters XML 1.0 cannot represent, then escape the five special ones.
pub fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\t' | '\n' | '\r' => out.push(c),
            c if (c as u32) < 0x20 => {}
            '\u{FFFE}' | '\u{FFFF}' => {}
            c if (0xD800..=0xDFFF).contains(&(c as u32)) => {}
            c => out.push(c),
        }
    }
    out
}

fn secs(ms: u64) -> String {
    format!("{}.{:03}", ms / 1000, ms % 1000)
}

pub fn to_junit(report: &RunReport) -> String {
    let m = &report.metadata;
    let cases: usize = report.tests.iter().map(|t| t.steps.len()).sum();
    let count = |o: Outcome| -> usize {
        report
            .tests
            .iter()
            .flat_map(|t| &t.steps)
            .filter(|s| s.outcome == o)
            .count()
    };
    let mut x = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    let _ = writeln!(
        x,
        "<testsuites name=\"autolab\" tests=\"{cases}\" failures=\"{}\" errors=\"{}\" skipped=\"{}\" time=\"{}\">",
        count(Outcome::Failed),
        count(Outcome::Error),
        count(Outcome::Skipped),
        secs(m.duration_ms)
    );
    for t in &report.tests {
        let n = |o: Outcome| t.steps.iter().filter(|s| s.outcome == o).count();
        let _ = writeln!(
            x,
            "  <testsuite name=\"{}\" tests=\"{}\" failures=\"{}\" errors=\"{}\" skipped=\"{}\" time=\"{}\" timestamp=\"{}\" file=\"{}\">",
            xml_escape(&t.name),
            t.steps.len(),
            n(Outcome::Failed),
            n(Outcome::Error),
            n(Outcome::Skipped),
            secs(t.duration_ms),
            xml_escape(&m.started_at),
            xml_escape(&t.file)
        );
        for s in &t.steps {
            let head = format!(
                "    <testcase classname=\"{}\" name=\"{}\" time=\"{}\"",
                xml_escape(&t.name),
                xml_escape(&label(s)),
                secs(s.duration_ms)
            );
            let detail = xml_escape(&s.detail);
            match s.outcome {
                Outcome::Passed => {
                    let _ = writeln!(x, "{head}/>");
                }
                Outcome::Failed => {
                    let kind = if s.exit_code == Some(6) {
                        "timeout"
                    } else {
                        "assertion"
                    };
                    let _ = writeln!(
                        x,
                        "{head}>\n      <failure message=\"{detail}\" type=\"{kind}\">{detail}</failure>\n    </testcase>"
                    );
                }
                Outcome::Error => {
                    let _ = writeln!(
                        x,
                        "{head}>\n      <error message=\"{detail}\" type=\"error\">{detail}</error>\n    </testcase>"
                    );
                }
                Outcome::Skipped => {
                    let _ = writeln!(
                        x,
                        "{head}>\n      <skipped message=\"{detail}\"/>\n    </testcase>"
                    );
                }
            }
        }
        let _ = writeln!(
            x,
            "    <system-out>{}</system-out>\n  </testsuite>",
            xml_escape(&step_log(t))
        );
    }
    x.push_str("</testsuites>\n");
    x
}

/// `YYYY-MM-DDTHH:MM:SSZ` from Unix seconds (UTC), without a date library.
pub fn iso8601_utc(unix: u64) -> String {
    let (days, rem) = (unix / 86_400, unix % 86_400);
    // Howard Hinnant's civil_from_days
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() {
        "test".into()
    } else {
        out.chars().take(60).collect()
    }
}

/// Write `metadata.json`, the requested reports and one log per test. Returns the paths written.
pub fn write_artifacts(
    dir: &Path,
    report: &RunReport,
    formats: &[Format],
) -> std::io::Result<Vec<PathBuf>> {
    let mut written = Vec::new();
    let mut put = |rel: String, body: String| -> std::io::Result<()> {
        let path = dir.join(rel);
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::fs::write(&path, body)?;
        written.push(path);
        Ok(())
    };
    put(
        "metadata.json".into(),
        serde_json::to_string_pretty(&report.metadata).expect("metadata serializes"),
    )?;
    for f in formats {
        match f {
            Format::Json => put("results.json".into(), to_json(report))?,
            Format::Junit => put("results.xml".into(), to_junit(report))?,
        }
    }
    for (i, t) in report.tests.iter().enumerate() {
        put(
            format!("logs/{:02}-{}.log", i + 1, slug(&t.name)),
            step_log(t),
        )?;
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(
        index: usize,
        action: &str,
        outcome: Outcome,
        detail: &str,
        code: Option<u8>,
    ) -> StepResult {
        StepResult {
            index,
            action: action.into(),
            outcome,
            detail: detail.into(),
            duration_ms: 1500,
            exit_code: code,
        }
    }

    fn test(name: &str, outcome: Outcome, code: u8, steps: Vec<StepResult>) -> TestResult {
        TestResult {
            name: name.into(),
            file: "tests/a.yaml".into(),
            outcome,
            message: (outcome != Outcome::Passed).then(|| "why".into()),
            steps,
            duration_ms: 2400,
            exit_code: code,
        }
    }

    fn report(tests: Vec<TestResult>) -> RunReport {
        RunReport {
            metadata: Metadata {
                autolab_version: "0.1.0".into(),
                started_at: iso8601_utc(1_700_000_000),
                started_at_unix: 1_700_000_000,
                duration_ms: 3000,
                backend: Some(BackendMeta {
                    name: "dhu".into(),
                    ..Default::default()
                }),
                device_model: None,
                android_version: None,
                android_auto_version: None,
                display: None,
                summary: Summary::of(&tests),
            },
            exit_code: exit_code_for(&tests),
            interrupted: false,
            tests,
        }
    }

    #[test]
    fn escapes_markup_and_strips_characters_xml_forbids() {
        assert_eq!(xml_escape("a<b>&\"c'"), "a&lt;b&gt;&amp;&quot;c&apos;");
        assert_eq!(
            xml_escape("ok\x00\x01\x1b[31mred\x7f\u{FFFE}\n\t"),
            "ok[31mred\x7f\n\t"
        );
        assert_eq!(xml_escape("caf\u{e9} \u{1F600}"), "caf\u{e9} \u{1F600}");
    }

    #[test]
    fn iso8601_matches_known_instants() {
        assert_eq!(iso8601_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso8601_utc(1_700_000_000), "2023-11-14T22:13:20Z");
        assert_eq!(iso8601_utc(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(iso8601_utc(1_709_251_199), "2024-02-29T23:59:59Z");
    }

    #[test]
    fn exit_code_precedence_is_fixed_and_deterministic() {
        let t = |c| {
            test(
                "t",
                if c == 0 {
                    Outcome::Passed
                } else {
                    Outcome::Failed
                },
                c,
                vec![],
            )
        };
        assert_eq!(exit_code_for(&[]), 0);
        assert_eq!(exit_code_for(&[t(0), t(0)]), 0);
        assert_eq!(exit_code_for(&[t(7), t(6)]), 6);
        assert_eq!(
            exit_code_for(&[t(6), t(7)]),
            6,
            "order of tests must not matter"
        );
        assert_eq!(exit_code_for(&[t(7), t(3), t(5)]), 3);
        assert_eq!(exit_code_for(&[t(7), t(2)]), 2);
        assert_eq!(exit_code_for(&[t(1), t(7)]), 7);
        assert_eq!(exit_code_for(&[t(9)]), 9);
        assert_eq!(
            exit_code_for(&[t(7), t(130)]),
            130,
            "an interrupt outranks everything"
        );
    }

    #[test]
    fn json_report_uses_camel_case_and_explicit_nulls() {
        let r = report(vec![test(
            "t",
            Outcome::Passed,
            0,
            vec![step(0, "preflight", Outcome::Passed, "ok", None)],
        )]);
        let v: serde_json::Value = serde_json::from_str(&to_json(&r)).unwrap();
        assert_eq!(v["metadata"]["autolabVersion"], "0.1.0");
        assert_eq!(v["metadata"]["androidVersion"], serde_json::Value::Null);
        assert_eq!(v["metadata"]["deviceModel"], serde_json::Value::Null);
        assert_eq!(v["metadata"]["summary"]["passed"], 1);
        assert_eq!(v["tests"][0]["steps"][0]["outcome"], "passed");
        assert_eq!(v["exitCode"], 0);
    }

    #[test]
    fn junit_has_counts_children_and_escapes_hostile_detail() {
        let hostile = "[E]: bad <tag> & \"quote\" \x1b[31m";
        let r = report(vec![test(
            "Focus <recovery> & more",
            Outcome::Failed,
            7,
            vec![
                step(0, "preflight", Outcome::Passed, "ok", None),
                step(1, "assert.status", Outcome::Failed, hostile, Some(7)),
                step(2, "waitFor", Outcome::Failed, "late", Some(6)),
                step(3, "screenshot", Outcome::Error, "disk", Some(1)),
                step(4, "wait", Outcome::Skipped, "not run", None),
            ],
        )]);
        let x = to_junit(&r);
        assert!(x.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
        assert!(x.contains("<testsuites name=\"autolab\" tests=\"5\" failures=\"2\" errors=\"1\" skipped=\"1\" time=\"3.000\">"), "{x}");
        assert!(
            x.contains("testsuite name=\"Focus &lt;recovery&gt; &amp; more\""),
            "{x}"
        );
        assert!(x.contains("type=\"assertion\"") && x.contains("type=\"timeout\""));
        assert!(x.contains("<error message=\"disk\""));
        assert!(x.contains("<skipped message=\"not run\"/>"));
        assert!(!x.contains('\x1b'), "control characters must be stripped");
        assert!(x.contains("bad &lt;tag&gt; &amp; &quot;quote&quot;"));
        // No raw angle bracket may survive inside any attribute or text from the hostile detail.
        assert!(!x.contains("<tag>"));
        assert!(x.trim_end().ends_with("</testsuites>"));
    }

    #[test]
    fn artifacts_layout_matches_the_prd() {
        let dir = std::env::temp_dir().join(format!("al-report-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let r = report(vec![
            test(
                "Video focus recovery",
                Outcome::Passed,
                0,
                vec![step(0, "preflight", Outcome::Passed, "ok", None)],
            ),
            test("!!!", Outcome::Passed, 0, vec![]),
        ]);
        let written = write_artifacts(&dir, &r, &[Format::Json, Format::Junit]).unwrap();
        assert_eq!(written.len(), 5);
        for f in [
            "metadata.json",
            "results.json",
            "results.xml",
            "logs/01-video-focus-recovery.log",
            "logs/02-test.log",
        ] {
            assert!(dir.join(f).is_file(), "missing {f}");
        }
        let only_json = std::env::temp_dir().join(format!("al-report-json-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&only_json);
        write_artifacts(&only_json, &r, &[Format::Json]).unwrap();
        assert!(
            !only_json.join("results.xml").exists(),
            "unrequested formats are not written"
        );
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&only_json);
    }

    #[test]
    fn log_names_every_step_and_the_reason() {
        let t = test(
            "t",
            Outcome::Failed,
            7,
            vec![step(1, "assert.status", Outcome::Failed, "nope", Some(7))],
        );
        let log = step_log(&t);
        assert!(log.contains("FAIL  1. assert.status  nope"), "{log}");
        assert!(log.contains("Reason: why"));
    }
}
