//! SVG input normalization and rasterization.
use std::collections::BTreeMap;
use std::path::Path;

use ab_glyph::FontVec;
use resvg::usvg::{self, Group, Node};
use tiny_skia::{Color, Pixmap, Transform};

use crate::metadata::{IndexDoc, LayoutMeta};
use crate::render::{finish_raster, Raster, RenderOpts, RenderedLabel};

pub(crate) fn has_svg_root(mut text: &str) -> bool {
    loop {
        text = text.trim_start();
        let terminator = if text.starts_with("<!--") {
            "-->"
        } else if text.starts_with("<?") {
            "?>"
        } else if text.starts_with("<!DOCTYPE svg") {
            ">"
        } else {
            break;
        };
        let Some((_, rest)) = text.split_once(terminator) else {
            return false;
        };
        text = rest;
    }
    let Some(tag) = text.strip_prefix('<') else {
        return false;
    };
    let Some(end) = tag.find(|c: char| c.is_ascii_whitespace() || matches!(c, '/' | '>')) else {
        return false;
    };
    tag[..end].rsplit(':').next() == Some("svg")
}

pub(crate) fn read(path: &Path, font: Option<&FontVec>) -> Result<usvg::Tree, String> {
    let source = std::fs::read_to_string(path).map_err(|e| format!("read SVG: {e}"))?;
    let mut options = usvg::Options {
        image_href_resolver: usvg::ImageHrefResolver {
            // Drawing contents cannot request files or URLs from the host.
            resolve_string: Box::new(|_, _| None),
            ..Default::default()
        },
        ..Default::default()
    };
    if let Some(font) = font {
        let db = options.fontdb_mut();
        db.load_font_data(font.as_slice().to_vec());
        let family = db
            .faces()
            .next()
            .and_then(|face| face.families.first())
            .map(|family| family.0.clone());
        if let Some(family) = family {
            db.set_serif_family(family.clone());
            db.set_sans_serif_family(family.clone());
            db.set_monospace_family(family.clone());
            options.font_family = family;
        }
    }
    let tree = usvg::Tree::from_str(&source, &options).map_err(|e| format!("parse SVG: {e}"))?;
    if tree.root().children().is_empty() {
        return Err("parse SVG: no renderable content".into());
    }
    Ok(tree)
}

pub(crate) fn render(
    tree: &usvg::Tree,
    opts: RenderOpts,
    export_svg: bool,
) -> Result<(Raster, Option<String>), String> {
    let size = tree.size();
    let scale = opts.target_long.min(opts.max_side as f32) / size.width().max(size.height());
    let width = (size.width() * scale)
        .round()
        .clamp(1.0, opts.max_side as f32) as u32;
    let height = (size.height() * scale)
        .round()
        .clamp(1.0, opts.max_side as f32) as u32;
    let vector = export_svg.then(|| export(tree, width, height, scale));
    let pixmap = match &vector {
        Some(svg) => rasterize_export(svg, width, height)?,
        None => rasterize(tree, width, height, scale)?,
    };

    let mut labels = Vec::new();
    visit(tree.root(), &mut |node| {
        if let Node::Text(text) = node {
            let rect = text.abs_bounding_box();
            let color = text
                .chunks()
                .first()
                .and_then(|chunk| chunk.spans().first())
                .and_then(|span| span.fill())
                .and_then(|fill| match fill.paint() {
                    usvg::Paint::Color(c) => {
                        Some(format!("#{:02x}{:02x}{:02x}", c.red, c.green, c.blue))
                    }
                    _ => None,
                })
                .unwrap_or_default();
            let transform = text.abs_transform();
            labels.push(RenderedLabel {
                text: text_value(text),
                x: rect.left() * scale,
                y: rect.bottom() * scale,
                w: rect.width() * scale,
                h: rect.height() * scale,
                rot: transform.ky.atan2(transform.sx).to_degrees(),
                color,
                layer: String::new(),
            });
        }
    });
    finish_raster(pixmap, labels, opts).map(|raster| (raster, vector))
}

pub(crate) fn index(tree: &usvg::Tree, name: &str, parse_ms: u128) -> IndexDoc {
    let mut histogram = BTreeMap::new();
    let mut text_samples = Vec::new();
    visit(tree.root(), &mut |node| {
        let kind = match node {
            Node::Group(_) => return,
            Node::Path(_) => "Path",
            Node::Image(_) => "Image",
            Node::Text(text) => {
                if text_samples.len() < 60 {
                    text_samples.push(text_value(text));
                }
                "Text"
            }
        };
        *histogram.entry(kind.into()).or_insert(0) += 1;
    });
    let entity_count = histogram.values().sum();
    IndexDoc {
        file: name.into(),
        version: "SVG".into(),
        parse_ms,
        entity_count,
        layer_count: 0,
        layers: Vec::new(),
        blocks: Vec::new(),
        layouts: vec![LayoutMeta {
            name: "Model".into(),
            paper_w: 0.0,
            paper_h: 0.0,
            entity_count,
        }],
        histogram,
        text_samples,
        attributes: Vec::new(),
        notifications: Vec::new(),
    }
}

fn visit(group: &Group, visitor: &mut impl FnMut(&Node)) {
    for node in group.children() {
        visitor(node);
        if let Node::Group(group) = node {
            visit(group, visitor);
        }
    }
}

fn text_value(text: &usvg::Text) -> String {
    text.chunks().iter().map(|chunk| chunk.text()).collect()
}

pub(crate) fn start(width: u32, height: u32, background: &str) -> String {
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}"><rect width="100%" height="100%" fill="{background}"/>"#
    )
}

pub(crate) fn blank(width: u32, height: u32, background: &str) -> String {
    start(width, height, background) + "</svg>"
}

fn rasterize(tree: &usvg::Tree, width: u32, height: u32, scale: f32) -> Result<Pixmap, String> {
    let mut pixmap = Pixmap::new(width, height).ok_or("failed to allocate SVG pixmap")?;
    pixmap.fill(Color::WHITE);
    resvg::render(
        tree,
        Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    Ok(pixmap)
}

/// Render the exported geometry so that reopening the SVG at its output size
/// yields the same pixels as the accompanying PNG.
pub(crate) fn rasterize_export(svg: &str, width: u32, height: u32) -> Result<Pixmap, String> {
    let tree = usvg::Tree::from_str(svg, &Default::default())
        .map_err(|e| format!("parse exported SVG: {e}"))?;
    rasterize(&tree, width, height, 1.0)
}

fn export(tree: &usvg::Tree, width: u32, height: u32, scale: f32) -> String {
    let mut svg = start(width, height, "#ffffff");
    svg.push_str(&format!(r#"<g transform="scale({scale})">"#));
    svg.push_str(&tree.to_string(&usvg::WriteOptions::default()));
    svg.push_str("</g></svg>");
    svg
}

pub(crate) fn draw_paths(
    svg: &mut String,
    paths: &[crate::render::PaintPath],
    xf: Transform,
    clip: Option<(usize, crate::model::Bounds)>,
) {
    use std::fmt::Write;
    use tiny_skia::PathSegment;

    let _ = write!(
        svg,
        r#"<g transform="matrix({} {} {} {} {} {})">"#,
        xf.sx, xf.ky, xf.kx, xf.sy, xf.tx, xf.ty
    );
    let clip = clip.filter(|(_, bounds)| bounds.is_valid());
    if let Some((id, bounds)) = clip {
        let _ = write!(
            svg,
            r#"<clipPath id="clip{id}"><rect x="{}" y="{}" width="{}" height="{}"/></clipPath><g clip-path="url(#clip{id})">"#,
            bounds.minx as f32,
            bounds.miny as f32,
            bounds.width() as f32,
            bounds.height() as f32
        );
    }
    for path in paths {
        let (r, g, b) = path.color;
        if let Some(width) = path.stroke_width {
            let _ = write!(
                svg,
                r##"<path fill="none" stroke="#{r:02x}{g:02x}{b:02x}" stroke-width="{width}" stroke-linecap="round" stroke-linejoin="round" d=""##
            );
        } else {
            let _ = write!(svg, r##"<path fill="#{r:02x}{g:02x}{b:02x}" d=""##);
        }
        for segment in path.path.segments() {
            let _ = match segment {
                PathSegment::MoveTo(p) => write!(svg, "M{} {} ", p.x, p.y),
                PathSegment::LineTo(p) => write!(svg, "L{} {} ", p.x, p.y),
                PathSegment::QuadTo(a, b) => write!(svg, "Q{} {} {} {} ", a.x, a.y, b.x, b.y),
                PathSegment::CubicTo(a, b, c) => {
                    write!(svg, "C{} {} {} {} {} {} ", a.x, a.y, b.x, b.y, c.x, c.y)
                }
                PathSegment::Close => write!(svg, "Z"),
            };
        }
        svg.push_str("\"/>");
    }
    if clip.is_some() {
        svg.push_str("</g>");
    }
    svg.push_str("</g>");
}
