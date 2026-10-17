//! Runner input crosses the real CLI, daemon HTTP API, and native child protocol.
mod common;

use common::*;

#[test]
fn yaml_runner_forwards_tap_swipe_and_key_to_native_session() {
    let sandbox = Sandbox::new();
    let fake = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../opendhu-native/tests/fixtures/fake-native-hu.sh"
    );
    let started = sandbox.opendhu_env(
        &[
            "start",
            "--backend",
            "native",
            "--serial",
            "FAKE0001",
            "--native-tcp",
            "127.0.0.1:15277",
        ],
        &[("OPENDHU_NATIVE_HU", fake)],
    );
    assert_eq!(code(&started), 0, "{}", text(&started));
    let test = sandbox.scratch("input.yaml");
    std::fs::write(
        &test,
        "name: Projected inputs\nbackend: native\nrequirements: [touchInput]\nsteps:\n  - action: tap\n    x: 10\n    y: 20\n  - action: swipe\n    from: [10, 20]\n    to: [100, 20]\n    duration: 500ms\n  - action: key\n    value: back\n    long: true\n",
    ).unwrap();
    let artifacts = sandbox.scratch("artifacts");
    let output = sandbox.opendhu(&[
        "test",
        test.to_str().unwrap(),
        "--headless",
        "--artifacts-dir",
        artifacts.to_str().unwrap(),
        "--report",
        "json",
    ]);
    assert_eq!(code(&output), 0, "{}", text(&output));
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(artifacts.join("results.json")).unwrap()).unwrap();
    assert_eq!(report["tests"][0]["outcome"], "passed");
    let steps = report["tests"][0]["steps"].as_array().unwrap();
    for (index, action) in ["tap", "swipe", "key"].iter().enumerate() {
        let step = &steps[index + 1];
        assert_eq!(step["action"], *action);
        assert_eq!(step["outcome"], "passed");
        assert!(
            step["detail"]
                .as_str()
                .unwrap()
                .contains("not acknowledged by the phone")
        );
    }
    // A failed command must remain a failed runner result; it cannot be treated as a sent input.
    assert_eq!(code(&sandbox.opendhu(&["focus", "video", "off"])), 0);
    let refused = sandbox.opendhu(&[
        "test",
        test.to_str().unwrap(),
        "--headless",
        "--artifacts-dir",
        artifacts.to_str().unwrap(),
    ]);
    assert_ne!(code(&refused), 0);
    assert!(
        text(&refused).contains("not projecting"),
        "{}",
        text(&refused)
    );
}
