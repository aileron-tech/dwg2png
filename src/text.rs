//! Vector text rendering via TrueType glyph outlines (ab_glyph).
//!
//! Produces filled contours in model coordinates for a text string placed at a
//! given insertion point, height (world units) and rotation, folded through the
//! active INSERT affine. Glyph curves are flattened to polygons; holes are
//! preserved by emitting all contours into a single non-zero-winding fill.

use ab_glyph::{Font, FontVec, OutlineCurve, Point as GPoint};

use crate::model::{Aff, P};

const GAP_EPS: f32 = 0.5; // font-unit distance that starts a new contour
const QUAD_STEPS: usize = 6;
const CUBIC_STEPS: usize = 10;

/// Load a font from a TTF/OTF file.
pub fn load_font(path: &str) -> Result<FontVec, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read font {path}: {e}"))?;
    FontVec::try_from_vec(bytes).map_err(|e| format!("parse font {path}: {e}"))
}

/// Reconstruct filled contours (model coords) for `text`.
pub fn text_contours(
    font: &FontVec,
    text: &str,
    height: f64,
    origin: P,
    rotation: f64,
    aff: &Aff,
) -> Vec<Vec<P>> {
    let upm = font.units_per_em().unwrap_or(1000.0) as f64;
    if upm <= 0.0 || height <= 0.0 {
        return Vec::new();
    }
    let s = height / upm;
    let (sin, cos) = rotation.sin_cos();
    let mut pen: f64 = 0.0;
    let mut contours: Vec<Vec<P>> = Vec::new();

    for ch in text.chars() {
        if ch == '\n' || (ch.is_control() && ch != '\t') {
            continue;
        }
        let id = font.glyph_id(ch);
        let pen_now = pen;
        // font-space (fx,fy) -> model coords
        let map = |fx: f32, fy: f32| -> P {
            let ex = s * (pen_now + fx as f64);
            let ey = s * fy as f64;
            let lx = origin.x + cos * ex - sin * ey;
            let ly = origin.y + sin * ex + cos * ey;
            aff.apply(P::new(lx, ly))
        };

        if let Some(outline) = font.outline(id) {
            let mut cur: Vec<P> = Vec::new();
            let mut last: Option<GPoint> = None;
            for curve in &outline.curves {
                let start = curve_start(curve);
                if last.map_or(true, |l| dist(l, start) > GAP_EPS) {
                    if cur.len() >= 2 {
                        contours.push(std::mem::take(&mut cur));
                    } else {
                        cur.clear();
                    }
                    cur.push(map(start.x, start.y));
                }
                flatten(curve, &map, &mut cur);
                last = Some(curve_end(curve));
            }
            if cur.len() >= 2 {
                contours.push(cur);
            }
        }
        pen += font.h_advance_unscaled(id) as f64;
    }
    contours
}

fn dist(a: GPoint, b: GPoint) -> f32 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
}

fn curve_start(c: &OutlineCurve) -> GPoint {
    match c {
        OutlineCurve::Line(a, _) => *a,
        OutlineCurve::Quad(a, _, _) => *a,
        OutlineCurve::Cubic(a, _, _, _) => *a,
    }
}

fn curve_end(c: &OutlineCurve) -> GPoint {
    match c {
        OutlineCurve::Line(_, b) => *b,
        OutlineCurve::Quad(_, _, b) => *b,
        OutlineCurve::Cubic(_, _, _, b) => *b,
    }
}

fn flatten(c: &OutlineCurve, map: &impl Fn(f32, f32) -> P, out: &mut Vec<P>) {
    match c {
        OutlineCurve::Line(_, b) => out.push(map(b.x, b.y)),
        OutlineCurve::Quad(a, ctrl, b) => {
            for i in 1..=QUAD_STEPS {
                let t = i as f32 / QUAD_STEPS as f32;
                let mt = 1.0 - t;
                let x = mt * mt * a.x + 2.0 * mt * t * ctrl.x + t * t * b.x;
                let y = mt * mt * a.y + 2.0 * mt * t * ctrl.y + t * t * b.y;
                out.push(map(x, y));
            }
        }
        OutlineCurve::Cubic(a, c1, c2, b) => {
            for i in 1..=CUBIC_STEPS {
                let t = i as f32 / CUBIC_STEPS as f32;
                let mt = 1.0 - t;
                let x = mt * mt * mt * a.x
                    + 3.0 * mt * mt * t * c1.x
                    + 3.0 * mt * t * t * c2.x
                    + t * t * t * b.x;
                let y = mt * mt * mt * a.y
                    + 3.0 * mt * mt * t * c1.y
                    + 3.0 * mt * t * t * c2.y
                    + t * t * t * b.y;
                out.push(map(x, y));
            }
        }
    }
}

/// Decode the `%%` control codes of single-line text (TEXT, attributes, dimension
/// text): `%%d` degree, `%%p` plus/minus, `%%c` diameter, `%%%` percent, `%%nnn` a
/// character by its decimal code. The underline, overline and strike toggles
/// (`%%u`, `%%o`, `%%k`) are dropped.
pub fn decode_text_codes(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(at) = rest.find("%%") {
        out.push_str(&rest[..at]);
        let after = &rest[at + 2..];
        let mut chars = after.chars();
        match chars.next() {
            Some('u' | 'U' | 'o' | 'O' | 'k' | 'K') => rest = &after[1..],
            Some('d' | 'D') => {
                out.push('\u{b0}');
                rest = &after[1..];
            }
            Some('p' | 'P') => {
                out.push('\u{b1}');
                rest = &after[1..];
            }
            Some('c' | 'C') => {
                out.push('\u{d8}');
                rest = &after[1..];
            }
            Some('%') => {
                out.push('%');
                rest = &after[1..];
            }
            Some(d) if d.is_ascii_digit() => {
                let digits: String = after.chars().take(3).take_while(char::is_ascii_digit).collect();
                match digits.parse::<u32>().ok().and_then(char::from_u32) {
                    Some(ch) if digits.len() == 3 => out.push(ch),
                    _ => out.push_str(&format!("%%{digits}")),
                }
                rest = &after[digits.len()..];
            }
            _ => {
                out.push_str("%%");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Reduce MTEXT content to the text a reader sees: formatting codes are removed,
/// paragraph breaks become spaces, stacked fractions become "a/b".
///
/// Codes with a value run to the next `;` (font `\f`, height `\H`, color `\C`,
/// tracking `\T`, oblique `\Q`, width `\W`, alignment `\A`, paragraph `\p`, stack
/// `\S`). Toggles have no value (underline `\L \l`, overline `\O \o`, strike
/// `\K \k`), so nothing after them is skipped.
pub fn strip_mtext(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                // paragraph break, column break, wrap marker, non-breaking space
                Some('P') | Some('N') | Some('X') | Some('~') => out.push(' '),
                Some('\\') => out.push('\\'),
                Some('{') => out.push('{'),
                Some('}') => out.push('}'),
                // stacked text: "1/2", "1#2" (diagonal) or "+0.1^-0.1" (tolerance)
                Some('S') => {
                    // "10" followed by the stack "1/2" reads "10 1/2", not "101/2"
                    let after_digit = out.chars().next_back().is_some_and(|p| p.is_ascii_digit());
                    if after_digit && chars.peek().is_some_and(char::is_ascii_digit) {
                        out.push(' ');
                    }
                    for d in chars.by_ref() {
                        match d {
                            ';' => break,
                            '#' => out.push('/'),
                            '^' => out.push(' '),
                            other => out.push(other),
                        }
                    }
                }
                // \U+XXXX: a Unicode code point in four hex digits
                Some('U') if chars.peek() == Some(&'+') => {
                    chars.next();
                    let hex: String = chars.by_ref().take(4).collect();
                    match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                        Some(ch) => out.push(ch),
                        None => out.push_str(&hex),
                    }
                }
                // codes with a value: skip to ';'
                Some(f) if "fFHhCcTQWAp".contains(f) => {
                    for d in chars.by_ref() {
                        if d == ';' {
                            break;
                        }
                    }
                }
                // toggles without a value, and any unknown code: drop the code only
                Some(_) | None => {}
            },
            '{' | '}' => {}
            _ => out.push(c),
        }
    }
    out
}


#[cfg(test)]
mod tests {
    use super::{decode_text_codes, strip_mtext};

    /* REQ-CLI-025: %% control codes of single-line text are decoded or dropped */
    #[test]
    fn text_control_codes_are_decoded() {
        assert_eq!(decode_text_codes("%%uMASTER BEDROOM"), "MASTER BEDROOM");
        assert_eq!(decode_text_codes("%%UGREAT ROOM%%U"), "GREAT ROOM");
        assert_eq!(decode_text_codes("45%%d TO 60%%d"), "45\u{b0} TO 60\u{b0}");
        assert_eq!(decode_text_codes("%%c25 %%p0.1"), "\u{d8}25 \u{b1}0.1");
        assert_eq!(decode_text_codes("100%%%"), "100%");
        assert_eq!(decode_text_codes("%%065BC"), "ABC");
        assert_eq!(decode_text_codes("plain text 50% off"), "plain text 50% off");
        assert_eq!(decode_text_codes("ends with %%"), "ends with %%");
    }

    /* REQ-CLI-020: MTEXT formatting codes are removed and the text is kept */
    #[test]
    fn formatting_codes_are_removed() {
        assert_eq!(strip_mtext("{\\fArial|b0|i0|c0|p34;\\H2.5x;\\C1;NOTE}"), "NOTE");
        assert_eq!(strip_mtext("line one\\Pline two"), "line one line two");
        assert_eq!(strip_mtext("a\\~b \\\\ \\{c\\}"), "a b \\ {c}");
    }

    /* REQ-CLI-021: paragraph codes are dropped, not mistaken for a paragraph break */
    #[test]
    fn paragraph_codes_are_dropped() {
        assert_eq!(strip_mtext("\\pi-12,l12,t12;REVISIONS"), "REVISIONS");
        assert_eq!(strip_mtext("\\pxqc;TITLE\\PSUBTITLE"), "TITLE SUBTITLE");
    }

    /* REQ-CLI-022: stacked fractions and tolerances read as plain text */
    #[test]
    fn stacked_text_is_flattened() {
        assert_eq!(strip_mtext("ELEV. 5'-10\\S1/2;\" TOP OF LANDING"), "ELEV. 5'-10 1/2\" TOP OF LANDING");
        assert_eq!(strip_mtext("\\S1#2;"), "1/2");
        assert_eq!(strip_mtext("10\\S+0.1^-0.1;"), "10+0.1 -0.1");
    }

    /* REQ-CLI-023: underline, overline and strike toggles do not swallow the text after them */
    #[test]
    fn toggles_keep_the_following_text() {
        assert_eq!(strip_mtext("\\LGENERAL NOTES\\l; see sheet A-2; rev 3"), "GENERAL NOTES; see sheet A-2; rev 3");
        assert_eq!(strip_mtext("\\OAB\\o \\Kold\\k new; done"), "AB old new; done");
    }

    /* REQ-CLI-024: a Unicode escape becomes its character */
    #[test]
    fn unicode_escapes_are_decoded() {
        assert_eq!(strip_mtext("\\U+00D8 25"), "\u{d8} 25");
    }
}
