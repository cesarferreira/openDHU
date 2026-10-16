//! `autolab-native-hu`: AutoLab's own Android Auto head unit, started by the daemon.
//!
//! It speaks JSON lines on stdin and stdout (`autolab_native::protocol`) and exits when stdin
//! closes. Stdout carries protocol lines only. Set `AUTOLAB_NATIVE_LOG=1` for the protocol crate's
//! log on stderr.

mod headunit;
mod screenshot;

use std::io::Write;
use std::sync::Arc;

use autolab_native::protocol::{self, Event, Line};
use tokio::io::{AsyncBufReadExt, BufReader};

/// Every protocol line goes through here so two tasks never interleave bytes in one line.
pub fn emit(line: &Line) {
    let text = protocol::encode(line);
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(text.as_bytes());
    let _ = out.flush();
}

/// A `VideoData` line followed by that many raw bytes, written under one lock so nothing can come
/// between them.
pub fn emit_video(data: &[u8]) {
    let head = protocol::encode(&Line::Event(Event::VideoData { len: data.len() }));
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(head.as_bytes());
    let _ = out.write_all(data);
    let _ = out.flush();
}

#[tokio::main]
async fn main() {
    if std::env::var_os("AUTOLAB_NATIVE_LOG").is_some() {
        let _ = simple_logger::SimpleLogger::new()
            .with_level(log::LevelFilter::Info)
            .init();
    }
    let setup = android_auto::setup();
    let hu = Arc::new(headunit::Shared::new());

    // stdin is the lifetime: closing it (or a `quit`) ends the process, whatever the phone is doing.
    let handler = Arc::clone(&hu);
    tokio::spawn(async move {
        let mut lines = BufReader::new(tokio::io::stdin()).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let Ok(req) = serde_json::from_str::<protocol::Request>(&line) else {
                continue;
            };
            if req.command == protocol::Command::Quit {
                std::process::exit(0);
            }
            let hu = Arc::clone(&handler);
            // Each request on its own task: a screenshot must not block a status read.
            tokio::spawn(async move {
                let outcome = hu.handle(req.command).await;
                emit(&Line::Response(protocol::Response {
                    id: req.id,
                    outcome,
                }));
            });
        }
        std::process::exit(0);
    });

    let reason = match headunit::run(Arc::clone(&hu), &setup).await {
        Ok(()) => "the USB link to the phone ended".to_string(),
        Err(e) => format!("the Android Auto session failed: {e}"),
    };
    emit(&Line::Event(Event::Disconnected { reason }));
    std::process::exit(0);
}
