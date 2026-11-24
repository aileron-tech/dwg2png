# LOG

## 2026-06-25 — Initial DWG→multipage PNG renderer + indexer

**Goal:** Build a pure-Rust SaaS backend library that indexes DWG files —
render every layout to PNG and extract searchable metadata — verified on real
sample files with proof in an HTML report.

**Motivation:** Indexing DWGs needs both per-sheet previews and searchable
fields (text, title-block attributes, layers, layouts). Chose `acadrust`
(MPL-2.0, pure Rust) over LibreDWG to avoid GPL + C FFI + the crashy C parser.

**Changes:**
- `acadrust 0.4.0` adapter (`lib.rs`): failsafe DWG read, `catch_unwind` around
  parse and render for crash isolation.
- Pure geometry layer: entity → render primitives with arc/circle/ellipse/bulge
  sampling, INSERT block recursion (composed affine + ByBlock color), TrueType
  vector text (ab_glyph).
- Page model: Layout objects → Model space + paper sheets. Paper-space
  **viewport projection** of model geometry (position/scale/twist + clip mask).
- `tiny-skia` CPU renderer: world→screen transform, color-batched stroke/fill,
  hairline widths, Y-flip, viewport clip masks.
- `metadata.rs` → `IndexDoc` JSON (histogram, layers, blocks, attributes, text,
  layouts). `report.rs` → self-contained HTML with thumbnails + PDF comparison.

**Verification status:** PASS. 11/11 real DWG 2018 files (4–46 MB) parsed,
23 pages rendered, 0 crashes. Corrupt/recovered file degraded via failsafe.
Side-by-side vs plotted PDF matches network topology and building footprints.
Metadata extracts real cadastral attributes and Cyrillic text correctly.

**Open / next:** Hatch fills, dimension geometry, leaders, lineweights,
per-viewport UCS/layer-freeze, De Boor spline evaluation, text alignment.
Productionize acadrust risk: vendor/fork, corpus regression, sandboxed worker.

## 2026-06-25 — Label readability for VLM consumption

**Goal:** Make every label readable by a VLM / understand the blueprint 100%.

**Motivation:** Tiny CAD labels are sub-pixel against a large extent, and VLMs
downscale on input, so raster resolution alone can't make all labels legible.

**Changes:**
- **Text layer:** capture every text run (Text, MText, Attribute, Dimension,
  MultiLeader) as a `Label` with world placement, project it through viewports,
  and emit `labels/*.json` with exact strings + pixel `x,y,w,h`, rotation, color,
  layer. Lossless (no OCR).
- **Resolution:** `--size` (default 3000, up to 12000).
- **Tiling:** `--tiles` slices each page into ~1500 px crops (tiny-skia
  `clone_rect`) with per-tile labels — readable VLM input.
- **Report:** toggleable inline-SVG text overlay (crisp at any zoom) + tile grids.

**Verification:** PASS. Full batch at `--size 5000 --tiles`: 11/11 files,
23 pages, 216 tiles, **128,410 position-referenced labels** (pipe-spec labels
legible in tiles). 8 unit tests pass, 0 warnings.

## 2026-11-24 - Reposition as DWG ingestion for AI applications

**Goal:** Present the project as a DWG ingestion step for AI applications (RAG,
search, VLM) instead of a renderer for one SaaS backend, after the repository
moved to the `aileron-tech` organization.

**Changes:**
- README: new lead with an output-to-use table; the RAG section moved to the
  top and renamed "Use in AI pipelines (RAG, search, VLM)"; new "Agent skill"
  section with the install command.
- README, `skill/README.md`, `skill/SKILL.md`: repository URLs now point to
  `aileron-tech/dwg2png`.
- GitHub repository description and topics updated to match.

**Verification:** Docs only; no code changed. No reference to the previous
organization remains outside this log; relative links in the README resolve to existing files.

## 2026-11-24 - Companion programs: `--mcp` and `--apify`

**Goal:** Let the MCP server and the Apify Actor ship as separate programs
(`dwg-mcp`, `dwg-apify`, each its own crate and repository, installable on its
own) while `dwg2png --mcp` and `dwg2png --apify` stay the one entry point.

**Changes:**
- `src/plugin.rs`: maps the two flags to their programs, looks the program up
  next to the executable and then on `PATH`, and hands the process over
  (`exec` on Unix, spawn and wait elsewhere). A missing program prints the
  install command and exits 127.
- `src/main.rs`: checks the first argument for a companion flag before the
  renderer's own argument parsing. The renderer CLI is otherwise unchanged.
- First integration tests of the repository: `tests/companion_dispatch.rs`.

**Verification:** PASS. `cargo test --release`: 8 existing unit tests, 2 new
unit tests (REQ-CLI-001, 002) and 4 end-to-end tests that run the built binary
against a stand-in companion (REQ-CLI-003 to 006). `cargo clippy` reports
nothing in the new code; two older findings in `metadata.rs` and `text.rs` are
left as they were.

## 2026-11-24 - Install hint points at the companion's repository

When a companion program is missing, `dwg2png --mcp` and `--apify` now print
the link to its install instructions instead of a Homebrew command: each
companion has its own tap and installers, documented in its README.
Verified with `cargo test --release` (the four companion tests pass).

## 2026-11-24 - DXF input

**Goal:** Read DXF as well as DWG.

**Changes:**
- `src/lib.rs`: `detect_format` recognises a drawing from its first bytes (DWG
  version code, binary DXF sentinel, text DXF opening); `read_document` picks
  the acadrust DWG or DXF reader from it, falling back to the extension. Both
  run in failsafe mode with the parser wrapped in `catch_unwind`.
  `process_file` and `index_to_json` use it.
- `src/main.rs`: folders are searched for `.dwg` and `.dxf`.
- No change to geometry, rendering or metadata: both readers produce the same
  document type.

**Verification:** PASS. `cargo test --release`: 5 new tests in
`tests/dxf_input.rs` (REQ-CLI-010 to 014) run the built CLI on DXF written at
test time: labels and layers match the DWG of the same drawing; a folder with
both formats; a DXF misnamed `.dwg`; format detection. A hand-written minimal
R12-style DXF also rendered. A truncated DXF yields an empty page with an
error notification (failsafe), a file of plain text fails with a parse error.

**Not verified:** binary DXF end to end (only its detection), DXF with paper
space layouts, and DXF files from other CAD programs.

## 2026-11-24 - Text fidelity, found on real sample drawings

**Goal:** Verify on drawings from other sources. Six public files were run:
four Autodesk AutoCAD samples (DWG AC1021 and AC1024: two architectural, one
blocks-and-tables floor plan, one mechanical) and two DXF files from open
source repositories (a house floor plan, AC1018; curtain-wall details with four
sheets, AC1027). All six parsed with no reader errors: 15 pages, 99 to 4500
entities, 6 to 49 layers.

**Problems found and fixed:**
- MTEXT paragraph codes (`\\pi-12,l12,t12;`) were taken for paragraph breaks
  and leaked into labels; stacked fractions (`\\S1/2;`) leaked as `S1/2;`; the
  underline, overline and strike toggles (`\\L \\O \\K`), which take no value,
  swallowed the text up to the next `;`. `strip_mtext` now follows the MTEXT
  code table; `\\U+XXXX` is decoded.
- `%%` control codes of TEXT, attributes and MTEXT (`%%u`, `%%d`, `%%p`, `%%c`,
  `%%nnn`) appeared verbatim in labels. New `decode_text_codes`.
- Attributes flagged invisible are no longer drawn or labelled; they stay in
  the index.

**Known limitation:** In the Autodesk blocks-and-tables sample, window tags
are drawn with all their attribute values (manufacturer, size, cost), which
AutoCAD hides. The parser reports none of that file's 492 attributes or 240
attribute definitions as invisible, so the renderer cannot tell.

**Verification:** PASS. `cargo test --release`: 6 new unit tests in
`src/text.rs` (REQ-CLI-020 to 025) and 3 end-to-end tests in
`tests/text_fidelity.rs` (REQ-CLI-030 to 032). Re-rendered the six samples:
labels such as `MASTER BEDROOM`, `ELEV. 5'-10 1/2" TOP OF LANDING` and
`45 deg TO 60 deg` (with the degree sign) are now clean. Two pages were
inspected by eye: the DXF house plan and the two-floor plan.

**Not verified:** an Audi A3 drawing. Every source found requires an account
or payment.

- Add SVG input and optional per-layout vector SVG export for DWG, DXF and SVG. Render PNG previews and tiles from the exported geometry when SVG is enabled so that reopening the export at the same size yields the same pixels.
