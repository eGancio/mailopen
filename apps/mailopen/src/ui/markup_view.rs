//! Live rendering of the compose markup inside the text editor: headings big and bold, bold and
//! italic for real, list markers and links in the accent colour, the markup signs themselves
//! faint. The text is untouched; only the layout changes (`TextEdit::layouter`).

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId, Stroke};

use crate::ui::theme::{self, Tokens};

struct Style {
    base: TextFormat,
    faint: TextFormat,
    /// Markup signs the writer should not see: tiny and transparent (the cursor still crosses them).
    hidden: TextFormat,
    accent: TextFormat,
    link: TextFormat,
}

fn fmt(font: FontId, color: Color32) -> TextFormat {
    TextFormat {
        font_id: font,
        color,
        ..TextFormat::default()
    }
}

fn push(job: &mut LayoutJob, text: &str, start: usize, end: usize, f: &TextFormat) {
    if start < end && end <= text.len() {
        job.append(text.get(start..end).unwrap_or(""), 0.0, f.clone());
    }
}

/// Build the layout for `text`, wrapped at `wrap_width`.
pub fn layout(text: &str, wrap_width: f32, t: &Tokens, size: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap_width;
    let st = Style {
        base: fmt(FontId::proportional(size), t.text),
        faint: fmt(FontId::proportional(size), t.border),
        hidden: fmt(FontId::proportional(0.5), Color32::TRANSPARENT),
        accent: fmt(theme::bold_font(size), t.accent),
        link: TextFormat {
            underline: Stroke::new(1.0, t.accent),
            ..fmt(FontId::proportional(size), t.accent)
        },
    };
    let mut pos = 0usize;
    for line in text.split_inclusive('\n') {
        let start = pos;
        pos += line.len();
        line_sections(&mut job, text, start, line, &st, size);
    }
    job
}

fn line_sections(job: &mut LayoutJob, text: &str, start: usize, line: &str, st: &Style, size: f32) {
    let trimmed = line.trim_start();
    let indent = line.len() - trimmed.len();
    let mut cur = start;
    push(job, text, cur, cur + indent, &st.base);
    cur += indent;
    // Block markers
    let (marker_len, body_fmt, marker_fmt) = if trimmed.starts_with("## ") {
        (
            3,
            fmt(theme::bold_font(size * 1.3), st.base.color),
            st.hidden.clone(),
        )
    } else if trimmed.starts_with("# ") {
        (
            2,
            fmt(theme::bold_font(size * 1.6), st.base.color),
            st.hidden.clone(),
        )
    } else if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
        (2, st.base.clone(), st.accent.clone())
    } else {
        let digits = trimmed.chars().take_while(|c| c.is_ascii_digit()).count();
        let numbered = digits > 0 && trimmed.get(digits..).is_some_and(|r| r.starts_with(". "));
        if numbered {
            (digits + 2, st.base.clone(), st.accent.clone())
        } else {
            (0, st.base.clone(), st.faint.clone())
        }
    };
    push(job, text, cur, cur + marker_len, &marker_fmt);
    cur += marker_len;
    inline_sections(job, text, cur, start + line.len(), &body_fmt, st);
}

fn inline_sections(
    job: &mut LayoutJob,
    text: &str,
    from: usize,
    to: usize,
    body: &TextFormat,
    st: &Style,
) {
    let s = text.get(from..to).unwrap_or("");
    let bytes = s.as_bytes();
    let mut bold = false;
    let mut italic = false;
    let mut i = 0usize;
    let mut seg_start = 0usize;
    let style = |bold: bool, italic: bool| -> TextFormat {
        let mut f = body.clone();
        if bold {
            f.font_id = theme::bold_font(f.font_id.size);
        }
        f.italics = italic;
        f
    };
    while i < bytes.len() {
        let rest = s.get(i..).unwrap_or("");
        // [text](url)
        if rest.starts_with('[')
            && let Some(close) = rest.find("](")
            && let Some(end) = rest
                .get(close..)
                .and_then(|r| r.find(')'))
                .map(|e| close + e)
        {
            push(job, text, from + seg_start, from + i, &style(bold, italic));
            push(job, text, from + i, from + i + 1, &st.faint);
            push(job, text, from + i + 1, from + i + close, &st.link);
            push(job, text, from + i + close, from + i + close + 2, &st.faint);
            push(job, text, from + i + close + 2, from + i + end, &st.faint);
            push(job, text, from + i + end, from + i + end + 1, &st.faint);
            i += end + 1;
            seg_start = i;
            continue;
        }
        if rest.starts_with("http://") || rest.starts_with("https://") {
            let len = rest.find(char::is_whitespace).unwrap_or(rest.len());
            push(job, text, from + seg_start, from + i, &style(bold, italic));
            push(job, text, from + i, from + i + len, &st.link);
            i += len;
            seg_start = i;
            continue;
        }
        if rest.starts_with("**") {
            push(job, text, from + seg_start, from + i, &style(bold, italic));
            push(job, text, from + i, from + i + 2, &st.hidden);
            bold = !bold;
            i += 2;
            seg_start = i;
            continue;
        }
        if rest.starts_with('*') {
            let next_space = rest
                .get(1..)
                .and_then(|r| r.chars().next())
                .is_none_or(|c| c.is_whitespace());
            if italic || !next_space {
                push(job, text, from + seg_start, from + i, &style(bold, italic));
                push(job, text, from + i, from + i + 1, &st.hidden);
                italic = !italic;
                i += 1;
                seg_start = i;
                continue;
            }
        }
        // advance one char
        let step = rest.chars().next().map(char::len_utf8).unwrap_or(1);
        i += step;
    }
    push(job, text, from + seg_start, to, &style(bold, italic));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn covers(text: &str) {
        let job = layout(text, 400.0, &theme::LIGHT, 14.0);
        let mut pos = 0usize;
        for s in &job.sections {
            assert_eq!(s.byte_range.start.0, pos, "gap in {text:?}");
            pos = s.byte_range.end.0;
        }
        assert_eq!(pos, text.len(), "text not fully covered: {text:?}");
        assert_eq!(job.text, text);
    }

    #[test]
    fn sections_cover_everything() {
        for t in [
            "",
            "ciao",
            "# Titolo\n\nCiao **Anna**, vedi *qui*.\n- uno\n- due\n\n1. a\n2. b\nhttps://x.it fine [l](https://a.b)",
            "**",
            "*",
            "[a](",
            "àè **ü** *ö*\n  - indent",
            "***x***",
            "# \n## \n- \n1. ",
        ] {
            covers(t);
        }
    }

    #[test]
    fn heading_is_bigger_and_bold_is_bold() {
        let job = layout("# T\n**b**", 400.0, &theme::LIGHT, 14.0);
        let t_section = job
            .sections
            .iter()
            .find(|s| job.text.get(s.byte_range.start.0..s.byte_range.end.0) == Some("T\n"))
            .unwrap();
        assert!(t_section.format.font_id.size > 20.0);
        let b_section = job
            .sections
            .iter()
            .find(|s| job.text.get(s.byte_range.start.0..s.byte_range.end.0) == Some("b"))
            .unwrap();
        assert_eq!(b_section.format.font_id.family, theme::bold_family());
    }
}
