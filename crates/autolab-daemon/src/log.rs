//! Structured logs: one JSON object per line on stdout (the daemon's log file). Secrets such as
//! the token and the `Authorization` header must never be passed in here.

use autolab_core::{iso8601_utc, unix_now};
use serde_json::{Value, json};

pub fn info(event: &str, fields: Value) {
    emit("info", event, fields);
}

pub fn warn(event: &str, fields: Value) {
    emit("warn", event, fields);
}

fn emit(level: &str, event: &str, fields: Value) {
    let mut line = json!({ "ts": iso8601_utc(unix_now()), "level": level, "event": event });
    if let (Some(base), Some(extra)) = (line.as_object_mut(), fields.as_object()) {
        for (k, v) in extra {
            base.insert(k.clone(), v.clone());
        }
    }
    println!("{line}");
}
