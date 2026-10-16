# AutoLab — Programmable Android Auto Head Unit & Testing Framework

**Product Requirements Document (PRD)**

**Version:** 1.0  
**Status:** Ready for implementation  
**Target platforms:** macOS (primary), Linux (secondary)  
**Primary languages:** Rust, Swift/SwiftUI, Kotlin  
**License:** Apache 2.0 for original project code  
**Product type:** Open-source developer tool  
**Working name:** AutoLab

---

# 1. Executive Summary

AutoLab is an open-source, programmable Android Auto head unit and end-to-end testing framework.

Its purpose is to replace the manual, limited workflow of Google's Android Auto Desktop Head Unit (DHU) with a fully scriptable development environment.

Developers should be able to:

- Launch an Android Auto head unit from their terminal.
- Connect a physical Android phone running Android Auto.
- See the projected Android Auto interface in a native desktop window.
- Control head unit behaviour programmatically.
- Toggle video focus, audio focus, navigation focus, driving state, and night mode.
- Simulate touchscreen interactions.
- Capture screenshots and recordings.
- Run automated end-to-end tests.
- Assert application state and visible UI elements.
- Run tests headlessly in CI.
- Integrate tests with JUnit and Android development workflows.

The long-term vision is **Espresso for Android Auto**.

Unlike Espresso, which operates primarily within an Android application process, AutoLab will provide end-to-end testing across the Android Auto projection boundary.

## 1.1 Core principle

**Everything that can be done through the GUI must also be possible through the CLI and automation API.**

No GUI-only functionality.

## 1.2 Implementation strategy

AutoLab must support interchangeable head unit backends.

The first backend will automate Google's existing DHU, allowing rapid development and validation of the testing framework.

The second backend will implement an independent Android Auto receiver using existing open-source implementations where feasible.

Both backends must implement a common interface.

The testing framework must not depend on a particular head unit implementation.

---

# 2. Problem Statement

Google's Android Auto DHU is useful for manually testing Android Auto applications, but it is not designed as a modern, programmable test environment.

Current pain points include:

1. Head unit controls are cumbersome to operate repeatedly.
2. Focus changes require manual interaction or console commands.
3. There is no comprehensive, stable automation API.
4. Automated interaction with projected applications is difficult.
5. End-to-end assertions against Android Auto UI are limited.
6. Regression tests are difficult to reproduce reliably.
7. CI integration requires custom tooling.
8. Developers cannot freely extend Google's proprietary DHU.
9. Recording and replaying user interactions is not straightforward.
10. Testing lifecycle and projection edge cases is unnecessarily manual.

For example, consider testing whether an Android Auto navigation application recovers correctly after losing video focus.

Today, a developer may need to:

1. Launch the DHU.
2. Connect their Android device.
3. Launch the navigation application.
4. Manually trigger a video-focus change.
5. Restore video focus.
6. Visually inspect the application.
7. Repeat the process after making code changes.

AutoLab should turn this into a repeatable automated test.

---

# 3. Goals

## 3.1 Primary goals

### G1 — Programmable head unit

Expose head unit functionality through a consistent CLI and local API.

### G2 — Automated end-to-end testing

Allow developers to define, execute, and validate Android Auto interactions without manually operating the DHU.

### G3 — Backend independence

Support both Google's DHU and an independent open-source receiver through the same public API.

### G4 — Native developer experience

Provide a lightweight native macOS application with convenient controls, keyboard shortcuts, and live projection.

### G5 — Headless execution

Allow tests to execute without a visible desktop window, provided the selected backend and device environment support it.

### G6 — CI integration

Provide deterministic exit codes, structured reports, screenshots, logs, and JUnit-compatible test results.

### G7 — Extensibility

Allow future integration with AndroidX Car App testing, custom application instrumentation, and richer semantic UI assertions.

## 3.2 Non-goals for MVP

The initial implementation must not attempt to:

- Reimplement the entire Android Auto protocol.
- Recreate Android Auto's UI rendering.
- Replace Android Automotive OS.
- Support Apple CarPlay.
- Provide a cloud device farm.
- Automatically generate AI-based tests.
- Implement OCR-based element detection.
- Provide a browser-based frontend.
- Support multiple concurrent head units.
- Require modifications to the application under test.
- Guarantee compatibility with every Android Auto version.

The MVP must focus on providing reliable automation around an existing DHU installation.

---

# 4. Target Users

## 4.1 Android Auto developers

Developers implementing applications with AndroidX Car App Library.

Typical applications include:

- Navigation applications.
- Media applications.
- Point-of-interest applications.
- Communication applications.
- Other supported Android Auto app categories.

## 4.2 Android QA engineers

Engineers responsible for regression testing, compatibility testing, and release validation.

## 4.3 CI and build engineers

Engineers who need repeatable, machine-readable Android Auto test execution.

## 4.4 Open-source developers

Developers interested in extending Android Auto tooling, exploring the projection protocol, or implementing custom receiver functionality.

---

# 5. Product Architecture

AutoLab will be built as a modular system.

```text
                   AutoLab
                      |
       +--------------+--------------+
       |              |              |
   macOS GUI         CLI         Test Runner
    SwiftUI          Rust          Rust
       |              |              |
       +--------------+--------------+
                      |
               AutoLab Daemon
                  Rust
                      |
               Local JSON API
                      |
              HeadUnitBackend
                      |
           +----------+----------+
           |                     |
       DHU Backend          Native Backend
       Google DHU           Open Receiver
           |                     |
           +----------+----------+
                      |
              Android Auto
                 Device
                      |
             Application Under
                    Test
```

## 5.1 Major components

### autolab-core

Rust library containing:

- Backend abstractions.
- Head unit state.
- Session management.
- Focus management.
- Input events.
- Screenshot management.
- Test execution primitives.
- Event collection.
- Error definitions.

Must not depend on a GUI framework.

### autolab-daemon

Long-running local process responsible for:

- Managing head unit sessions.
- Starting and stopping backends.
- Exposing the automation API.
- Maintaining connection state.
- Streaming events.
- Executing operations.
- Managing session ownership.
- Recording diagnostic information.

### autolab-cli

Rust executable providing a user-friendly command-line interface.

Binary name:

`aa`

### autolab-dhu

Backend adapter for Google's official DHU.

Responsible for:

- Discovering the DHU executable.
- Launching and terminating the DHU.
- Managing its console input/output.
- Translating AutoLab operations into supported DHU commands.
- Tracking supported capabilities.
- Detecting connection state where possible.
- Handling errors and process termination.

### autolab-native

Future independent Android Auto receiver.

Responsible for:

- Android Auto protocol communication.
- Video decoding.
- Audio handling.
- Input forwarding.
- Focus events.
- Projection lifecycle.
- Device communication.

### autolab-test

Test runner supporting:

- YAML test definitions.
- Assertions.
- Timeouts.
- Screenshots.
- Failure diagnostics.
- Test reports.
- Retry policies.
- Test isolation.

### autolab-macos

Native macOS frontend built with SwiftUI.

Communicates with autolab-daemon.

Must not contain head unit business logic.

---

# 6. Technology Stack

## 6.1 Rust

Use Rust for:

- Core business logic.
- CLI.
- Daemon.
- Backend interfaces.
- Test runner.
- Local API.
- Process management.

Recommended dependencies, subject to evaluation:

| Requirement | Candidate |
|---|---|
| Async runtime | Tokio |
| CLI | clap |
| Serialization | serde |
| YAML | serde_yaml or maintained equivalent |
| HTTP API | axum |
| HTTP client | reqwest |
| Logging | tracing |
| Error handling | thiserror / anyhow |
| Testing | cargo test |
| Snapshots | insta |

Use minimal dependencies and avoid introducing frameworks without clear justification.

## 6.2 macOS GUI

Use SwiftUI.

Requirements:

- Native macOS appearance.
- Dark and light mode.
- Keyboard shortcuts.
- Responsive layout.
- Minimal resource consumption.
- No Electron.
- No browser-based UI wrapper.

## 6.3 Android testing SDK

Use Kotlin for the optional app-aware testing integration.

Support JUnit-based tests where practical.

The Rust test runner remains the primary orchestration engine.

## 6.4 Native projection backend

Evaluate these projects before choosing an implementation:

- https://github.com/opencardev/openauto
- https://github.com/opencardev/aasdk
- https://github.com/buzzcola3/OpenAutoCore
- https://github.com/mossyhub/openautolink

Assess current maintenance status, platform compatibility, supported transport, protocol coverage, dependencies, and licensing.

Do not assume these projects are directly usable on macOS or compatible with current Android Auto versions.

A separate technical feasibility spike is required before committing to the native receiver architecture.

---

# 7. Functional Requirements

## FR-001 — Head Unit Lifecycle

The CLI must support:

```bash
aa start
aa stop
aa restart
aa status
```

Explicit backend selection:

```bash
aa start --backend dhu
aa start --backend native
```

The default backend for MVP is `dhu`.

Expected behaviour:

- Detect existing sessions.
- Avoid duplicate sessions.
- Provide meaningful errors.
- Track process lifecycle.
- Clean up child processes.
- Support graceful shutdown.
- Recover from unexpected backend termination.

Example status output:

```text
AutoLab 0.1.0

Backend:      dhu
State:        connected
Device:       Pixel 6
Resolution:   1280x720
Video focus:  on
Audio focus:  on
Night mode:   off
Session:      8f31c2
```

Only report values that can actually be observed or confirmed.

Unknown state must be represented explicitly rather than guessed.

---

## FR-002 — Video Focus

The system must support:

```bash
aa focus video on
aa focus video off
```

Requirements:

- Toggle video focus.
- Support repeatable calls.
- Return meaningful success or failure information.
- Record requested state changes.
- Expose observed state separately where available.
- Publish focus events when the backend provides them.

Example:

```bash
aa focus video off
```

Expected output:

```text
Video focus command accepted
```

Do not claim that the Android device has acknowledged the change unless acknowledgement is available.

This distinction is important for automated testing.

---

## FR-003 — Audio Focus

Commands:

```bash
aa focus audio on
aa focus audio off
```

The implementation must distinguish between backend-supported audio-focus operations and unsupported generic focus requests.

Where the backend requires a specific focus type or state, expose that capability explicitly.

---

## FR-004 — Navigation Focus

Commands:

```bash
aa focus nav on
aa focus nav off
```

Must support backend-specific navigation focus capabilities where available.

Unsupported operations must fail with a clear capability error.

---

## FR-005 — Driving Restrictions

Commands:

```bash
aa driving parked
aa driving moving
```

Expected behaviour:

- Simulate supported driving restriction states.
- Trigger relevant backend restriction commands.
- Allow tests to verify application behaviour under restrictions.

Important: driving restrictions are not necessarily equivalent to a vehicle speed signal.

The API must model these as distinct concepts.

Vehicle speed simulation may be added when supported by the native backend.

---

## FR-006 — Night Mode

Commands:

```bash
aa night-mode on
aa night-mode off
```

Requirements:

- Change the reported display mode where supported.
- Allow tests to validate theme transitions.
- Support screenshot capture after changes.
- Report unsupported operations correctly.

---

## FR-007 — Touch Input

Commands:

```bash
aa tap 400 300
aa swipe 100 500 100 200
aa long-press 400 300
```

Coordinate system:

- Coordinates refer to the projected Android Auto viewport.
- Origin is the top-left corner.
- Units are logical viewport pixels.
- Scaling between the desktop window and projection viewport must be handled internally.

Requirements:

- Validate coordinates.
- Support configurable gesture duration.
- Avoid desktop-window chrome offsets.
- Handle viewport resizing.
- Report whether the backend supports the requested input type.

Example:

```bash
aa swipe 500 600 500 200 --duration 350
```

For the DHU backend, investigate whether these operations can be implemented reliably through supported interfaces.

Do not assume arbitrary touch injection is available through the DHU console.

If unavailable, mark the operation unsupported for that backend rather than using fragile global mouse automation silently.

---

## FR-008 — Keyboard and Rotary Input

Proposed commands:

```bash
aa key back
aa key home
aa rotary clockwise
aa rotary counterclockwise
aa rotary select
```

These are capability-dependent operations.

The initial implementation must discover and document what each backend actually supports.

---

## FR-009 — Screenshots

Command:

```bash
aa screenshot output.png
```

Requirements:

- Capture the projected Android Auto display.
- Exclude the AutoLab GUI.
- Produce PNG files.
- Preserve the projection's native resolution where possible.
- Return the absolute output path.
- Support headless operation where available.

Additional options:

```bash
aa screenshot --output screenshots/home.png
```

Screenshots must contain only the relevant head unit viewport, not desktop window decorations.

If a backend cannot provide a clean projected frame, that limitation must be reported.

---

## FR-010 — Screen Recording

Command:

```bash
aa record start --output recording.mp4
aa record stop
```

Requirements:

- Capture the projection viewport.
- Record timestamps.
- Support configurable frame rate.
- Avoid capturing unrelated desktop content.
- Produce playable output files.

Recording is a post-MVP feature.

---

## FR-011 — Display Configuration

Commands:

```bash
aa display resolution 1280x720
aa display dpi 160
aa display orientation landscape
```

Requirements:

- Configure supported projection properties.
- Validate backend limitations.
- Distinguish configuration requiring reconnection from live configuration.
- Report effective settings.

Do not assume all properties can be changed during an active projection session.

---

## FR-012 — Device Management

Commands:

```bash
aa devices
aa device select SERIAL
aa device status
```

Use ADB where appropriate.

Requirements:

- Detect attached Android devices.
- Display serial, model, and connection state.
- Support explicit device selection.
- Detect device disconnection.
- Avoid silently selecting the wrong device when multiple devices are present.

Example:

```text
SERIAL            MODEL       STATE
1A141FDF600AF2    Pixel 6     device
```

ADB visibility alone does not imply that Android Auto projection is connected.

Track ADB connectivity and projection connectivity separately.

---

# 8. CLI Design

The CLI is a first-class product interface.

## 8.1 Design principles

- Commands must be discoverable.
- Syntax must be consistent.
- Help must be comprehensive.
- Errors must be actionable.
- Commands must work in shell scripts.
- JSON output must be supported.
- Exit codes must be deterministic.

Example:

```bash
aa status --json
```

Output:

```json
{
  "backend": "dhu",
  "sessionId": "8f31c2",
  "state": "connected",
  "device": {
    "serial": "1A141FDF600AF2",
    "model": "Pixel 6"
  },
  "focus": {
    "video": {
      "requested": "on",
      "observed": "unknown"
    }
  }
}
```

## 8.2 Exit codes

| Code | Meaning |
|---|---|
| 0 | Success |
| 1 | General error |
| 2 | Invalid arguments |
| 3 | Backend unavailable |
| 4 | Device disconnected |
| 5 | Operation unsupported |
| 6 | Timeout |
| 7 | Assertion failure |
| 8 | Session conflict |

## 8.3 Output conventions

Default output must be concise and human-readable.

Machine-readable output:

```bash
aa status --json
```

Diagnostic output:

```bash
aa --verbose status
```

The CLI must never report an operation as successful merely because a command was written to a process.

Use clear result semantics:

- Accepted
- Acknowledged
- Observed
- Unsupported
- Failed
- Timed out

---

# 9. Local Automation API

The daemon must expose a local API.

Default binding:

`127.0.0.1`

The API must not listen on public network interfaces by default.

## 9.1 Example endpoints

```text
GET    /v1/status
GET    /v1/capabilities
GET    /v1/devices

POST   /v1/session/start
POST   /v1/session/stop

POST   /v1/focus/video
POST   /v1/focus/audio
POST   /v1/focus/navigation

POST   /v1/display/configure
POST   /v1/input/tap
POST   /v1/input/swipe

POST   /v1/screenshot

GET    /v1/events
```

Use JSON request and response bodies.

## 9.2 Example request

```http
POST /v1/focus/video
Content-Type: application/json

{
  "enabled": false
}
```

Example response:

```json
{
  "operationId": "op-1024",
  "status": "accepted",
  "requestedState": "off",
  "observedState": "unknown"
}
```

## 9.3 Event stream

Provide an event stream using SSE or WebSockets.

Example event:

```json
{
  "type": "focus.video.requested",
  "timestamp": "2026-10-15T13:30:00Z",
  "sessionId": "8f31c2",
  "data": {
    "enabled": false
  }
}
```

The event taxonomy must distinguish commands requested by AutoLab from state changes confirmed by the backend.

## 9.4 API requirements

- Versioned endpoints.
- Consistent error schemas.
- Request validation.
- Session identifiers.
- Operation identifiers.
- Timeouts.
- Structured logs.
- Graceful cancellation.
- Single-session command serialization where necessary.
- Loopback-only access by default.
- No unauthenticated remote exposure.

---

# 10. Backend Abstraction

This is a critical architectural requirement.

The test runner must never call the Google DHU directly.

Instead, all operations must go through a common backend interface.

Illustrative Rust API:

```rust
#[async_trait]
pub trait HeadUnitBackend: Send + Sync {
    async fn start(&mut self) -> Result<()>;
    async fn stop(&mut self) -> Result<()>;

    async fn status(&self) -> Result<HeadUnitStatus>;

    async fn capabilities(&self) -> Result<Capabilities>;

    async fn set_video_focus(
        &mut self,
        enabled: bool,
    ) -> Result<OperationResult>;

    async fn set_night_mode(
        &mut self,
        enabled: bool,
    ) -> Result<OperationResult>;

    async fn tap(
        &mut self,
        x: u32,
        y: u32,
    ) -> Result<OperationResult>;

    async fn screenshot(&mut self) -> Result<Image>;
}
```

This interface is illustrative and may be refined during implementation.

## 10.1 Capability negotiation

Not every backend will support every feature.

Each backend must advertise its capabilities.

Example:

```json
{
  "backend": "dhu",
  "capabilities": {
    "videoFocus": true,
    "audioFocus": true,
    "nightMode": true,
    "touchInput": false,
    "screenshots": true,
    "screenRecording": false,
    "vehicleSpeed": false,
    "semanticUI": false
  }
}
```

These values are examples, not verified DHU capabilities.

The implementation must populate them from validated backend behaviour.

Tests requiring unsupported capabilities must fail clearly or be explicitly skipped according to test configuration.

---

# 11. Google DHU Backend

This is the first backend to implement.

## 11.1 Discovery

Search for the DHU executable in:

1. Explicit user configuration.
2. Environment variable.
3. Android SDK installation paths.
4. Known platform-specific SDK locations.

Example:

```bash
aa config set dhu.path \
  "$HOME/Library/Android/sdk/extras/google/auto/desktop-head-unit"
```

Validate the actual executable path on the target machine.

## 11.2 Process management

The backend must:

- Launch the DHU.
- Capture stdout and stderr.
- Provide command input where supported.
- Detect process termination.
- Detect startup failure.
- Support graceful shutdown.
- Prevent orphaned processes.

## 11.3 DHU command support

Investigate the installed DHU's documented and actual command interface.

Potential operations include:

- Video focus.
- Audio focus.
- Navigation focus.
- Driving restrictions.
- Screenshots.
- Display configuration.
- Input simulation.

Do not hardcode assumptions based solely on online examples.

Validate commands against the installed version.

## 11.4 Persistent console investigation

Determine whether the DHU supports reliable interactive stdin or another console control channel.

If so, maintain a persistent session.

If not, document the limitation and implement only supported alternatives.

Do not restart the DHU for every focus command.

## 11.5 Version compatibility

Record the DHU version when available.

Introduce a compatibility layer for differences between versions.

Do not silently assume that commands behave identically across versions.

## 11.6 Diagnostic mode

Provide:

```bash
aa doctor
```

Checks should include:

- Android SDK detected.
- DHU executable found.
- DHU executable can launch.
- ADB available.
- Android device detected.
- Required forwarding configured.
- Projection connection available.
- Supported commands verified where safely testable.

The doctor command must not unexpectedly disrupt an active session.

---

# 12. Automated Testing Framework

The testing framework is AutoLab's most important long-term feature.

It must support declarative tests and eventually a Kotlin DSL.

## 12.1 YAML tests

Example:

```yaml
name: Video focus recovery

backend: dhu

requirements:
  - videoFocus
  - screenshots

steps:
  - action: focus.video
    value: off

  - action: wait
    duration: 1s

  - action: focus.video
    value: on

  - action: screenshot
    path: screenshots/restored.png
```

Execution:

```bash
aa test tests/video-focus.yaml
```

## 12.2 Test result

Example:

```text
AutoLab Test Runner

Test: Video focus recovery

PASS  Video focus off command accepted
PASS  Wait 1s
PASS  Video focus on command accepted
PASS  Screenshot captured

Result: PASSED
Duration: 2.4s
```

Important: this example validates command execution and screenshot capture, not actual navigation recovery.

A real recovery test must include an independent observation or assertion of the expected application state.

## 12.3 Supported initial steps

MVP steps:

- Focus changes.
- Waiting.
- Screenshot capture.
- Backend capability checks.
- Session status assertions.
- Connection status assertions.

Post-MVP steps:

- Tap.
- Swipe.
- Key press.
- Launch application.
- Image assertions.
- Semantic UI assertions.
- Event assertions.
- Screen recording.

## 12.4 Timeouts

Every asynchronous action must support a timeout.

Example:

```yaml
- action: waitFor
  condition:
    projectionState: connected
  timeout: 15s
```

Tests must not hang indefinitely.

## 12.5 Test isolation

The runner must:

- Track session ownership.
- Reset supported backend state before tests.
- Avoid leaking state between tests.
- Restore configurable state after tests where possible.
- Preserve diagnostics on failure.
- Handle interruption cleanly.

Do not assume that a fresh DHU session resets application state on the phone.

## 12.6 Reports

Support:

```bash
aa test tests/ --report junit
aa test tests/ --report json
```

JUnit XML output must be compatible with common CI systems.

Artifacts:

```text
test-results/
  results.xml
  results.json
  logs/
  screenshots/
  metadata.json
```

Metadata should include:

- AutoLab version.
- Backend version.
- Android device model.
- Android version.
- Android Auto version, if detectable.
- Display configuration.
- Test duration.
- Failure reason.

---

# 13. UI Assertions

A central challenge is that Android Auto projection does not automatically provide the head unit with an Espresso-style view hierarchy.

AutoLab must support multiple assertion strategies.

## 13.1 Level 1 — Image-based assertions

Compare screenshots against reference images.

Example:

```yaml
- action: assert.image
  reference: snapshots/navigation.png
  threshold: 0.95
```

Requirements:

- Configurable similarity threshold.
- Difference image generation.
- Optional ignored regions.
- Resolution-aware comparison.
- Clear failure reporting.

Avoid treating minor rendering differences as universal test failures.

This feature is post-MVP.

## 13.2 Level 2 — App-aware semantic assertions

Provide an optional Android-side test companion.

This companion may expose:

- Current screen identifier.
- Current Car App template type.
- Template title.
- Relevant application state.
- Navigation state.
- Loading state.
- Error state.
- Test-specific semantic identifiers.

Example:

```json
{
  "screen": "Navigation",
  "template": "NavigationTemplate",
  "state": {
    "navigationActive": true,
    "videoFocus": true
  }
}
```

This data must be obtained through a deliberate app-side integration, not inferred from the projection stream.

The companion must be opt-in and intended for debug/test builds.

## 13.3 Level 3 — Kotlin testing DSL

Long-term desired API:

```kotlin
@Test
fun navigationRecoversAfterFocusLoss() = autoTest {
    launchApp("com.example.app")

    onScreen("Navigation").assertVisible()

    headUnit.videoFocus(false)
    headUnit.videoFocus(true)

    onScreen("Navigation")
        .assertVisible(timeout = 5.seconds)

    screenshot("navigation-restored")
}
```

This is a target API, not an existing implementation.

The framework should communicate with AutoLab's daemon and optional app-side instrumentation.

## 13.4 AndroidX Car App integration

Investigate integration with:

`androidx.car.app:app-testing`

Use it where appropriate for template and application behaviour assertions.

Do not represent it as an end-to-end projection test framework.

The semantic adapter should complement, not replace, real projection testing.

---

# 14. macOS Application

Build a lightweight native macOS interface.

## 14.1 Main window

The main window should contain:

### Projection viewport

- Live Android Auto display.
- Resizable viewport.
- Aspect ratio preservation.
- Screenshot button.
- Fullscreen mode.
- Connection indicator.

### Control toolbar

Buttons for:

- Video focus.
- Audio focus.
- Navigation focus.
- Night mode.
- Driving restrictions.
- Screenshot.
- Reconnect.

### Device panel

Display:

- Connected device.
- Android version.
- Android Auto version where available.
- Backend.
- Projection status.
- Resolution.

### Event panel

Display timestamped events:

```text
14:32:10  Device connected
14:32:11  Projection started
14:32:15  Video focus requested: off
14:32:17  Video focus requested: on
14:32:18  Screenshot captured
```

Events must distinguish requested operations from confirmed state changes.

## 14.2 Keyboard shortcuts

Proposed defaults:

| Shortcut | Action |
|---|---|
| Cmd+Shift+V | Toggle video focus |
| Cmd+Shift+N | Toggle night mode |
| Cmd+Shift+S | Screenshot |
| Cmd+Shift+R | Reconnect |
| Cmd+L | Show logs |

All shortcuts must be configurable eventually.

## 14.3 GUI/backend separation

The SwiftUI application must use the same daemon API as the CLI.

It must not communicate directly with the DHU or native receiver.

For the DHU backend, projection may initially remain in Google's own window.

Embedding the projection into the native AutoLab window is not a requirement for Phase 1.

The independent receiver should eventually provide a proper embeddable video surface.

---

# 15. Independent Native Head Unit

This is a later milestone.

## 15.1 Objective

Replace Google's proprietary DHU with an independently controlled Android Auto receiver.

## 15.2 Required capabilities

- Establish Android Auto projection sessions.
- Receive projected video.
- Decode and display video.
- Forward touchscreen events.
- Manage focus changes.
- Handle connection and disconnection.
- Expose session state.
- Support screenshots.
- Operate without Google's DHU executable.

## 15.3 Initial transport

Prefer USB for the initial prototype, unless feasibility research demonstrates that another transport is significantly simpler and more reliable.

Wireless Android Auto can be added later.

## 15.4 Feasibility requirements

Before implementation, produce a technical spike documenting:

1. Available open-source implementations.
2. Protocol compatibility with current Android Auto.
3. macOS support.
4. Linux support.
5. USB transport requirements.
6. Video decoding requirements.
7. Focus protocol handling.
8. Licensing constraints.
9. Maintenance risks.
10. Minimum viable connection flow.

## 15.5 Language selection

Rust is preferred for the control plane.

C++ is acceptable for integrating existing projection libraries.

Use FFI when necessary.

Do not rewrite an established protocol implementation in Rust solely for language consistency.

## 15.6 Legal and licensing constraints

- Do not redistribute Google's proprietary DHU.
- Do not bundle proprietary Google binaries.
- Require users to install official SDK components themselves when using the DHU backend.
- Preserve third-party license notices.
- Review protocol implementation dependencies for license compatibility.
- Do not assume reverse-engineered protocol compatibility is guaranteed.

---

# 16. Configuration

Use a TOML configuration file.

Proposed path:

```text
~/.config/autolab/config.toml
```

Example:

```toml
[general]
backend = "dhu"
log_level = "info"

[dhu]
path = "/path/to/desktop-head-unit"

[device]
serial = "1A141FDF600AF2"

[display]
width = 1280
height = 720
dpi = 160

[daemon]
host = "127.0.0.1"
port = 8765

[testing]
default_timeout_ms = 10000
artifacts_dir = "./test-results"
```

Configuration precedence:

1. CLI arguments.
2. Environment variables.
3. Configuration file.
4. Built-in defaults.

---

# 17. Repository Structure

Recommended monorepo:

```text
autolab/
├── Cargo.toml
├── README.md
├── LICENSE
├── rust-toolchain.toml
│
├── crates/
│   ├── autolab-core/
│   ├── autolab-cli/
│   ├── autolab-daemon/
│   ├── autolab-dhu/
│   ├── autolab-native/
│   ├── autolab-test/
│   └── autolab-protocol/
│
├── apps/
│   └── macos/
│       └── AutoLab/
│
├── sdk/
│   └── kotlin/
│
├── tests/
│   ├── fixtures/
│   ├── integration/
│   └── e2e/
│
├── examples/
│   ├── video-focus.yaml
│   ├── night-mode.yaml
│   └── screenshots.yaml
│
├── docs/
│   ├── architecture.md
│   ├── cli.md
│   ├── api.md
│   ├── testing.md
│   └── backends.md
│
└── .github/
    └── workflows/
```

Only create directories and crates when they become necessary.

Avoid empty scaffolding that provides no value.

---

# 18. Implementation Milestones

## Milestone 0 — Feasibility and Discovery

**Goal:** Validate assumptions before implementing the architecture.

Tasks:

- Inspect the official DHU documentation.
- Locate an installed DHU.
- Investigate its console interface.
- Verify persistent command input.
- Test video focus changes.
- Test screenshot capture.
- Investigate process lifecycle.
- Investigate projection connectivity detection.
- Document unsupported operations.
- Evaluate whether headless operation is practical.

Deliverable:

`docs/feasibility.md`

Acceptance criteria:

- At least one focus operation is demonstrated or a documented blocker is identified.
- Backend control mechanism is understood.
- Supported capabilities are recorded.
- No unsupported functionality is assumed.

If persistent DHU control proves impossible, stop and present alternative designs rather than building a fragile workaround.

---

## Milestone 1 — Rust Core and CLI

**Goal:** Establish a working programmable DHU.

Tasks:

- Create Cargo workspace.
- Implement core backend trait.
- Implement DHU process manager.
- Implement daemon.
- Implement CLI.
- Implement status reporting.
- Implement video focus control.
- Implement screenshot capture where supported.
- Implement structured errors.
- Implement configuration loading.
- Implement `aa doctor`.

Acceptance criteria:

```bash
aa doctor
aa start
aa status
aa focus video off
aa focus video on
aa stop
```

All supported commands must operate against a real DHU session.

Tests must cover parsing, process lifecycle, and error handling.

---

## Milestone 2 — Automation API

**Goal:** Make every supported operation accessible programmatically.

Tasks:

- Implement versioned HTTP API.
- Add session management.
- Add capability discovery.
- Add focus endpoints.
- Add screenshot endpoint.
- Add event stream.
- Add operation identifiers.
- Add API integration tests.

Acceptance criteria:

- CLI uses the daemon API.
- Focus can be controlled through HTTP.
- Session state can be queried.
- Unsupported operations return structured errors.
- Multiple clients cannot corrupt session state.

---

## Milestone 3 — YAML Test Runner

**Goal:** Execute automated Android Auto tests.

Tasks:

- Define YAML schema.
- Implement schema validation.
- Implement action execution.
- Implement timeout handling.
- Implement status assertions.
- Implement screenshot artifacts.
- Implement JSON reports.
- Implement JUnit XML reports.
- Implement failure diagnostics.

Acceptance criteria:

```bash
aa test examples/video-focus.yaml
```

Produces a deterministic result and a machine-readable report.

At least one integration test must run against a real connected Android Auto device.

Unit tests must use a mock backend so CI can run without hardware.

---

## Milestone 4 — macOS GUI

**Goal:** Provide a native control interface.

Tasks:

- Create SwiftUI application.
- Connect to daemon API.
- Display session status.
- Add focus controls.
- Add device selection.
- Add event log.
- Add screenshot control.
- Add keyboard shortcuts.
- Handle daemon disconnection.

Acceptance criteria:

- Buttons trigger the same operations as CLI commands.
- State updates correctly.
- Unsupported controls are disabled.
- GUI does not contain backend-specific business logic.

---

## Milestone 5 — End-to-End UI Testing

**Goal:** Introduce meaningful application assertions.

Tasks:

- Implement screenshot comparisons.
- Implement visual diff artifacts.
- Implement optional app-aware semantic adapter.
- Define semantic assertion protocol.
- Add test retry and wait-for mechanisms.
- Provide sample navigation regression tests.

Acceptance criteria:

A test can:

1. Establish projection.
2. Confirm the navigation screen is active using a supported assertion mechanism.
3. Disable video focus.
4. Restore video focus.
5. Wait for navigation recovery.
6. Assert the expected state.
7. Produce diagnostics on failure.

This milestone must verify actual application behaviour, not merely successful command submission.

---

## Milestone 6 — Independent Receiver

**Goal:** Eliminate the dependency on Google's DHU.

Tasks:

- Complete native receiver feasibility spike.
- Select protocol implementation.
- Establish projection connection.
- Decode and display video.
- Implement input forwarding.
- Implement focus management.
- Integrate with AutoLab backend interface.
- Add backend conformance tests.

Acceptance criteria:

- Android Auto projects to AutoLab without Google's DHU running.
- Focus can be controlled programmatically.
- Screenshots can be captured.
- Existing tests run unchanged when selecting the native backend, provided required capabilities are supported.

---

# 19. Testing Strategy

## 19.1 Unit tests

Test:

- CLI parsing.
- Configuration precedence.
- Backend capability checks.
- State transitions.
- Error mapping.
- Test schema parsing.
- Timeout behaviour.
- Report generation.

Use mocked backend implementations.

## 19.2 Integration tests

Test:

- Daemon API.
- CLI-to-daemon communication.
- Backend process lifecycle.
- Command serialization.
- Event delivery.
- Screenshot persistence.
- Graceful shutdown.

## 19.3 Hardware-in-the-loop tests

Use a real Android device connected to a development machine.

Validate:

- Projection establishment.
- Video focus changes.
- Disconnection recovery.
- Screenshot capture.
- Repeated test execution.
- Device selection.

Hardware tests must be separate from ordinary CI unit tests.

## 19.4 Backend conformance tests

Every backend must pass a common suite validating:

- Lifecycle semantics.
- Status reporting.
- Capability discovery.
- Error handling.
- Cancellation.
- Supported focus operations.
- Screenshot contracts.

Capability-dependent tests may be skipped only when the backend explicitly reports the capability as unsupported.

---

# 20. Reliability Requirements

AutoLab must be suitable for repeated automated execution.

Requirements:

- No orphaned DHU processes.
- No indefinite waits.
- No silent failures.
- No unbounded retry loops.
- No implicit global desktop input injection.
- No stale session state reported as current.
- No successful test result without completed assertions.
- Diagnostic artifacts on failure.
- Clean handling of Ctrl+C and termination signals.

A disconnected device must never be reported as connected solely because ADB still lists it.

---

# 21. Security Requirements

- Bind the daemon to loopback by default.
- Do not expose remote control without explicit configuration and authentication.
- Validate file output paths.
- Avoid shell command injection.
- Launch subprocesses with structured argument arrays.
- Do not log credentials or sensitive device data.
- Restrict access to local daemon control where practical.
- Do not silently enable ADB over a network.
- Do not install applications or modify devices without an explicit operation.

---

# 22. Documentation Requirements

The project must include:

### README.md

- What AutoLab is.
- Why it exists.
- Installation.
- Quick start.
- Supported platforms.
- Supported backends.
- Example commands.
- Known limitations.

### docs/architecture.md

- Component diagram.
- Backend abstraction.
- Daemon design.
- Event model.
- State ownership.

### docs/cli.md

Complete CLI reference.

### docs/api.md

HTTP API contract and examples.

### docs/testing.md

YAML test authoring and execution.

### docs/backends.md

Backend capabilities and limitations.

### docs/feasibility.md

Verified DHU behaviour and native receiver research.

Documentation must reflect implemented behaviour, not aspirational features.

---

# 23. Success Metrics

## MVP success

AutoLab is successful at MVP when a developer can:

1. Install the CLI.
2. Detect their DHU installation.
3. Start a DHU session.
4. Connect an Android device.
5. Toggle video focus using the CLI.
6. Capture a screenshot where supported.
7. Run a YAML automation test.
8. Receive a meaningful pass/fail result.
9. Inspect generated artifacts.
10. Stop the session cleanly.

## Long-term success

AutoLab is successful as a complete product when:

- Developers can run real Android Auto UI regression tests.
- Tests work across supported head unit backends.
- Tests can run in CI.
- A native receiver works without Google's DHU.
- The macOS application provides full programmatic control.
- Application assertions can be performed without fragile coordinate-only tests.

---

# 24. Definition of Done

A milestone is complete only when:

- Functionality is implemented.
- Unit tests pass.
- Integration tests pass where applicable.
- Error cases are handled.
- Documentation is updated.
- CLI help is accurate.
- No unsupported feature is presented as working.
- No critical TODOs remain in the milestone's core path.
- Relevant behaviour has been manually validated against a real device when required.

Mock tests alone are insufficient evidence that DHU integration works.

---

# 25. Agent Implementation Instructions

The following instructions are intended for the coding agent implementing this project.

## General approach

You are building AutoLab, an open-source programmable Android Auto head unit and testing framework.

Prioritise working software over speculative abstractions.

Follow these rules:

1. Read the entire PRD before implementing.
2. Start with Milestone 0.
3. Investigate the installed DHU before making assumptions.
4. Document verified capabilities.
5. Implement one milestone at a time.
6. Keep the Rust core independent of the GUI.
7. Keep the testing framework independent of head unit backends.
8. Write tests for core functionality.
9. Use idiomatic Rust.
10. Prefer simple, maintainable architecture.
11. Do not introduce unnecessary dependencies.
12. Do not build the native receiver before proving the automation framework.
13. Do not claim that a feature works without testing it.
14. Do not confuse command acceptance with observed device state.
15. Update documentation as implementation progresses.

## Development workflow

For each milestone:

1. Review requirements.
2. Identify technical uncertainties.
3. Implement the smallest working solution.
4. Write tests.
5. Run tests.
6. Validate relevant real-device behaviour.
7. Update documentation.
8. Summarise completed work and remaining limitations.

## Important architectural constraints

- The CLI must use the same API as the GUI.
- The test runner must use the core backend abstraction.
- No direct DHU calls from tests.
- No direct DHU calls from SwiftUI.
- No hardcoded device serial numbers.
- No hardcoded SDK paths.
- No unbounded waits.
- No unsupported success claims.
- No dependence on proprietary DHU binaries being redistributed.

## First implementation task

**Begin with Milestone 0: DHU feasibility.**

Investigate whether Google's Desktop Head Unit can be controlled through a persistent console session.

Specifically verify:

- How the DHU is launched.
- How commands are submitted.
- Whether `focus video off` and `focus video on` work.
- Whether screenshots can be captured programmatically.
- Whether the DHU can be controlled without restarting it.
- Whether projection connection state can be observed.
- Which capabilities are available for the installed version.

Produce `docs/feasibility.md`.

If a working DHU installation and Android device are available, demonstrate a small end-to-end proof of concept that toggles video focus.

If the required hardware or software is unavailable, implement only environment-independent discovery and tests, clearly mark hardware validation as pending, and explain exactly what is needed.

After the feasibility work, proceed to Milestone 1 only if the chosen control approach is viable.

**The first meaningful deliverable is a working CLI that can control video focus on an existing Android Auto DHU session.**

Everything else builds on that foundation.

---

# 26. Future Enhancements

These features are intentionally outside the initial scope but should remain architecturally possible.

- Visual test recorder.
- Test generation from recorded interactions.
- Multiple device sessions.
- Wireless Android Auto.
- Android Auto version matrix testing.
- Automatic screenshot baseline management.
- Interactive timeline of focus and lifecycle events.
- Video recording and replay.
- Network condition simulation.
- GPS and route simulation.
- Vehicle sensor simulation.
- GitHub Actions integration.
- Android Studio plugin.
- VS Code extension.
- Test result dashboard.
- Remote device execution.
- Performance metrics.
- Frame-rate monitoring.
- Projection latency analysis.
- Memory and resource diagnostics.
- Test flakiness detection.
- AI-assisted test generation.

---

# 27. Final Product Vision

AutoLab should eventually make Android Auto testing feel as natural as testing an ordinary Android application.

A developer should be able to write:

```kotlin
@Test
fun navigationSurvivesFocusLoss() = autoTest {
    launchApp("com.example.app")

    onScreen("Navigation").assertVisible()

    headUnit.videoFocus(false)
    headUnit.videoFocus(true)

    onScreen("Navigation")
        .assertVisible(timeout = 5.seconds)
}
```

And execute:

```bash
aa test
```

Without manually operating a head unit, navigating a proprietary console, or visually inspecting the result.

**AutoLab's ultimate purpose is to make Android Auto projection programmable, observable, reproducible, and testable.**

