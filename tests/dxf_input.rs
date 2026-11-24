//! DXF input: the renderer reads DXF as well as DWG, choosing the reader from
//! the content of the file.

use std::path::Path;
use std::process::Command;

use acadrust::entities::{EntityType, Line, Text};
use acadrust::tables::Layer;
use acadrust::types::{Color, Vector3};
use acadrust::{CadDocument, DwgWriter, DxfVersion, DxfWriter};
use dwg2png::{detect_format, CadFormat};

const LABELS: [&str; 3] = ["ROOM 101", "ROOM 102", "Stair A"];

fn drawing() -> CadDocument {
    let mut doc = CadDocument::with_version(DxfVersion::AC1032);
    let mut layer = Layer::with_color("ROOMS", Color::RED);
    layer.handle = doc.allocate_handle();
    doc.layers.add(layer).unwrap();
    for (x1, y1, x2, y2) in [(0.0, 0.0, 300.0, 0.0), (300.0, 0.0, 300.0, 200.0), (300.0, 200.0, 0.0, 200.0), (0.0, 200.0, 0.0, 0.0)] {
        doc.add_entity(EntityType::Line(Line::from_coords(x1, y1, 0.0, x2, y2, 0.0))).unwrap();
    }
    for (i, label) in LABELS.iter().enumerate() {
        let mut text = Text::with_value(*label, Vector3::new(20.0, 30.0 + 50.0 * i as f64, 0.0)).with_height(12.0);
        text.common.layer = "ROOMS".to_string();
        doc.add_entity(EntityType::Text(text)).unwrap();
    }
    doc
}

/// Run the CLI on `input` and return (stderr, labels of the Model page, index JSON).
fn render(input: &Path, out: &Path) -> (String, Vec<(String, String)>, serde_json::Value) {
    let run = Command::new(env!("CARGO_BIN_EXE_dwg2png"))
        .arg(input)
        .args(["--out", out.to_str().unwrap(), "--size", "1024", "--no-compare"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&run.stderr).into_owned();
    assert!(run.status.success(), "{stderr}");
    let read = |p: &Path| -> serde_json::Value { serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap() };
    let labels = read(&out.join("labels/f00-p00.json"))["labels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| (l["text"].as_str().unwrap().to_string(), l["layer"].as_str().unwrap().to_string()))
        .collect();
    (stderr, labels, read(&out.join("json/f00.json")))
}

/* REQ-CLI-010: a DXF file renders to pages with its labels and layers, like a DWG */
#[test]
fn dxf_renders_with_labels_and_layers() {
    let dir = tempfile::tempdir().unwrap();
    let dxf = dir.path().join("rooms.dxf");
    DxfWriter::new(&drawing()).write_to_file(&dxf).unwrap();
    let (stderr, labels, index) = render(&dxf, &dir.path().join("out"));
    assert!(stderr.contains("1/1 files ok"), "{stderr}");
    let mut texts: Vec<&str> = labels.iter().map(|(t, _)| t.as_str()).collect();
    texts.sort();
    assert_eq!(texts, ["ROOM 101", "ROOM 102", "Stair A"]);
    assert!(labels.iter().all(|(_, layer)| layer == "ROOMS"));
    assert!(index["layers"].as_array().unwrap().iter().any(|l| l == "ROOMS"));
    assert!(dir.path().join("out/img/f00-p00.png").is_file());
}

/* REQ-CLI-011: the same drawing gives the same labels from DXF and from DWG */
#[test]
fn dxf_and_dwg_give_the_same_labels() {
    let dir = tempfile::tempdir().unwrap();
    let (dxf, dwg) = (dir.path().join("a.dxf"), dir.path().join("b.dwg"));
    DxfWriter::new(&drawing()).write_to_file(&dxf).unwrap();
    DwgWriter::write_to_file(&dwg, &drawing()).unwrap();
    let (_, mut from_dxf, _) = render(&dxf, &dir.path().join("out-dxf"));
    let (_, mut from_dwg, _) = render(&dwg, &dir.path().join("out-dwg"));
    from_dxf.sort();
    from_dwg.sort();
    assert_eq!(from_dxf, from_dwg);
}

/* REQ-CLI-012: a folder is searched for .dwg and .dxf files, in any letter case */
#[test]
fn folders_are_searched_for_both_formats() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in");
    std::fs::create_dir(&input).unwrap();
    DxfWriter::new(&drawing()).write_to_file(input.join("one.DXF")).unwrap();
    DwgWriter::write_to_file(input.join("two.dwg"), &drawing()).unwrap();
    std::fs::write(input.join("notes.txt"), "not a drawing").unwrap();
    let (stderr, _, _) = render(&input, &dir.path().join("out"));
    assert!(stderr.contains("found 2 drawing(s)"), "{stderr}");
    assert!(stderr.contains("2/2 files ok"), "{stderr}");
}

/* REQ-CLI-013: the format is read from the content, so a DXF named .dwg still parses */
#[test]
fn format_follows_content_not_extension() {
    let dir = tempfile::tempdir().unwrap();
    let misnamed = dir.path().join("really-a-dxf.dwg");
    DxfWriter::new(&drawing()).write_to_file(&misnamed).unwrap();
    let (_, labels, _) = render(&misnamed, &dir.path().join("out"));
    assert_eq!(labels.len(), 3);
}

/* REQ-CLI-014: the format of a file is recognised from its first bytes */
#[test]
fn formats_are_detected_from_the_first_bytes() {
    assert_eq!(detect_format(b"AC1032\0\0\0"), Some(CadFormat::Dwg));
    assert_eq!(detect_format(b"  0\r\nSECTION\r\n  2\r\nHEADER\r\n"), Some(CadFormat::Dxf));
    assert_eq!(detect_format(b"0\nSECTION\n2\nENTITIES\n"), Some(CadFormat::Dxf));
    assert_eq!(detect_format(b"999\ncreated by a CAD program\n0\nSECTION\n"), Some(CadFormat::Dxf));
    assert_eq!(detect_format(b"AutoCAD Binary DXF\r\n\x1a\0"), Some(CadFormat::Dxf));
    assert_eq!(detect_format(b"<html><body>Sign in</body></html>"), None);
    assert_eq!(detect_format(b"%PDF-1.7"), None);
    assert_eq!(detect_format(b""), None);
}

/* REQ-CLI-018: DWG and DXF export every page as SVG whose reimport produces the same PNG pixels. */
#[test]
fn cad_svg_export_matches_png() {
    let root = tempfile::tempdir().unwrap();
    let dxf = root.path().join("rooms.dxf");
    let dwg = root.path().join("rooms.dwg");
    DxfWriter::new(&drawing()).write_to_file(&dxf).unwrap();
    DwgWriter::write_to_file(&dwg, &drawing()).unwrap();
    for (input, name) in [(dxf, "dxf"), (dwg, "dwg")] {
        let out = root.path().join(name);
        let run = Command::new(env!("CARGO_BIN_EXE_dwg2png"))
            .arg(&input)
            .arg("--out")
            .arg(&out)
            .args(["--svg", "--size", "1024", "--no-compare"])
            .output()
            .unwrap();
        assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
        let pages = std::fs::read_dir(out.join("img")).unwrap().flatten().filter(|p| p.path().extension().is_some_and(|e| e == "png"));
        let mut count = 0;
        for page in pages {
            let original = page.path();
            let vector = original.with_extension("svg");
            let svg = std::fs::read_to_string(&vector).unwrap();
            assert!(svg.contains("<path") && !svg.contains("<image"), "{vector:?}");
            let replay = root.path().join(format!("{name}-replay"));
            // Keep the page viewport when the CAD layout is a placeholder.
            let pixmap = tiny_skia::Pixmap::load_png(&original).unwrap();
            let size = pixmap.width().max(pixmap.height()).to_string();
            render_svg_png(&vector, &replay, &size);
            let replayed = tiny_skia::Pixmap::load_png(replay.join("img/f00-p00.png")).unwrap();
            assert_eq!((pixmap.width(), pixmap.height()), (replayed.width(), replayed.height()), "{vector:?}");
            let differing = pixmap.pixels().iter().zip(replayed.pixels()).filter(|(a,b)| a != b).count();
            assert_eq!(differing, 0, "{vector:?}");
            count += 1;
        }
        assert!(count > 0);
    }
}

fn render_svg_png(input: &Path, out: &Path, size: &str) {
    let run = Command::new(env!("CARGO_BIN_EXE_dwg2png"))
        .arg(input)
        .arg("--out")
        .arg(out)
        .args(["--size", size, "--no-compare"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(run.status.success() && stderr.contains("1/1 files ok"), "{stderr}");
}
