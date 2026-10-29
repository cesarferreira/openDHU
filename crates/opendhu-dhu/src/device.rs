//! Device-scoped, bounded commands and Android Auto developer-server bootstrap.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use opendhu_core::{Error, Result};

const PACKAGE: &str = "com.google.android.projection.gearhead";
const SERVER: &str = ".companion.DeveloperHeadUnitNetworkService";

/// A session-owned forwarding listener for an SDK emulator's head-unit server.
/// Keeping this value alive keeps the listener available; failed startup and session teardown
/// release only this listener, leaving other adb forwards untouched.
pub struct EmulatorConnection {
    program: PathBuf,
    serial: String,
    port: u16,
}

impl EmulatorConnection {
    pub fn prepare(program: &Path, serial: &str) -> Result<Self> {
        restart_head_unit_server(program, serial)?;
        let allocated = adb(program, serial, &["forward", "tcp:0", "tcp:5277"])?;
        let port = allocated
            .trim()
            .parse::<u16>()
            .ok()
            .filter(|p| *p != 0)
            .ok_or_else(|| {
                Error::Other("adb did not return its allocated forwarding port".into())
            })?;
        Ok(Self {
            program: program.to_owned(),
            serial: serial.to_owned(),
            port,
        })
    }

    pub fn endpoint(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }
}

impl Drop for EmulatorConnection {
    fn drop(&mut self) {
        let _ = command(
            &self.program,
            &[
                "-s",
                &self.serial,
                "forward",
                "--remove",
                &format!("tcp:{}", self.port),
            ],
            Duration::from_secs(5),
        );
    }
}

/// Run adb against exactly one device. Arguments are never included in diagnostics.
pub fn adb(program: &Path, serial: &str, args: &[&str]) -> Result<String> {
    let mut scoped = vec!["-s", serial];
    scoped.extend_from_slice(args);
    command(program, &scoped, Duration::from_secs(30))
}

/// Run a command without printing output, killing it when the deadline expires.
pub fn command(program: &Path, args: &[&str], limit: Duration) -> Result<String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            Error::BackendUnavailable(format!("could not start {}: {e}", program.display()))
        })?;
    // Drain both pipes while the child runs; dumpsys output can exceed pipe capacity.
    let stdout = child.stdout.take().expect("stdout pipe");
    let stderr = child.stderr.take().expect("stderr pipe");
    let capture = |mut pipe: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = pipe.read_to_end(&mut bytes);
            String::from_utf8_lossy(&bytes).into_owned()
        })
    };
    let out = capture(Box::new(stdout));
    let err = capture(Box::new(stderr));
    let deadline = Instant::now() + limit;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                // Do not join: a descendant may have inherited a pipe.
                return Err(Error::Timeout(format!(
                    "command did not finish within {}s",
                    limit.as_secs_f64()
                )));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Error::Other(format!("could not wait for command: {e}")));
            }
        }
    };
    while !out.is_finished() || !err.is_finished() {
        if Instant::now() >= deadline {
            return Err(Error::Timeout(
                "command output pipes did not close before deadline".into(),
            ));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let stdout = out.join().unwrap_or_default();
    let stderr = err.join().unwrap_or_default();
    if status.success() {
        Ok(stdout)
    } else {
        Err(Error::Other(format!(
            "command failed ({status}); stdout: {}; stderr: {}",
            stdout.chars().take(2000).collect::<String>().trim(),
            stderr.chars().take(2000).collect::<String>().trim()
        )))
    }
}

/// Whether Android Auto's developer head-unit server is running on the device.
pub fn head_unit_server_running(program: &Path, serial: &str) -> Result<bool> {
    server_running(program, serial)
}

fn server_running(program: &Path, serial: &str) -> Result<bool> {
    let services = adb(
        program,
        serial,
        &["shell", "dumpsys", "activity", "services", PACKAGE],
    )?;
    Ok(services
        .lines()
        .any(|line| line.contains("ServiceRecord{") && line.contains(SERVER)))
}

/// Discard Quick Boot projection handlers before connecting a dedicated test session.
pub fn restart_head_unit_server(program: &Path, serial: &str) -> Result<()> {
    adb(
        program,
        serial,
        &[
            "shell",
            "am",
            "force-stop",
            "com.google.android.projection.gearhead",
        ],
    )?;
    let deadline = Instant::now() + Duration::from_secs(15);
    while server_running(program, serial)? {
        if Instant::now() >= deadline {
            return Err(Error::Timeout(
                "Android Auto head-unit server did not stop".into(),
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    ensure_head_unit_server(program, serial)
}

/// Set boolean entries in one of a debuggable app's SharedPreferences files, keeping every other
/// entry. The app is stopped first so it reads the file fresh on its next launch. Uses `run-as`,
/// so the app must be a debuggable build; nothing outside the app's own storage is touched.
pub fn set_boolean_preferences(
    program: &Path,
    serial: &str,
    package: &str,
    file: &str,
    booleans: &std::collections::BTreeMap<String, bool>,
) -> Result<()> {
    adb(program, serial, &["shell", "am", "force-stop", package])?;
    let existing = read_preferences(program, serial, package, file)?;
    write_preferences(
        program,
        serial,
        package,
        file,
        &with_boolean_preferences(&existing, booleans),
    )
}

/// Remove entries of any type from one of a debuggable app's SharedPreferences files, keeping
/// every other entry. Missing keys and a missing file are not errors. The app is stopped first so
/// it neither holds the old values in memory nor rewrites them on exit. Uses `run-as`, so the app
/// must be a debuggable build.
pub fn remove_preferences(
    program: &Path,
    serial: &str,
    package: &str,
    file: &str,
    keys: &[String],
) -> Result<()> {
    adb(program, serial, &["shell", "am", "force-stop", package])?;
    let existing = read_preferences(program, serial, package, file)?;
    if existing.is_empty() {
        return Ok(());
    }
    write_preferences(
        program,
        serial,
        package,
        file,
        &without_preferences(&existing, keys),
    )
}

/// The file's XML, or an empty string when it does not exist yet.
fn read_preferences(program: &Path, serial: &str, package: &str, file: &str) -> Result<String> {
    let path = format!("shared_prefs/{file}.xml");
    match adb(program, serial, &["shell", "run-as", package, "cat", &path]) {
        Ok(xml) => Ok(xml),
        Err(error) => {
            let text = error.message();
            if text.contains("not debuggable") || text.contains("Unknown package") {
                return Err(Error::BackendUnavailable(format!(
                    "cannot set app preferences: {package} must be a debuggable build installed on the device ({})",
                    text.trim()
                )));
            }
            Ok(String::new()) // The file does not exist yet.
        }
    }
}

fn write_preferences(
    program: &Path,
    serial: &str,
    package: &str,
    file: &str,
    xml: &str,
) -> Result<()> {
    let path = format!("shared_prefs/{file}.xml");
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let local =
        std::env::temp_dir().join(format!("opendhu-prefs-{}-{nonce}.xml", std::process::id()));
    let remote = format!(
        "/data/local/tmp/opendhu-prefs-{}-{nonce}.xml",
        std::process::id()
    );
    std::fs::write(&local, xml).map_err(|e| Error::Other(e.to_string()))?;
    let local_text = local.to_string_lossy().into_owned();
    let result = adb(program, serial, &["push", &local_text, &remote])
        .and_then(|_| {
            adb(
                program,
                serial,
                &["shell", "run-as", package, "mkdir", "-p", "shared_prefs"],
            )
        })
        .and_then(|_| {
            adb(
                program,
                serial,
                &["shell", "run-as", package, "cp", &remote, &path],
            )
        });
    let _ = adb(program, serial, &["shell", "rm", "-f", &remote]);
    let _ = std::fs::remove_file(&local);
    result.map(|_| ())
}

/// `xml` without the named entries. Android writes scalars as one `<tag name="k" ... />` line and
/// strings as `<string name="k">...</string>`, which can span lines; every other line is kept.
pub fn without_preferences(xml: &str, keys: &[String]) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut skipping_string = false;
    for line in xml.lines() {
        if skipping_string {
            skipping_string = !line.contains("</string>");
            continue;
        }
        let trimmed = line.trim_start();
        let removed = keys.iter().any(|key| {
            let name = format!(" name=\"{key}\"");
            trimmed
                .strip_prefix('<')
                .and_then(|rest| rest.split_once(&name))
                .is_some_and(|(tag, _)| {
                    !tag.is_empty() && tag.chars().all(|c| c.is_ascii_alphabetic())
                })
        });
        if removed {
            skipping_string = trimmed.starts_with("<string ")
                && !trimmed.contains("</string>")
                && !trimmed.trim_end().ends_with("/>");
            continue;
        }
        out.push(line);
    }
    let mut text = out.join("\n");
    text.push('\n');
    text
}

/// `xml` with each boolean set: an existing entry of the same name is replaced and everything else
/// is kept. Android writes `<boolean name="k" value="v" />` lines inside a single `<map>`.
pub fn with_boolean_preferences(
    xml: &str,
    booleans: &std::collections::BTreeMap<String, bool>,
) -> String {
    let mut lines: Vec<String> = xml.lines().map(str::to_string).collect();
    if !lines.iter().any(|l| l.contains("<map")) {
        lines = vec![
            "<?xml version='1.0' encoding='utf-8' standalone='yes' ?>".into(),
            "<map>".into(),
            "</map>".into(),
        ];
    }
    for line in &mut lines {
        if line.trim() == "<map />" {
            *line = "<map>".into();
        }
    }
    if !lines.iter().any(|l| l.trim() == "</map>") {
        lines.push("</map>".into());
    }
    for (key, value) in booleans {
        let marker = format!("<boolean name=\"{key}\" ");
        lines.retain(|l| !l.contains(&marker));
        let end = lines
            .iter()
            .rposition(|l| l.trim() == "</map>")
            .expect("map close tag");
        lines.insert(
            end,
            format!("    <boolean name=\"{key}\" value=\"{value}\" />"),
        );
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// Start the developer head-unit server through its public Settings UI.
/// Android Auto must already have developer mode and initial consent provisioned.
pub fn ensure_head_unit_server(program: &Path, serial: &str) -> Result<()> {
    if server_running(program, serial)? {
        return Ok(());
    }
    adb(
        program,
        serial,
        &[
            "shell",
            "am",
            "start",
            "-a",
            "com.google.android.projection.gearhead.SETTINGS",
        ],
    )?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = format!("/sdcard/opendhu-aa-{}-{nonce}.xml", std::process::id());
    let result = start_from_settings(program, serial, &path);
    let _ = adb(program, serial, &["shell", "rm", "-f", &path]);
    result
}

fn start_from_settings(program: &Path, serial: &str, path: &str) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut opened_menu = false;
    let mut last_note = String::new();
    loop {
        if server_running(program, serial)? {
            return Ok(());
        }
        // uiautomator can exit 0 without writing a dump while the UI is busy or not yet up.
        let dumped = adb(program, serial, &["shell", "uiautomator", "dump", path]);
        let xml = match &dumped {
            Ok(_) => adb(program, serial, &["shell", "cat", path]).ok(),
            Err(_) => None,
        };
        let Some(xml) = xml else {
            last_note = match dumped {
                Ok(out) => out.trim().to_string(),
                Err(e) => e.to_string(),
            };
            if Instant::now() >= deadline {
                return Err(provisioning_error(&last_note));
            }
            std::thread::sleep(Duration::from_millis(500));
            continue;
        };
        if let Some((x, y)) = node_center(&xml, "text", "Start head unit server") {
            tap(program, serial, x, y)?;
        } else if node_center(&xml, "text", "Stop head unit server").is_some() {
            // Another caller may have started it between checks. Never toggle it off.
        } else if !opened_menu {
            if let Some((x, y)) = node_center(&xml, "content-desc", "More options") {
                tap(program, serial, x, y)?;
                opened_menu = true;
            }
        }
        if Instant::now() >= deadline {
            return Err(provisioning_error(&last_note));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn provisioning_error(note: &str) -> Error {
    let note = if note.is_empty() {
        String::new()
    } else {
        format!(" Last uiautomator output: {note}.")
    };
    Error::BackendUnavailable(format!(
        "Android Auto head-unit server could not start. Provision this emulator baseline: open Android Auto Settings, enable developer mode by tapping Version repeatedly, accept the developer prompt, then choose More options > Start head unit server. Complete initial projection consent before starting a session. Settings UI must use English.{note}"
    ))
}

pub fn tap(program: &Path, serial: &str, x: u32, y: u32) -> Result<()> {
    adb(
        program,
        serial,
        &["shell", "input", "tap", &x.to_string(), &y.to_string()],
    )?;
    Ok(())
}

pub fn attribute<'a>(node: &'a str, key: &str) -> Option<&'a str> {
    let marker = format!(" {key}=\"");
    let value = node.split_once(&marker)?.1;
    Some(value.split_once('"')?.0)
}

fn node_center(xml: &str, key: &str, value: &str) -> Option<(u32, u32)> {
    xml.split("<node ")
        .filter_map(|tail| tail.split_once('>').map(|(tag, _)| tag))
        .find_map(|tag| {
            // Leading space preserves attribute boundaries, including the first attr.
            let tag = format!(" {tag}");
            if attribute(&tag, key)? != value || attribute(&tag, "enabled") == Some("false") {
                return None;
            }
            bounds_center(attribute(&tag, "bounds")?)
        })
}

pub fn bounds_center(bounds: &str) -> Option<(u32, u32)> {
    let (first, second) = bounds
        .strip_prefix('[')?
        .strip_suffix(']')?
        .split_once("][")?;
    let pair = |value: &str| -> Option<(u32, u32)> {
        let (x, y) = value.split_once(',')?;
        Some((x.parse().ok()?, y.parse().ok()?))
    };
    let (left, top) = pair(first)?;
    let (right, bottom) = pair(second)?;
    if right <= left || bottom <= top {
        return None;
    }
    Some((left + (right - left) / 2, top + (bottom - top) / 2))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferences_are_merged_without_losing_other_entries() {
        let flags = std::collections::BTreeMap::from([("EnableRobotaxiMode".to_string(), true)]);
        let fresh = with_boolean_preferences("", &flags);
        assert!(fresh.contains("<map>") && fresh.contains("</map>"));
        assert!(fresh.contains(r#"<boolean name="EnableRobotaxiMode" value="true" />"#));

        let existing = "<?xml version='1.0' encoding='utf-8' standalone='yes' ?>\n<map>\n    <string name=\"keep\">me</string>\n    <boolean name=\"EnableRobotaxiMode\" value=\"false\" />\n    <boolean name=\"Other\" value=\"true\" />\n</map>\n";
        let merged = with_boolean_preferences(existing, &flags);
        assert!(merged.contains(r#"<string name="keep">me</string>"#));
        assert!(merged.contains(r#"<boolean name="Other" value="true" />"#));
        assert_eq!(merged.matches("EnableRobotaxiMode").count(), 1);
        assert!(merged.contains(r#"<boolean name="EnableRobotaxiMode" value="true" />"#));

        let empty_map = "<?xml version='1.0' encoding='utf-8' standalone='yes' ?>\n<map />\n";
        let filled = with_boolean_preferences(empty_map, &flags);
        assert!(filled.contains("<map>") && !filled.contains("<map />"));
        assert_eq!(with_boolean_preferences(&filled, &flags), filled);
    }

    #[test]
    fn removed_preferences_drop_scalars_and_multiline_strings_only() {
        let xml = "<?xml version='1.0' encoding='utf-8' standalone='yes' ?>\n<map>\n    <string name=\"vehicleMetadata\">{&quot;a&quot;:1}</string>\n    <string name=\"spans\">first\nsecond</string>\n    <string name=\"keep\">me</string>\n    <boolean name=\"flag\" value=\"true\" />\n    <int name=\"vehicleMetadataCount\" value=\"3\" />\n</map>\n";
        let keys = [
            "vehicleMetadata".to_string(),
            "spans".to_string(),
            "flag".to_string(),
        ];
        let out = without_preferences(xml, &keys);
        assert!(
            !out.contains("vehicleMetadata\"") && !out.contains("spans") && !out.contains("flag")
        );
        assert!(!out.contains("second"));
        assert!(out.contains(r#"<string name="keep">me</string>"#));
        assert!(out.contains(r#"<int name="vehicleMetadataCount" value="3" />"#));
        assert!(out.starts_with("<?xml") && out.contains("<map>") && out.contains("</map>"));
        assert_eq!(without_preferences(&out, &keys), out);
    }

    #[cfg(unix)]
    #[test]
    fn restart_waits_for_dying_service_before_starting_settings() {
        use std::os::unix::fs::PermissionsExt;

        struct Fixture(std::path::PathBuf);
        impl Drop for Fixture {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let fixture = Fixture(
            std::env::temp_dir().join(format!("aa-dying-server-{}-{nonce}", std::process::id())),
        );
        std::fs::create_dir_all(&fixture.0).unwrap();
        let program = fixture.0.join("adb");
        std::fs::write(
            &program,
            r#"#!/bin/sh
set -eu
state="$(dirname "$0")"
[ "$1" = '-s' ] && [ "$2" = 'emulator-test' ]
shift 2
case "$*" in
  'shell am force-stop com.google.android.projection.gearhead')
    echo force-stop >> "$state/events"
    echo 0 > "$state/polls"
    ;;
  'shell dumpsys activity services com.google.android.projection.gearhead')
    if [ -f "$state/started" ]; then
      echo 'ServiceRecord{new .companion.DeveloperHeadUnitNetworkService}'
    else
      count=$(cat "$state/polls")
      count=$((count + 1))
      echo "$count" > "$state/polls"
      if [ "$count" -le 2 ]; then
        echo present >> "$state/events"
        echo 'ServiceRecord{dying .companion.DeveloperHeadUnitNetworkService}'
      else
        echo absent >> "$state/events"
        touch "$state/stopped"
      fi
    fi
    ;;
  'shell am start -a com.google.android.projection.gearhead.SETTINGS')
    [ -f "$state/stopped" ] || { echo 'Settings opened before old service stopped' >&2; exit 7; }
    echo settings >> "$state/events"
    ;;
  'shell uiautomator dump '*) ;;
  'shell cat '*)
    if [ -f "$state/menu" ] && [ ! -f "$state/stale-read" ]; then
      touch "$state/stale-read"
      echo '<hierarchy><node content-desc="More options" bounds="[10,20][30,40]"/></hierarchy>'
    elif [ -f "$state/menu" ]; then
      echo '<hierarchy><node text="Start head unit server" bounds="[40,50][80,90]"/></hierarchy>'
    else
      echo '<hierarchy><node content-desc="More options" bounds="[10,20][30,40]"/></hierarchy>'
    fi
    ;;
  'shell input tap 20 30') touch "$state/menu" ;;
  'shell input tap 60 70')
    touch "$state/started"
    echo start-server >> "$state/events"
    ;;
  'shell rm -f '*) ;;
  *) echo "Unexpected fake adb command" >&2; exit 8 ;;
esac
"#,
        )
        .unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();

        restart_head_unit_server(&program, "emulator-test").unwrap();

        let events = std::fs::read_to_string(fixture.0.join("events")).unwrap();
        let events: Vec<_> = events.lines().collect();
        assert_eq!(
            &events[..4],
            &["force-stop", "present", "present", "absent"]
        );
        let settings = events
            .iter()
            .position(|event| *event == "settings")
            .unwrap();
        let started = events
            .iter()
            .position(|event| *event == "start-server")
            .unwrap();
        assert!(
            settings > 3,
            "Settings started while old service was present"
        );
        assert!(
            started > settings,
            "server must start through fresh Settings UI"
        );
    }

    #[test]
    fn selects_exact_enabled_node_and_computes_center() {
        let xml = r#"<hierarchy><node text="Start head unit server elsewhere" bounds="[0,0][20,20]"/><node text="Start head unit server" enabled="false" bounds="[0,0][20,20]"/><node text="Start head unit server" enabled="true" bounds="[607,514][1038,578]"/></hierarchy>"#;
        assert_eq!(
            node_center(xml, "text", "Start head unit server"),
            Some((822, 546))
        );
        assert_eq!(node_center(xml, "content-desc", "More options"), None);
    }

    #[test]
    fn bounds_reject_invalid_empty_reversed_and_overflow() {
        for value in [
            "",
            "[1,2][1,4]",
            "[20,1][10,2]",
            "[-1,0][3,4]",
            "[0,0][99999999999,1]",
            "[0,0][1,1]junk",
        ] {
            assert_eq!(bounds_center(value), None, "{value}");
        }
        assert_eq!(
            bounds_center("[4294967290,0][4294967295,8]"),
            Some((4294967292, 4))
        );
    }

    #[test]
    fn command_failure_includes_both_output_streams() {
        let error = command(
            Path::new("/bin/sh"),
            &["-c", "printf out; printf err >&2; exit 7"],
            Duration::from_secs(1),
        )
        .unwrap_err();
        let message = error.to_string();
        assert!(message.contains("stdout: out"));
        assert!(message.contains("stderr: err"));
    }

    #[test]
    fn command_deadline_is_bounded() {
        let start = Instant::now();
        let error = command(
            Path::new("/bin/sh"),
            &["-c", "exec sleep 5"],
            Duration::from_millis(40),
        )
        .unwrap_err();
        assert!(matches!(error, Error::Timeout(_)));
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn command_drains_more_than_pipe_capacity() {
        let output = command(
            Path::new("/bin/sh"),
            &["-c", "head -c 200000 /dev/zero"],
            Duration::from_secs(2),
        )
        .unwrap();
        assert_eq!(output.len(), 200000);
    }

    #[test]
    fn missing_adb_is_actionable() {
        assert!(matches!(
            adb(Path::new("/does-not-exist/adb"), "emulator-5554", &[]),
            Err(Error::BackendUnavailable(_))
        ));
    }
}
