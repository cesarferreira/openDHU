//! JSON, JUnit and HTML reports plus the artifacts directory layout from PRD section 12.6.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::Serialize;

pub use opendhu_core::iso8601_utc;

use crate::model::{Outcome, StepResult, TestResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Json,
    Junit,
    Html,
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
    pub opendhu_version: String,
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
        "<testsuites name=\"opendhu\" tests=\"{cases}\" failures=\"{}\" errors=\"{}\" skipped=\"{}\" time=\"{}\">",
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

/// A portable report with no network resources or scripts. Evidence paths stay relative to it.
pub fn to_html(report: &RunReport, dir: &Path) -> String {
    fn status(outcome: Outcome) -> &'static str {
        match outcome {
            Outcome::Passed => "passed",
            Outcome::Failed => "failed",
            Outcome::Error => "error",
            Outcome::Skipped => "skipped",
        }
    }
    fn url(path: &str) -> String {
        let mut encoded = String::new();
        for byte in path.bytes() {
            if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
                encoded.push(byte as char);
            } else {
                let _ = write!(encoded, "%{byte:02X}");
            }
        }
        encoded
    }
    fn screenshots(base: &Path, dir: &Path, found: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if kind.is_dir() {
                screenshots(base, &path, found);
            } else if kind.is_file()
                && path
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("png"))
            {
                if let Ok(relative) = path.strip_prefix(base) {
                    found.push(relative.to_string_lossy().replace('\\', "/"));
                }
            }
        }
    }
    let m = &report.metadata;
    let s = &m.summary;
    let mut html = String::from(include_str!("report_header.html"));
    let _ = write!(
        html,
        "<header><p class=\"eyebrow\">openDHU · Test report</p><h1>{}</h1><p>{} · {}s · openDHU {}</p></header>",
        if report.interrupted {
            "Interrupted"
        } else if report.exit_code == 0 {
            "Passed"
        } else {
            "Failed"
        },
        xml_escape(&m.started_at),
        secs(m.duration_ms),
        xml_escape(&m.opendhu_version)
    );
    let _ = write!(
        html,
        "<section class=\"counts\" aria-label=\"Summary\"><strong>{} tests</strong><span class=\"passed\">{} passed</span><span class=\"failed\">{} failed</span><span class=\"error\">{} errors</span><span class=\"skipped\">{} skipped</span></section>",
        s.tests, s.passed, s.failed, s.errors, s.skipped
    );
    if let Some(backend) = &m.backend {
        let _ = write!(html, "<p>Backend: {}</p>", xml_escape(&backend.name));
    }
    for (index, test) in report.tests.iter().enumerate() {
        let state = status(test.outcome);
        let _ = write!(
            html,
            "<details{}><summary><span class=\"badge {state}\">{state}</span> {} <small>{}s</small></summary><div class=\"body\"><p class=\"source\">{}</p><a href=\"logs/{:02}-{}.log\">Test log</a>",
            if test.passed() { "" } else { " open" },
            xml_escape(&test.name),
            secs(test.duration_ms),
            xml_escape(&test.file),
            index + 1,
            slug(&test.name)
        );
        if let Some(message) = &test.message {
            let _ = write!(html, "<p class=\"{state}\">{}</p>", xml_escape(message));
        }
        html.push_str("<div class=\"table\"><table><thead><tr><th>Step</th><th>Result</th><th>Duration</th><th>Details</th></tr></thead><tbody>");
        for step in &test.steps {
            let state = status(step.outcome);
            let _ = write!(
                html,
                "<tr><td>{}</td><td class=\"{state}\">{state}</td><td>{}s</td><td class=\"detail\">{}</td></tr>",
                xml_escape(&label(step)),
                secs(step.duration_ms),
                xml_escape(&step.detail)
            );
        }
        html.push_str("</tbody></table></div></div></details>");
    }
    let mut images = Vec::new();
    screenshots(dir, dir, &mut images);
    images.sort();
    if !images.is_empty() {
        let _ = write!(
            html,
            "<details><summary>Screenshots ({})</summary><div class=\"gallery\">",
            images.len()
        );
        for path in images {
            let href = url(&path);
            let caption = xml_escape(&path);
            let _ = write!(
                html,
                "<figure><a href=\"{href}\"><img loading=\"lazy\" src=\"{href}\" alt=\"{caption}\"></a><figcaption>{caption}</figcaption></figure>"
            );
        }
        html.push_str("</div></details>");
    }
    html.push_str("<footer><a href=\"metadata.json\">Run metadata</a> · Keep this file with its artifact folder to preserve evidence links.</footer></main></body></html>\n");
    html
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
            Format::Html => put("results.html".into(), to_html(report, dir))?,
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
                opendhu_version: "0.1.0".into(),
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
        assert_eq!(v["metadata"]["opendhuVersion"], "0.1.0");
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
        assert!(x.contains("<testsuites name=\"opendhu\" tests=\"5\" failures=\"2\" errors=\"1\" skipped=\"1\" time=\"3.000\">"), "{x}");
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
    fn html_report_escapes_content_and_links_portable_evidence() {
        let dir = std::env::temp_dir().join(format!("al-report-html-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("screenshots")).unwrap();
        std::fs::write(dir.join("screenshots/screen#1.png"), b"png").unwrap();
        let mut r = report(vec![test(
            "<script>alert(1)</script>",
            Outcome::Failed,
            7,
            vec![step(
                1,
                "ui.expect",
                Outcome::Failed,
                "Expected <Home> & more",
                Some(7),
            )],
        )]);
        r.interrupted = true;
        let written = write_artifacts(&dir, &r, &[Format::Html]).unwrap();
        assert!(written.contains(&dir.join("results.html")));
        let html = std::fs::read_to_string(dir.join("results.html")).unwrap();
        assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
        assert!(!html.contains("<script>"));
        assert!(html.contains("Expected &lt;Home&gt; &amp; more"));
        assert!(html.contains("1 failed") && html.contains("Interrupted"));
        assert!(html.contains("href=\"logs/01-script-alert-1-script.log\""));
        assert!(html.contains("screenshots/screen%231.png"));
        assert!(!html.contains(&dir.display().to_string()));
        std::fs::remove_dir_all(dir).unwrap();
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
