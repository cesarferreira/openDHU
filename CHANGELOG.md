# Changelog

All notable changes to this project will be documented in this file.

## [0.4.0] - 2026-11-09

### 💼 Other

- Add aa skill install: an agent skill embedded in the binary
## [0.3.0] - 2026-11-07

### 💼 Other

- Load project head-unit config and use nextest consistently
- Use opendhu.yaml as the single project config
- Separate managed test and interactive display resolutions
## [0.2.0] - 2026-11-03

### 🚀 Features

- Add DHU video focus proof of concept and feasibility doc
- Add autolab screenshot and wait for the phone on start
- Add YAML test runner and adb reverse tunnels on start
- Stop autolab test cleanly on Ctrl+C and SIGTERM
- Re-apply adb reverse tunnels the phone drops mid-session
- Add loopback HTTP daemon that owns the session and serves the CLI
- Add TOML config file, desktop window, event-stream client and unplug findings
- Add a native Android Auto backend that reports what the phone says
- Add tap and swipe on the native backend, and a backend choice in the GUI
- Live view of the phone's screen in the GUI, fed by a new GET /v1/video
- Car buttons, and fixes that make the live view show what the phone shows
- Stream and record the phone's audio, and a feature guide README
- Load an optional native head-unit certificate
- Support 720p and 1080p on the native backend, not just 800x480
- Add headless Android Auto emulator test suites
- Support production backend journeys in test suites
- Stream test progress and show the head unit on demand
- Add interactive test progress and named suite selection

### 🐛 Bug Fixes

- GUI says head unit display, and reports a native focus change by what the phone did
- The live view no longer dies after 10 seconds
- Dim the live view when the phone stops projecting; mark car buttons experimental
- Ffplay 9.x dropped -ac, so the GUI's audio player quit after one chunk
- Fix
- Fixed tests

### 💼 Other

- Init
- Make targets for everything AutoLab does today
- Certificates
- Rename to aa
- Progress
- Proper custom certs
- Recognize dimmed projected statuses and require launcher labels
- Encode only the final decoded frame for native screenshots
- Show the test head unit and animate progress by default
- Document default test viewer and interactive progress in README
- Add standalone Android Auto demo and black-box E2E journeys
- Write test artifacts under build and add HTML reports
- Default to native head unit and add --dhu selector
- Rename AutoLab to openDHU and add Docker test environment
- Add USB suite override and remove Docker test setup
- Updated readme
- Add suite listing and sequential all-suite execution
- Fail managed tests on app crashes and ANRs
- Add suite diagnostics and reusable suite setup
- Improve projected keyboard and enclosed control text recognition
- Run all registered suites by default with aa test
- Reuse suite emulators and fix DNS and OCR test failures
- Infer Android SDK tools and preflight test tools
- Retry head-unit settings UI dump instead of aborting
- Remove legacy config and credential fallbacks
- Make managed Android Auto runs reliable on a fresh emulator
- Add environment.preferences to set app SharedPreferences before a suite runs
- Simplify test summaries with colorful compact footers
- Simplified
- Redesign desktop GUI and fix automatic emulator connections
- Document DHU CPU and memory benchmark results
- Optimize GUI video pipeline and default to release installs
- Use GPU color conversion by default in the desktop viewer
- Fix launcher label OCR matching on USB phones
- Remove leftover AutoLab references
- Fix flaky launcher and tap handling on x86 emulators
- Emu
- Tui
- Add opendhu-tui: kitty-graphics head unit, aa tui and aa test --tui
- Make aa test --tui follow each test's daemon
- Draw each TUI frame as a new kitty image and delete the previous one
- Make the TUI usable over SSH: compress frames, send fewer and smaller
- Document aa test --tui and the terminal viewer
- Fix the macOS build and make install with a custom cargo target-dir
- Consolidate aa test all into one report and reuse a single results folder
- Add environment.build so aa test rebuilds the APK before installing
- Restyle the TUI footer as key chips and fix the no-session header during aa test
- Separate the TUI log pane from the key hints and make the log scrollable
- Show aa test --tui errors in the log pane and move the sample to android-sample
- Add step labels and print one terse line per passing step
- Speed up OCR by stopping at the first pass that answers the step
- Stop launcher OCR at its icon label and poll UI results every 250ms
- Show how long each suite setup phase took
- Reuse the previous OCR result when the screenshot is unchanged
- Read the original-size OCR pass first and shorten UI polling and launcher waits
- Skip the APK install when the device already runs the same build
- Keep the setup phase timing lines from being drawn as a spinner
- Skip the video-focus wait when the phone already reports it, and keep the head-unit server between suites
- Fix the failing and flaky tests
- Number tests, suites and steps so progress is visible
- Estimate the time left from earlier runs' durations
- Rewrite the README and move the benchmarks to docs/performance.md
- Point the GUI doc at docs/performance.md
- Remove the stale benchmark write-up
- Drop the n/total from step numbers; keep it on tests and suites
- Add phone.expect and a car-display hint to app.open failures ([#2](https://github.com/cesarferreira/openDHU/issues/2))
- Add GitHub Actions CI ([#3](https://github.com/cesarferreira/openDHU/issues/3))
- Wait for the launched app's car session to settle before the first test resets it ([#5](https://github.com/cesarferreira/openDHU/issues/5))
- Add Android Auto detection and aa setup gearhead
- Report Android Auto in aa doctor
- Add video recording and a GitHub Action for emulator test runs
- Bound the emulator boot wait in the GitHub Action
- Trim CI to save Actions minutes
- Rewrite the README around tasks
- Align the comments in the README code blocks
- Print suite artifact paths without ../ segments
- Add an app.removePreferences test step
- Explain that --usb is not auto-detected in the README
- Recover a USB phone's Android Auto host stuck on "Waiting..." after an app restart
- Also recover from the host's "unexpected error" card after an app restart
- Ask the phone for a keyframe before screenshots stop working
- Group session lifecycle commands under `aa session`
- Fix the dhu path index in the tunnels tests after the session rename
- Reset a phone left in USB accessory mode before a native USB start
- Add a Docker image that runs suites on a headless emulator
- Say Docker is not supported on macOS and Apple Silicon
- Fix the Docker image for Gradle builds and emulator reuse
- Make the Docker image run suites end to end
- Add a browser viewer: `aa web` and `aa test --web`
- Report what the phone did, not just that the daemon answered
- Send every field of a button event so the phone acts on it
- Say once that the phone does not acknowledge input
- Have the fake head unit say the phone does not acknowledge input
- Turn the rotary dial with `aa key rotate_left|rotate_right`
- Pick suites and tests from a checklist with `aa test -i`
- Run the tests with cargo nextest and make suite_cli parallel-safe
- Stop tests reaching a real adb and waiting out fake hangs
- Added release and tag

### 📚 Documentation

- Remove references to a specific company app and scripts
- Replace the stale pre-implementation PRD with a short vision note

### ⚙️ Miscellaneous Tasks

- Vendor the android-auto crate and a patched rustls-webpki
