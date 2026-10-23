//! Searchable text inside attachments: PDF (hayro, also the invisible OCR layer of scans),
//! Word and Excel (`.docx`, `.xlsx`), plain text and attached messages.
//!
//! Attachments are hostile input. Everything is capped (pages, characters) and the PDF reader
//! runs under `catch_unwind`, so a malformed file costs its text, never the indexer.

use std::panic::{AssertUnwindSafe, catch_unwind};

use hayro::hayro_interpret::font::{Glyph, GlyphRun};
use hayro::hayro_interpret::{
    BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageDrawProps,
    InterpreterCache, InterpreterSettings, SoftMask, TransformExt, interpret_page,
};
use hayro::hayro_syntax::Pdf;
use hayro::kurbo::{Affine, BezPath, Point, Rect};

use crate::model::{Attachment, FileKind};

/// Characters kept per file: enough for invoices, offers and the first pages of a contract.
pub const MAX_CHARS: usize = 20_000;
/// PDF pages read per file.
const MAX_PAGES: usize = 30;
/// Files bigger than this are listed but not read.
pub const MAX_BYTES: usize = 25 * 1024 * 1024;

/// The text of an attachment, whitespace collapsed and capped at [`MAX_CHARS`]. Empty for
/// images, archives, unknown formats and files that cannot be read.
pub fn text_of(a: &Attachment) -> String {
    if a.data.len() > MAX_BYTES {
        return String::new();
    }
    let raw = match a.kind() {
        FileKind::Pdf => pdf_text(&a.data),
        FileKind::Document if a.extension() == "docx" => crate::office::docx(&a.data)
            .map(|d| d.to_text())
            .unwrap_or_default(),
        FileKind::Spreadsheet if matches!(a.extension().as_str(), "xlsx" | "xlsm") => {
            crate::office::xlsx(&a.data)
                .map(|d| d.to_text())
                .unwrap_or_default()
        }
        FileKind::Text => decode_text(a.data.get(..MAX_CHARS.saturating_mul(4)).unwrap_or(&a.data)),
        FileKind::Message => crate::imap_client::parse_body(&a.data).text,
        _ => String::new(),
    };
    collapse(&raw)
}

/// UTF-8, else Windows-1252/Latin-1 (old CSV exports).
pub fn decode_text(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => bytes.iter().map(|b| char::from(*b)).collect(),
    }
}

/// One space between words, at most [`MAX_CHARS`] characters.
fn collapse(text: &str) -> String {
    let mut out = String::with_capacity(text.len().min(MAX_CHARS.saturating_mul(2)));
    let mut count = 0usize;
    for word in text.split_whitespace() {
        if count >= MAX_CHARS {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
            count = count.saturating_add(1);
        }
        for c in word.chars() {
            if count >= MAX_CHARS {
                break;
            }
            if !c.is_control() {
                out.push(c);
                count = count.saturating_add(1);
            }
        }
    }
    out
}

/// Text of a PDF in reading order, as far as the content streams tell it.
pub fn pdf_text(data: &[u8]) -> String {
    catch_unwind(AssertUnwindSafe(|| pdf_text_inner(data))).unwrap_or_default()
}

fn pdf_text_inner(data: &[u8]) -> String {
    let Ok(pdf) = Pdf::new(data.to_vec()) else {
        return String::new();
    };
    let cache = InterpreterCache::new();
    let settings = InterpreterSettings::default();
    let mut sink = TextSink::default();
    for page in pdf.pages().iter().take(MAX_PAGES) {
        if sink.chars >= MAX_CHARS {
            break;
        }
        let (w, h) = page.render_dimensions();
        let mut ctx = Context::new(
            page.initial_transform(true).to_kurbo(),
            Rect::new(0.0, 0.0, f64::from(w), f64::from(h)),
            &cache,
            page.xref(),
            settings.clone(),
        );
        sink.last = None;
        interpret_page(page, &mut ctx, &mut sink);
        sink.push('\n');
    }
    sink.out
}

/// Collects glyphs as text: a new line when the baseline moves, a space when there is a gap.
#[derive(Default)]
struct TextSink {
    out: String,
    chars: usize,
    /// Where the previous glyph ended (x), its baseline (y) and its size.
    last: Option<(f64, f64, f64)>,
}

impl TextSink {
    fn push(&mut self, c: char) {
        if self.chars < MAX_CHARS {
            self.out.push(c);
            self.chars = self.chars.saturating_add(1);
        }
    }
}

impl<'a> Device<'a> for TextSink {
    fn draw_path(&mut self, _: &BezPath, _: DrawProps<'a>, _: &DrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_image(&mut self, _: Image<'a, '_>, _: ImageDrawProps<'a>) {}
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}

    fn draw_glyph_run(&mut self, run: &GlyphRun<'_, 'a>, props: DrawProps<'a>, mode: &DrawMode) {
        // Fill-and-stroke text arrives twice; the fill (or the invisible OCR layer) is enough.
        if matches!(mode, DrawMode::Stroke(_)) || self.chars >= MAX_CHARS {
            return;
        }
        for g in run.glyphs() {
            let t: Affine = props.transform * g.transform();
            let origin = t * Point::ZERO;
            let size = (t * Point::new(0.0, 1000.0) - origin).hypot().max(0.1);
            let advance = match &**g {
                Glyph::Outline(o) => f64::from(o.advance_width().unwrap_or(500.0)),
                Glyph::Type3(_) => 500.0,
            };
            let end = (t * Point::new(advance, 0.0)).x;
            if let Some((last_end, last_y, last_size)) = self.last {
                let line = last_size.max(size);
                if (origin.y - last_y).abs() > line * 0.5 {
                    self.push('\n');
                } else if origin.x - last_end > line * 0.2 || origin.x < last_end - line * 2.0 {
                    self.push(' ');
                }
            }
            match g.as_unicode() {
                Some(hayro::hayro_interpret::hayro_cmap::BfString::Char(c)) => self.push(c),
                Some(hayro::hayro_interpret::hayro_cmap::BfString::String(s)) => {
                    for c in s.chars() {
                        self.push(c);
                    }
                }
                None => {}
            }
            self.last = Some((end, origin.y, size));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn att(name: &str, data: &[u8]) -> Attachment {
        Attachment {
            name: name.into(),
            mime: "application/octet-stream".into(),
            content_id: None,
            data: Arc::from(data),
        }
    }

    /// A one-page PDF with "Fattura 2026/118" in Helvetica, offsets computed here.
    fn tiny_pdf() -> Vec<u8> {
        let stream =
            "BT /F1 18 Tf 72 720 Td (Fattura 2026/118) Tj 0 -30 Td (Totale 1.250,00) Tj ET";
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_string(),
            format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len()),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        ];
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, o) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
        );
        for off in offsets {
            out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        out
    }

    #[test]
    fn reads_pdf_text_in_lines() {
        let text = pdf_text(&tiny_pdf());
        assert!(text.contains("Fattura 2026/118"), "{text:?}");
        assert!(text.contains("Totale 1.250,00"), "{text:?}");
        let a = att("fattura.pdf", &tiny_pdf());
        assert!(
            text_of(&a).contains("Fattura 2026/118 Totale"),
            "{}",
            text_of(&a)
        );
    }

    #[test]
    fn plain_text_is_collapsed_and_capped() {
        let a = att("note.txt", b"  riga uno\r\n\r\n  riga\tdue  ");
        assert_eq!(text_of(&a), "riga uno riga due");
        let long = "parola ".repeat(MAX_CHARS);
        assert_eq!(
            text_of(&att("x.txt", long.as_bytes())).chars().count(),
            MAX_CHARS
        );
    }

    #[test]
    fn garbage_never_panics() {
        for data in [
            &b""[..],
            b"%PDF-1.4\n",
            b"%PDF-1.7\n1 0 obj << >>",
            b"\xff\xfe\x00PK\x03\x04",
        ] {
            for name in ["a.pdf", "a.docx", "a.xlsx", "a.txt", "a.eml", "a.zip"] {
                let _ = text_of(&att(name, data));
            }
        }
    }
}
