# Vision

openDHU's purpose is to replace the manual, DHU-driven workflow of testing Android Auto apps with a fully
scriptable one: start a head unit from a terminal, drive it like Espresso drives an Android app, and get the same
controls from a CLI, a YAML test, an HTTP API or a desktop window.

**Core principle:** everything the GUI can do, the CLI and the HTTP API can do too. No GUI-only functionality.

This document used to be a 2000-line, pre-implementation PRD (Swift/SwiftUI and Kotlin were once considered; audio
focus, navigation focus, night mode and driving restrictions were once in scope). What was actually built, and in
Rust, is the README and [docs/architecture.md](architecture.md). The "Not implemented" section of the README is the
current list of what this vision doesn't cover yet.
