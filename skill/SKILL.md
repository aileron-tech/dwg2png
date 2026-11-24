---
name: dwg2png
description: Render DWG, DXF and SVG drawings to PNG previews, vector SVG pages and tiles, extract native text with pixel positions, and build a searchable metadata index. Use for AutoCAD files, SVG blueprints, CAD drawing previews, text extraction, or drawing ingestion for RAG, search and vision models.
---

# dwg2png

The dwg2png CLI ingests AutoCAD DWG (R13-R2018), text or binary DXF,
and SVG drawings. It produces PNG previews, optional vector SVG pages, readable tiles,
a native text layer with pixel positions, and a JSON index. CAD layouts
include Model space and paper sheets. SVG uses one canvas. The tool runs
offline on the CPU.

## Get the tool

Prefer the Docker image (self-contained, includes a font with broad script
coverage); fall back to a release binary or building from source. Source repo:
`aileron-tech/dwg2png`.

```bash
# A) Docker (build once from the repo)
git clone https://github.com/aileron-tech/dwg2png && cd dwg2png
docker build -t dwg2png:latest .

# B) Prebuilt binary (macOS arm64/x64, Linux x64, Windows x64)
gh release download --repo aileron-tech/dwg2png --pattern '*linux*'
tar -xzf dwg2png-*-linux-gnu.tar.gz   # -> ./dwg2png

# C) From source
cargo build --release --bin dwg2png   # -> target/release/dwg2png
```

Native runs need a TrueType font; default is macOS Arial. On Linux/Docker set
`DWG2PNG_FONT` (the image sets it to DejaVu Sans) or pass `--font /path.ttf`.
Use a font whose glyph coverage matches the drawings' labels.

## Run it

```bash
# whole folder -> high-res pages + tiles + text layer + index
dwg2png input/ --out out --size 6000 --tiles --no-compare

# Docker equivalent
docker run --rm -v "$PWD/input:/work/input:ro" -v "$PWD/out:/work/out" \
  dwg2png:latest /work/input --out /work/out --size 6000 --tiles --no-compare
```

Inputs may be files or directories. Folders are searched recursively for DWG, DXF and SVG files. Key flags:

- `--out DIR` — output root (default `out`)
- `--size PX` — long-edge pixels (default 3000, up to 12000)
- `--tiles` — also emit ~1500 px readable crops (`--tile-size N` to change)
- `--svg` writes a vector SVG per page alongside its PNG, including DWG and DXF conversions.
- `--font TTF` — typeface (or `DWG2PNG_FONT` env)
- `--no-compare` — skip the ground-truth PDF comparison (no `pdftoppm` needed)

Full flag reference: see [references/cli.md](references/cli.md).

## What it produces (under `--out`)

```
out/
  img/fNN-pMM.svg        vector pages when --svg is enabled
  img/fNN-pMM.png        page previews (p00 = Model space, then paper sheets)
  tiles/fNN-pMM/rRcC.png readable crops + tiles.json (per-tile labels)
  labels/fNN-pMM.json    text layer: exact strings + pixel x,y,w,h + layer + color
  json/fNN.json          per-file index: layers, blocks, attributes, histogram
  report.html            self-contained report (toggleable SVG text overlay)
```

`fNN` = file index, `pMM` = page index. Output schemas:
[references/outputs.md](references/outputs.md).

## Reading every label (the key idea)

CAD labels are sub-pixel against a large drawing extent, and VLMs downscale
images on input — so a single raster never makes every label legible. Two
channels solve this; use both:

1. **Text layer** (`labels/*.json`) — strings come from native drawing text, each anchored to a pixel box.
   Text converted to paths in the source SVG is available in the image. This is the reliable
   channel for "read every label" and for grounded citations.
2. **Tiles** (`--tiles`) — ~1500 px crops where text is large enough to read
   visually; each tile's `tiles.json` carries the labels inside it.

## Using it in a RAG / VLM pipeline

The text layer + index give OCR-free, positionally-grounded text; the tiles give
readable image context. Build chunks from the labels and attach geometry as
metadata for citations. A ready-made helper:

```bash
# cluster labels into embed-ready chunks (JSONL) with {file,page,layer,bbox}
python3 scripts/labels_to_chunks.py out/labels --out chunks.jsonl
```

Patterns (chunking, embedding, citation, multimodal): see
[references/rag.md](references/rag.md).

## Requirements & sizing

- Image ~81 MB; binary ~2.3 MB. No GPU/C deps.
- Typical file (≤1 MB): ~1–2 s, 200–450 MB RAM. Large 40–50 MB drawings:
  ~25–30 s, **~2.8 GB RAM** (parse-bound) — give those a high-memory queue.
- Memory tracks DWG **file size**, not render resolution. Process one DWG per
  invocation; scale out with parallel workers/replicas.

## Notes

- Not rendered (but counted/indexed): hatch fills, dimension geometry (its text
  *is* captured), leaders, lineweights. Drawings stay recognizable.
- A corrupt/unreadable DWG degrades to a placeholder via failsafe parsing instead
  of crashing — safe for batch ingestion.
