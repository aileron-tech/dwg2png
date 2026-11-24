//! DWG, DXF and SVG ingestion with raster, vector and metadata outputs.
//!
//! Boundaries: `parse` (acadrust adapter, here) -> `pages`/`tessellate`
//! (geometry) -> `render` (raster) and `metadata` (index). `report` renders
//! results to HTML. The acadrust dependency is touched only in this file,
//! `pages`, `tessellate`, and `metadata`.

pub mod color;
pub mod metadata;
pub mod model;
pub mod pages;
pub mod render;
pub mod report;
mod svg;
pub mod tessellate;
pub mod text;

use std::panic::AssertUnwindSafe;
use std::path::Path;
use std::time::Instant;

use ab_glyph::FontVec;
use acadrust::document::CadDocument;
use acadrust::io::dwg::{DwgReadOptions, DwgReader};
use acadrust::io::dxf::{DxfReader, DxfReaderConfiguration};

use crate::metadata::{extract, IndexDoc};
use crate::pages::build_pages;
use crate::render::{render_page_with_svg, Raster, RenderOpts, RenderedLabel};

/// The drawing formats the reader understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CadFormat {
    Dwg,
    Dxf,
    Svg,
}

/// The format of a drawing, from the first bytes of the file. A DWG starts with its
/// version code ("AC" and four digits). A binary DXF starts with a fixed sentinel; a
/// text DXF starts with group code 0 and SECTION, or with a 999 comment.
pub fn detect_format(head: &[u8]) -> Option<CadFormat> {
    if head.len() >= 6 && &head[..2] == b"AC" && head[2..6].iter().all(u8::is_ascii_digit) {
        return Some(CadFormat::Dwg);
    }
    if head.starts_with(b"AutoCAD Binary DXF") {
        return Some(CadFormat::Dxf);
    }
    let text = String::from_utf8_lossy(head);
    let text = text.trim_start_matches('\u{feff}');
    if svg::has_svg_root(text) {
        return Some(CadFormat::Svg);
    }
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    match (lines.next(), lines.next()) {
        (Some("999"), _) | (Some("0"), Some("SECTION")) => Some(CadFormat::Dxf),
        _ => None,
    }
}

/// Identify the format by content, falling back to the filename extension.
fn format_of(path: &Path) -> CadFormat {
    use std::io::Read;
    let mut head = [0u8; 4096];
    let read = std::fs::File::open(path).and_then(|mut f| f.read(&mut head)).unwrap_or(0);
    detect_format(&head[..read]).unwrap_or_else(|| {
        match path.extension().and_then(|e| e.to_str()) {
            Some(ext) if ext.eq_ignore_ascii_case("svg") => CadFormat::Svg,
            Some(ext) if ext.eq_ignore_ascii_case("dxf") => CadFormat::Dxf,
            _ => CadFormat::Dwg,
        }
    })
}

/// Read a DWG or DXF file into a document. Both readers run in failsafe mode, and a
/// panic inside the parser is caught: a malformed file becomes an error, not a crash.
fn read_document(path: &Path, format: CadFormat) -> Result<CadDocument, String> {
    let read = match format {
        CadFormat::Dwg => {
            let mut reader = DwgReader::from_file_with_options(path, DwgReadOptions::failsafe())
                .map_err(|e| format!("open: {e}"))?;
            std::panic::catch_unwind(AssertUnwindSafe(move || reader.read()))
        }
        CadFormat::Dxf => {
            let reader = DxfReader::from_file(path)
                .map_err(|e| format!("open: {e}"))?
                .with_configuration(DxfReaderConfiguration { failsafe: true, default_encoding: None });
            std::panic::catch_unwind(AssertUnwindSafe(move || reader.read()))
        }
        CadFormat::Svg => return Err("SVG is not an AutoCAD document".into()),
    };
    match read {
        Ok(Ok(doc)) => Ok(doc),
        Ok(Err(e)) => Err(format!("read: {e}")),
        Err(_) => Err("read: parser panicked (caught)".to_string()),
    }
}

/// A rendered page and where its PNG was written (relative to the report).
#[derive(Debug, Clone)]
pub struct PageImage {
    pub name: String,
    pub rel_png: String,
    pub width: u32,
    pub height: u32,
    pub entity_count: usize,
    pub prim_count: usize,
    /// PNG byte size — used as a visual-richness proxy for picking comparisons.
    pub byte_len: usize,
    /// Pixel-space text layer (for the report overlay and downstream VLM use).
    pub labels: Vec<RenderedLabel>,
    /// Readable tile crops (when tiling is enabled).
    pub tiles: Vec<TileRef>,
}

/// A written tile crop, referenced from the report.
#[derive(Debug, Clone)]
pub struct TileRef {
    pub rel_png: String,
    pub col: u32,
    pub row: u32,
    pub width: u32,
    pub height: u32,
    pub label_count: usize,
}

/// Optional side-by-side comparison against a ground-truth raster.
#[derive(Debug, Clone)]
pub struct Comparison {
    pub our_rel_png: String,
    pub our_label: String,
    pub reference_rel_png: String,
    pub reference_label: String,
}

/// Outcome of processing one DWG.
#[derive(Debug, Clone)]
pub struct FileReport {
    pub source: String,
    pub display_name: String,
    pub ok: bool,
    pub error: Option<String>,
    pub index: Option<IndexDoc>,
    pub pages: Vec<PageImage>,
    pub parse_ms: u128,
    pub render_ms: u128,
    pub comparison: Option<Comparison>,
}

/// Output directories for a drawing's page previews, text layer and tiles.
///
/// Parsing and rendering are wrapped in `catch_unwind`: a malformed drawing that
/// panics the parser degrades to an error row instead of taking down a batch.
pub struct OutDirs<'a> {
    pub img_dir: &'a Path,
    pub img_rel: &'a str,
    pub labels_dir: &'a Path,
    pub tiles_dir: &'a Path,
    pub tiles_rel: &'a str,
}

/// Parse a drawing and write PNG previews, label JSON and optional tiles.
pub fn process_file(
    path: &Path,
    dirs: &OutDirs,
    font: &FontVec,
    file_idx: usize,
    opts: RenderOpts,
) -> FileReport {
    process_drawing(path, dirs, font, file_idx, opts, false)
}

/// Process a drawing and also write one SVG next to each PNG preview.
pub fn process_file_with_svg(
    path: &Path,
    dirs: &OutDirs,
    font: &FontVec,
    file_idx: usize,
    opts: RenderOpts,
) -> FileReport {
    process_drawing(path, dirs, font, file_idx, opts, true)
}

fn process_drawing(
    path: &Path,
    dirs: &OutDirs,
    font: &FontVec,
    file_idx: usize,
    opts: RenderOpts,
    export_svg: bool,
) -> FileReport {
    let display_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("?").to_string();
    let source = path.display().to_string();

    let mut report = FileReport {
        source: source.clone(),
        display_name: display_name.clone(),
        ok: false,
        error: None,
        index: None,
        pages: Vec::new(),
        parse_ms: 0,
        render_ms: 0,
        comparison: None,
    };

    let t0 = Instant::now();
    let format = format_of(path);
    if format == CadFormat::Svg {
        let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let tree = svg::read(path, Some(font))?;
            report.parse_ms = t0.elapsed().as_millis();
            let index = svg::index(&tree, &display_name, report.parse_ms);
            let counts = (index.entity_count, index.entity_count);
            report.index = Some(index);
            let tr = Instant::now();
            let rendered = svg::render(&tree, opts, export_svg)?;
            let page = write_page(dirs, &format!("f{file_idx:02}-p00"), "Model", counts, rendered);
            report.render_ms = tr.elapsed().as_millis();
            page
        }));
        record_page(&mut report, "Model", result);
    } else {
        let doc = match read_document(path, format) {
            Ok(doc) => doc,
            Err(e) => {
                report.error = Some(e);
                return report;
            }
        };
        report.parse_ms = t0.elapsed().as_millis();
        report.index = Some(extract(&doc, &display_name, report.parse_ms));

        let tr = Instant::now();
        let pages = build_pages(&doc, font);
        for (pi, page) in pages.iter().enumerate() {
            let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
                let rendered = render_page_with_svg(page, opts, export_svg)?;
                let prim_count =
                    page.prims.len() + page.overlays.iter().map(|o| o.prims.len()).sum::<usize>();
                write_page(
                    dirs,
                    &format!("f{file_idx:02}-p{pi:02}"),
                    &page.name,
                    (page.entity_count, prim_count),
                    rendered,
                )
            }));
            record_page(&mut report, &page.name, result);
        }
        report.render_ms = tr.elapsed().as_millis();
    }
    report.ok = !report.pages.is_empty();
    report
}

fn record_page(
    report: &mut FileReport,
    name: &str,
    result: std::thread::Result<Result<PageImage, String>>,
) {
    match result {
        Ok(Ok(page)) => report.pages.push(page),
        Ok(Err(error)) => report.error = Some(format!("render {name}: {error}")),
        Err(_) => report.error = Some(format!("render {name}: panicked (caught)")),
    }
}

fn write_page(
    dirs: &OutDirs,
    stem: &str,
    name: &str,
    (entity_count, prim_count): (usize, usize),
    (raster, svg): (Raster, Option<String>),
) -> Result<PageImage, String> {
    let file_name = format!("{stem}.png");
    let abs = dirs.img_dir.join(&file_name);
    std::fs::write(&abs, &raster.png).map_err(|e| format!("write {}: {e}", abs.display()))?;
    if let Some(svg) = svg {
        let path = dirs.img_dir.join(format!("{stem}.svg"));
        std::fs::write(&path, svg).map_err(|e| format!("write {}: {e}", path.display()))?;
    }

    let labels_json = serde_json::json!({
        "image": file_name,
        "page": name,
        "width": raster.width,
        "height": raster.height,
        "label_count": raster.labels.len(),
        "labels": raster.labels,
    });
    if let Ok(js) = serde_json::to_string(&labels_json) {
        let _ = std::fs::write(dirs.labels_dir.join(format!("{stem}.json")), js);
    }

    let tiles = write_tiles(dirs, stem, &raster);
    Ok(PageImage {
        name: name.into(),
        rel_png: format!("{}/{file_name}", dirs.img_rel),
        width: raster.width,
        height: raster.height,
        entity_count,
        prim_count,
        byte_len: raster.png.len(),
        labels: raster.labels,
        tiles,
    })
}

/// Write tile PNGs + a per-page tile manifest; return references for the report.
fn write_tiles(dirs: &OutDirs, stem: &str, raster: &crate::render::Raster) -> Vec<TileRef> {
    if raster.tiles.is_empty() {
        return Vec::new();
    }
    let page_dir = dirs.tiles_dir.join(stem);
    if std::fs::create_dir_all(&page_dir).is_err() {
        return Vec::new();
    }
    let mut refs = Vec::new();
    let mut manifest = Vec::new();
    for t in &raster.tiles {
        let tname = format!("r{}c{}.png", t.row, t.col);
        if std::fs::write(page_dir.join(&tname), &t.png).is_err() {
            continue;
        }
        manifest.push(serde_json::json!({
            "file": tname, "col": t.col, "row": t.row,
            "x": t.x, "y": t.y, "width": t.width, "height": t.height,
            "labels": t.labels,
        }));
        refs.push(TileRef {
            rel_png: format!("{}/{stem}/{tname}", dirs.tiles_rel),
            col: t.col,
            row: t.row,
            width: t.width,
            height: t.height,
            label_count: t.labels.len(),
        });
    }
    let manifest_json = serde_json::json!({
        "image_width": raster.width, "image_height": raster.height, "tiles": manifest,
    });
    if let Ok(js) = serde_json::to_string(&manifest_json) {
        let _ = std::fs::write(page_dir.join("tiles.json"), js);
    }
    refs
}

/// Build a metadata index without rasterizing the drawing.
pub fn index_to_json(path: &Path) -> Result<IndexDoc, String> {
    let t0 = Instant::now();
    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("?");
    let format = format_of(path);
    if format == CadFormat::Svg {
        let tree = svg::read(path, None)?;
        return Ok(svg::index(&tree, name, t0.elapsed().as_millis()));
    }
    let doc = read_document(path, format)?;
    Ok(extract(&doc, name, t0.elapsed().as_millis()))
}
