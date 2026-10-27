//! Reads text from a `uiautomator dump` of the phone's own screen.
//!
//! Only the phone's default display is visible this way: Android Auto draws the car UI into
//! windows that accessibility does not expose, so car-screen assertions still use OCR.

/// Every non-empty `text` and `content-desc` attribute in the dump, in document order.
pub fn visible_texts(xml: &str) -> Vec<String> {
    let mut out = Vec::new();
    for tag in xml.split("<node ").skip(1) {
        let tag = format!(" {}", tag.split('>').next().unwrap_or(tag));
        for name in ["text", "content-desc"] {
            if let Some(value) = attribute(&tag, name)
                && !value.trim().is_empty()
            {
                out.push(value);
            }
        }
    }
    out
}

/// The first visible text containing `needle`, ignoring case.
pub fn find<'a>(texts: &'a [String], needle: &str) -> Option<&'a str> {
    let needle = needle.trim().to_lowercase();
    texts
        .iter()
        .find(|t| t.to_lowercase().contains(&needle))
        .map(String::as_str)
}

fn attribute(tag: &str, name: &str) -> Option<String> {
    let key = format!(" {name}=\"");
    let start = tag.find(&key)? + key.len();
    let rest = &tag[start..];
    Some(unescape(&rest[..rest.find('"')?]))
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#10;", "\n")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    const DUMP: &str = r#"<?xml version='1.0'?><hierarchy rotation="0"><node index="0" text="" resource-id="" class="android.widget.FrameLayout" content-desc="" bounds="[0,0][1,1]"><node index="0" text="Harbour Cafe" resource-id="" class="android.widget.TextView" content-desc="" /><node index="1" text="" class="android.widget.ImageView" content-desc="Wifi &amp; signal" /><node index="2" text="Tom's &quot;place&quot;" content-desc="" /></node></hierarchy>
UI hierchary dumped to: /dev/tty"#;

    #[test]
    fn collects_text_and_content_description() {
        assert_eq!(
            visible_texts(DUMP),
            ["Harbour Cafe", "Wifi & signal", "Tom's \"place\""]
        );
    }

    #[test]
    fn find_is_case_insensitive_substring() {
        let t = visible_texts(DUMP);
        assert_eq!(find(&t, "harbour"), Some("Harbour Cafe"));
        assert_eq!(find(&t, "wifi &"), Some("Wifi & signal"));
        assert_eq!(find(&t, "Riverside"), None);
    }

    #[test]
    fn empty_dump_has_no_text() {
        assert!(visible_texts("").is_empty());
        assert!(visible_texts("<hierarchy/>").is_empty());
    }
}
