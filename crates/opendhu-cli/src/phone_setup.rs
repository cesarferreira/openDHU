//! Phone-side Android Auto setup that can only be done through the phone's own UI: the first-run
//! welcome sheet. It is driven with uiautomator, like the head-unit server toggle, and does nothing
//! when the phone is already set up.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use opendhu_core::Result;

use crate::device::{adb, attribute, bounds_center, tap};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Node {
    pub text: String,
    pub description: String,
    pub class: String,
    pub checked: bool,
    pub center: Option<(u32, u32)>,
}

pub(crate) fn nodes(xml: &str) -> Vec<Node> {
    xml.split("<node ")
        .skip(1)
        .filter_map(|tail| tail.split_once('>').map(|(tag, _)| format!(" {tag}")))
        .map(|tag| Node {
            text: unescape(attribute(&tag, "text").unwrap_or_default()),
            description: unescape(attribute(&tag, "content-desc").unwrap_or_default()),
            class: attribute(&tag, "class").unwrap_or_default().to_string(),
            checked: attribute(&tag, "checked") == Some("true"),
            center: attribute(&tag, "bounds").and_then(bounds_center),
        })
        .collect()
}

fn unescape(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

fn find<'a>(nodes: &'a [Node], text: &str) -> Option<&'a Node> {
    nodes.iter().find(|n| n.text == text && n.center.is_some())
}

fn dump(program: &Path, serial: &str) -> Option<Vec<Node>> {
    let path = "/sdcard/opendhu-aa-setup.xml";
    adb(program, serial, &["shell", "uiautomator", "dump", path]).ok()?;
    let xml = adb(program, serial, &["shell", "cat", path]).ok()?;
    Some(nodes(&xml))
}

fn focus(program: &Path, serial: &str) -> String {
    adb(program, serial, &["shell", "dumpsys", "window"])
        .unwrap_or_default()
        .lines()
        .rfind(|l| l.contains("mCurrentFocus=") && !l.contains("mCurrentFocus=null"))
        .unwrap_or_default()
        .to_string()
}

/// Whether Android Auto's phone-side setup screen is in front.
fn on_preflight_screen(program: &Path, serial: &str) -> bool {
    let focus = focus(program, serial);
    focus.contains("com.google.android.projection.gearhead") && focus.contains(".preflight.")
}

/// Press "Continue" if the first-run welcome sheet is in front. Returns whether it was pressed.
fn accept_first_run_once(program: &Path, serial: &str) -> Result<bool> {
    if !on_preflight_screen(program, serial) {
        return Ok(false);
    }
    if let Some(screen) = dump(program, serial)
        && find(&screen, "Welcome to Android Auto").is_some()
        && let Some((x, y)) = find(&screen, "Continue").and_then(|n| n.center)
    {
        tap(program, serial, x, y)?;
        return Ok(true);
    }
    Ok(false)
}

/// Watches the phone for the first-run welcome sheet for as long as it lives. The sheet only
/// appears once a head unit connects, at a time that varies, and nothing but this can press it in
/// an unattended run. A phone that is already set up costs one cheap `dumpsys` every few seconds.
pub struct FirstRunWatcher {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl FirstRunWatcher {
    pub fn start(program: PathBuf, serial: String) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let handle = std::thread::spawn(move || {
            while !flag.load(Ordering::Relaxed) {
                let _ = accept_first_run_once(&program, &serial);
                // Sleep in short steps so dropping the watcher never waits long.
                for _ in 0..10 {
                    if flag.load(Ordering::Relaxed) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(200));
                }
            }
        });
        Self {
            stop,
            handle: Some(handle),
        }
    }
}

impl Drop for FirstRunWatcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHEET: &str = r#"<hierarchy><node index="0" text="Welcome to Android Auto" resource-id="" class="android.widget.TextView" checked="false" bounds="[63,1084][1017,1302]" /><node index="1" text="Continue" class="android.widget.Button" checked="false" bounds="[713,2123][1017,2253]" /></hierarchy>"#;

    #[test]
    fn finds_the_continue_button_center() {
        let screen = nodes(SHEET);
        assert!(find(&screen, "Welcome to Android Auto").is_some());
        assert_eq!(find(&screen, "Continue").unwrap().center, Some((865, 2188)));
        assert!(find(&screen, "Missing").is_none());
    }

    #[test]
    fn unescapes_entities_in_labels() {
        let screen = nodes(r#"<node text="Help &amp; feedback" bounds="[0,0][10,10]" />"#);
        assert_eq!(screen[0].text, "Help & feedback");
    }
}
