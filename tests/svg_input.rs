use std::process::Command;

const SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 100">
  <path d="M 5 5 H 195 V 95 H 5 Z" fill="none" stroke="black"/>
  <text x="20" y="40" font-size="12">Room &amp; stairs</text>
</svg>"#;

/* REQ-CLI-015: SVG renders a PNG with its viewport and native text layer. */
#[test]
fn svg_renders_with_native_text() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("plan.SVG");
    let out = dir.path().join("out");
    std::fs::write(&input, SVG).unwrap();
    let run = Command::new(env!("CARGO_BIN_EXE_dwg2png"))
        .arg(dir.path())
        .arg("--out")
        .arg(&out)
        .args(["--size", "512", "--no-compare"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(
        run.status.success() && stderr.contains("1/1 files ok"),
        "{stderr}"
    );
    let image = tiny_skia::Pixmap::decode_png(&std::fs::read(out.join("img/f00-p00.png")).unwrap())
        .unwrap();
    assert_eq!((image.width(), image.height()), (512, 256));
    let labels: serde_json::Value =
        serde_json::from_slice(&std::fs::read(out.join("labels/f00-p00.json")).unwrap()).unwrap();
    assert_eq!(labels["labels"][0]["text"], "Room & stairs");
    assert!(labels["labels"][0]["x"].as_f64().unwrap() > 0.0);
}

/* REQ-CLI-016: SVG export keeps page geometry as vectors and keeps the PNG. */
#[test]
fn svg_export_round_trips_the_viewport() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("plan.svg");
    let out = dir.path().join("out");
    std::fs::write(&input, SVG).unwrap();
    let run = Command::new(env!("CARGO_BIN_EXE_dwg2png"))
        .arg(&input)
        .arg("--out")
        .arg(&out)
        .args(["--size", "512", "--svg", "--no-compare"])
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let svg = std::fs::read_to_string(out.join("img/f00-p00.svg")).unwrap();
    assert!(svg.contains("<path"));
    assert!(!svg.contains("<image"));
    let tree = resvg::usvg::Tree::from_str(&svg, &Default::default()).unwrap();
    let mut rendered = tiny_skia::Pixmap::new(512, 256).unwrap();
    resvg::render(
        &tree,
        tiny_skia::Transform::identity(),
        &mut rendered.as_mut(),
    );
    let png = tiny_skia::Pixmap::load_png(out.join("img/f00-p00.png")).unwrap();
    assert_eq!(rendered, png);
}

/* REQ-CLI-017: external image references and scripts are dropped from exported SVG. */
#[test]
fn external_svg_content_is_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("unsafe.svg");
    let out = dir.path().join("out");
    let referenced = dir.path().join("local.svg");
    std::fs::write(&referenced, SVG).unwrap();
    let source = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100" onload="alert(1)">
        <rect width="100" height="100" fill="white"/>
        <image href="{}" width="100" height="100"/>
        <script>alert(1)</script></svg>"#,
        referenced.display()
    );
    std::fs::write(&input, source).unwrap();
    let run = Command::new(env!("CARGO_BIN_EXE_dwg2png"))
        .arg(&input)
        .arg("--out")
        .arg(&out)
        .args(["--svg", "--size", "512", "--no-compare"])
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let svg = std::fs::read_to_string(out.join("img/f00-p00.svg")).unwrap();
    for marker in ["<image", "<script", "onload=", "local.svg"] {
        assert!(!svg.contains(marker), "{svg}");
    }
    let png = tiny_skia::Pixmap::load_png(out.join("img/f00-p00.png")).unwrap();
    assert!(png
        .pixels()
        .iter()
        .all(|p| *p == tiny_skia::PremultipliedColorU8::from_rgba(255, 255, 255, 255).unwrap()));
}
