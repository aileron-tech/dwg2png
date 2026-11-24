//! Text as a reader sees it: control codes decoded, formatting codes removed,
//! invisible attributes kept out of the picture and the label layer.

use std::path::Path;
use std::process::Command;

use acadrust::entities::{EntityType, Line, MText, Text};
use acadrust::types::Vector3;
use acadrust::{CadDocument, DwgWriter, DxfVersion, DxfWriter};

fn drawing() -> CadDocument {
    let mut doc = CadDocument::with_version(DxfVersion::AC1032);
    doc.add_entity(EntityType::Line(Line::from_coords(0.0, 0.0, 0.0, 400.0, 300.0, 0.0))).unwrap();
    let text = |value: &str, y: f64| EntityType::Text(Text::with_value(value, Vector3::new(20.0, y, 0.0)).with_height(10.0));
    doc.add_entity(text("%%uMASTER BEDROOM", 250.0)).unwrap();
    doc.add_entity(text("SLOPE 45%%d", 220.0)).unwrap();
    let mtext = MText::with_value("\\pi-12,l12,t12;GENERAL NOTES\\PELEV. 10\\S1/2;\\PBRACING @ 45%%d", Vector3::new(20.0, 180.0, 0.0)).with_height(10.0);
    doc.add_entity(EntityType::MText(mtext)).unwrap();
    doc
}

/// A hand-written DXF: a block inserted once, with a visible attribute (TAG) and an
/// invisible one (MANUFACTURER, attribute flag 1).
const WINDOW_TAG_DXF: &str = "0\nSECTION\n2\nBLOCKS\n\
0\nBLOCK\n8\n0\n2\nWINDOW\n70\n2\n10\n0\n20\n0\n30\n0\n\
0\nLINE\n8\n0\n10\n0\n20\n0\n11\n40\n21\n0\n\
0\nENDBLK\n0\nENDSEC\n\
0\nSECTION\n2\nENTITIES\n\
0\nLINE\n8\n0\n10\n0\n20\n0\n11\n300\n21\n200\n\
0\nINSERT\n8\n0\n66\n1\n2\nWINDOW\n10\n100\n20\n100\n\
0\nATTRIB\n8\n0\n10\n100\n20\n120\n40\n8\n1\nW-101\n2\nTAG\n70\n0\n\
0\nATTRIB\n8\n0\n10\n100\n20\n140\n40\n8\n1\nANDERSEN\n2\nMANUFACTURER\n70\n1\n\
0\nSEQEND\n0\nENDSEC\n0\nEOF\n";

fn render(input: &Path, out: &Path) -> (Vec<String>, serde_json::Value) {
    let run = Command::new(env!("CARGO_BIN_EXE_dwg2png"))
        .arg(input)
        .args(["--out", out.to_str().unwrap(), "--size", "1024", "--no-compare"])
        .output()
        .unwrap();
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    let read = |p: &Path| -> serde_json::Value { serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap() };
    let mut labels: Vec<String> = read(&out.join("labels/f00-p00.json"))["labels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["text"].as_str().unwrap().to_string())
        .collect();
    labels.sort();
    (labels, read(&out.join("json/f00.json")))
}

const EXPECTED: [&str; 5] = ["BRACING @ 45\u{b0}", "ELEV. 10 1/2", "GENERAL NOTES", "MASTER BEDROOM", "SLOPE 45\u{b0}"];

/* REQ-CLI-030: labels of a DWG carry the text a reader sees, without control or format codes */
#[test]
fn dwg_labels_are_clean_text() {
    let dir = tempfile::tempdir().unwrap();
    let dwg = dir.path().join("text.dwg");
    DwgWriter::write_to_file(&dwg, &drawing()).unwrap();
    let (labels, _) = render(&dwg, &dir.path().join("out"));
    assert_eq!(labels, EXPECTED);
}

/* REQ-CLI-031: the same holds for a DXF */
#[test]
fn dxf_labels_are_clean_text() {
    let dir = tempfile::tempdir().unwrap();
    let dxf = dir.path().join("text.dxf");
    DxfWriter::new(&drawing()).write_to_file(&dxf).unwrap();
    let (labels, _) = render(&dxf, &dir.path().join("out"));
    assert_eq!(labels, EXPECTED);
}

/* REQ-CLI-032: an invisible attribute is not a label but stays in the index */
#[test]
fn invisible_attribute_is_data_not_a_label() {
    let dir = tempfile::tempdir().unwrap();
    let dxf = dir.path().join("window.dxf");
    std::fs::write(&dxf, WINDOW_TAG_DXF).unwrap();
    let (labels, index) = render(&dxf, &dir.path().join("out"));
    assert_eq!(labels, ["W-101"], "only the visible attribute is a label");
    let attributes = index["attributes"].as_array().unwrap();
    let value_of = |tag: &str| attributes.iter().find(|a| a["tag"] == tag).map(|a| a["value"].as_str().unwrap().to_string());
    assert_eq!(value_of("MANUFACTURER").as_deref(), Some("ANDERSEN"));
    assert_eq!(value_of("TAG").as_deref(), Some("W-101"));
}
