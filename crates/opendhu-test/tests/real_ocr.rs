//! Captured projected UI regressions. Requires ffmpeg and English Tesseract on PATH.
use opendhu_test::screen;

#[test]
#[ignore = "requires ffmpeg and English Tesseract; run explicitly with --ignored"]
fn recognizes_dimmed_robotaxi_status_from_a_real_projected_frame() {
    let dir = std::env::temp_dir().join(format!("aa-real-ocr-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let image = dir.join("robotaxi.png");
    std::fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/robotaxi-accepting-jobs.png"
        ),
        &image,
    )
    .unwrap();
    let observed = screen::recognize(&image, || false).unwrap();
    let found = screen::locate(&observed, "Accepting jobs");
    assert!(
        found.is_some(),
        "dimmed status omitted from projected OCR: {}",
        screen::text(&observed)
    );
    let (x, y) = found.unwrap();
    assert!(
        (150..250).contains(&x) && (90..130).contains(&y),
        "status bounds must remain in the original 800x480 image: {x},{y}"
    );
    std::fs::remove_dir_all(dir).unwrap();
}
