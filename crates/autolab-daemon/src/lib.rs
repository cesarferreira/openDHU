//! The AutoLab daemon: owns the head unit session and serves the loopback HTTP API (`/v1`).
//!
//! Trust model: anyone who can read `daemon.json` (it lives in a 0700 directory, mode 0600) holds
//! the bearer token and can make the daemon launch an executable and write PNG files as the user.
//! That is the same power as running `aa` yourself. See `docs/api.md`.

pub mod api;
mod auth;
mod backend;
pub mod client;
mod error;
mod events;
mod launch;
pub mod log;
pub mod runtime;
mod server;
mod state;
mod tunnels;

pub use client::Client;
pub use launch::find_native_hu;
pub use server::{DEFAULT_PORT, ServeOptions, router, run};
