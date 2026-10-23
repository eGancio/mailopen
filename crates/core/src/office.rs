//! Simplified previews of Word and Excel files (.docx, .xlsx), read straight from the zip with
//! no Office installed: paragraphs, bold/italic, headings, lists and tables; for workbooks the
//! first sheets as tables. The result is a [`Doc`], drawn like a message.
//!
//! Attachments are hostile input: every offset is checked and sizes are capped.

use crate::html::{
    Align, Block, Border, BoxStyle, Cell, Doc, Inline, Paragraph, Rgba, Row, Table, TextStyle,
    VAlign,
};

/// Uncompressed bytes read from one zip entry at most.
const MAX_ENTRY: usize = 32 * 1024 * 1024;
const MAX_ROWS: usize = 300;
const MAX_COLS: usize = 30;
const MAX_SHEETS: usize = 8;

fn u16_at(d: &[u8], i: usize) -> Option<usize> {
    let b = d.get(i..i.checked_add(2)?)?;
    Some(usize::from(u16::from_le_bytes([*b.first()?, *b.get(1)?])))
}

fn u32_at(d: &[u8], i: usize) -> Option<usize> {
    let b = d.get(i..i.checked_add(4)?)?;
    let v = u32::from_le_bytes([*b.first()?, *b.get(1)?, *b.get(2)?, *b.get(3)?]);
    usize::try_from(v).ok()
}

/// (name, method, compressed size, local header offset) of every entry.
fn zip_entries(d: &[u8]) -> Option<Vec<(String, usize, usize, usize)>> {
    // End of central directory: the last 0x06054b50 within 64 KiB of the end.
    let from = d.len().saturating_sub(22 + 65_535);
    let eocd = (from..d.len().saturating_sub(21))
        .rev()
        .find(|&i| d.get(i..i + 4) == Some(&[0x50, 0x4b, 0x05, 0x06][..]))?;
    let count = u16_at(d, eocd + 10)?;
    let mut p = u32_at(d, eocd + 16)?;
    let mut out = Vec::with_capacity(count.min(4096));
    for _ in 0..count.min(4096) {
        if d.get(p..p + 4) != Some(&[0x50, 0x4b, 0x01, 0x02][..]) {
            break;
        }
        let method = u16_at(d, p + 10)?;
        let comp = u32_at(d, p + 20)?;
        let n = u16_at(d, p + 28)?;
        let e = u16_at(d, p + 30)?;
        let c = u16_at(d, p + 32)?;
        let local = u32_at(d, p + 42)?;
        let name = String::from_utf8_lossy(d.get(p + 46..p + 46 + n)?).into_owned();
        out.push((name, method, comp, local));
        p = p.checked_add(46 + n + e + c)?;
    }
    Some(out)
}

fn zip_read(d: &[u8], entries: &[(String, usize, usize, usize)], name: &str) -> Option<Vec<u8>> {
    let (_, method, comp, local) = entries.iter().find(|(n, ..)| n == name)?;
    if d.get(*local..local + 4) != Some(&[0x50, 0x4b, 0x03, 0x04][..]) {
        return None;
    }
    let n = u16_at(d, local + 26)?;
    let e = u16_at(d, local + 28)?;
    let start = local.checked_add(30 + n + e)?;
    let data = d.get(start..start.checked_add(*comp)?)?;
    match method {
        0 => Some(data.to_vec()),
        8 => miniz_oxide::inflate::decompress_to_vec_with_limit(data, MAX_ENTRY).ok(),
        _ => None,
    }
}

fn para(inlines: Vec<Inline>, align: Align) -> Block {
    Block::Para(Paragraph {
        inlines,
        align,
        pre: false,
    })
}

fn heading(text: &str, size: f32) -> Block {
    para(
        vec![Inline::Text {
            text: text.to_string(),
            style: TextStyle {
                bold: true,
                size,
                ..TextStyle::default()
            },
            link: None,
        }],
        Align::Left,
    )
}

fn grid() -> Option<Border> {
    Some(Border {
        width: 1.0,
        color: Rgba::rgb(0xD0, 0xD4, 0xDA),
    })
}

fn cell(blocks: Vec<Block>, header: bool) -> Cell {
    Cell {
        style: BoxStyle {
            padding: [4.0, 8.0, 4.0, 8.0],
            border_top: grid(),
            border_right: grid(),
            border_bottom: grid(),
            border_left: grid(),
            background: header.then_some(Rgba::rgb(0xF3, 0xF4, 0xF6)),
            ..BoxStyle::default()
        },
        valign: VAlign::Top,
        colspan: 1,
        blocks,
    }
}

// ---------------------------------------------------------------------------------------------
// Word

fn attr<'a>(n: roxmltree::Node<'a, '_>, local: &str) -> Option<&'a str> {
    n.attributes()
        .find(|a| a.name() == local)
        .map(|a| a.value())
}

fn child<'a, 'i>(n: roxmltree::Node<'a, 'i>, local: &str) -> Option<roxmltree::Node<'a, 'i>> {
    n.children()
        .find(|c| c.is_element() && c.tag_name().name() == local)
}

/// `<w:b/>` is on, `<w:b w:val="0"/>` is off.
fn flag(props: Option<roxmltree::Node<'_, '_>>, local: &str) -> bool {
    props
        .and_then(|p| child(p, local))
        .is_some_and(|b| !matches!(attr(b, "val"), Some("0" | "false" | "none")))
}

fn word_paragraph(p: roxmltree::Node<'_, '_>) -> Option<Block> {
    let ppr = child(p, "pPr");
    let style = ppr
        .and_then(|x| child(x, "pStyle"))
        .and_then(|s| attr(s, "val"))
        .unwrap_or("")
        .to_ascii_lowercase();
    let level = style
        .strip_prefix("heading")
        .or_else(|| style.strip_prefix("titolo"))
        .and_then(|n| n.trim().parse::<u8>().ok());
    let base = match (style.as_str(), level) {
        ("title" | "titolo", _) => 24.0,
        (_, Some(1)) => 20.0,
        (_, Some(2)) => 17.0,
        (_, Some(_)) => 15.0,
        _ => 14.0,
    };
    let align = match ppr
        .and_then(|x| child(x, "jc"))
        .and_then(|j| attr(j, "val"))
    {
        Some("center") => Align::Center,
        Some("right" | "end") => Align::Right,
        _ => Align::Left,
    };
    let list = ppr.and_then(|x| child(x, "numPr")).is_some();
    let mut inlines: Vec<Inline> = Vec::new();
    if list {
        inlines.push(Inline::Text {
            text: "•  ".into(),
            style: TextStyle {
                size: base,
                ..TextStyle::default()
            },
            link: None,
        });
    }
    for r in p
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "r")
    {
        let rpr = child(r, "rPr");
        let size = rpr
            .and_then(|x| child(x, "sz"))
            .and_then(|s| attr(s, "val"))
            .and_then(|v| v.parse::<f32>().ok())
            .map(|half_points| (half_points / 2.0 * 4.0 / 3.0).clamp(9.0, 36.0))
            .unwrap_or(base);
        let style = TextStyle {
            bold: flag(rpr, "b") || level.is_some() || style == "title",
            italic: flag(rpr, "i"),
            underline: rpr
                .and_then(|x| child(x, "u"))
                .is_some_and(|u| attr(u, "val") != Some("none")),
            strike: flag(rpr, "strike"),
            size: if level.is_some() { base } else { size },
            ..TextStyle::default()
        };
        for t in r.children().filter(|n| n.is_element()) {
            match t.tag_name().name() {
                "t" => {
                    if let Some(text) = t.text() {
                        push(&mut inlines, text, &style);
                    }
                }
                "tab" => push(&mut inlines, "    ", &style),
                "br" | "cr" => inlines.push(Inline::Break),
                _ => {}
            }
        }
    }
    let visible = inlines
        .iter()
        .any(|i| matches!(i, Inline::Text { text, .. } if !text.trim().is_empty()));
    visible.then(|| para(inlines, align))
}

fn push(out: &mut Vec<Inline>, text: &str, style: &TextStyle) {
    if let Some(Inline::Text {
        text: t, style: s, ..
    }) = out.last_mut()
        && s == style
    {
        t.push_str(text);
        return;
    }
    out.push(Inline::Text {
        text: text.to_string(),
        style: style.clone(),
        link: None,
    });
}

fn word_blocks(parent: roxmltree::Node<'_, '_>, depth: usize, out: &mut Vec<Block>) {
    for n in parent.children().filter(|n| n.is_element()) {
        match n.tag_name().name() {
            "p" => {
                if let Some(p) = word_paragraph(n) {
                    out.push(p);
                    out.push(Block::Space(6.0));
                } else {
                    out.push(Block::Space(10.0));
                }
            }
            "tbl" if depth < 6 => {
                let mut rows = Vec::new();
                for tr in n
                    .children()
                    .filter(|c| c.is_element() && c.tag_name().name() == "tr")
                {
                    let mut cells = Vec::new();
                    for tc in tr
                        .children()
                        .filter(|c| c.is_element() && c.tag_name().name() == "tc")
                    {
                        let mut blocks = Vec::new();
                        word_blocks(tc, depth + 1, &mut blocks);
                        while matches!(blocks.last(), Some(Block::Space(_))) {
                            blocks.pop();
                        }
                        let span = child(tc, "tcPr")
                            .and_then(|p| child(p, "gridSpan"))
                            .and_then(|g| attr(g, "val"))
                            .and_then(|v| v.parse::<u16>().ok())
                            .unwrap_or(1);
                        let mut c = cell(blocks, false);
                        c.colspan = span.clamp(1, 24);
                        cells.push(c);
                    }
                    if !cells.is_empty() {
                        rows.push(Row { cells });
                    }
                    if rows.len() >= MAX_ROWS {
                        break;
                    }
                }
                if !rows.is_empty() {
                    out.push(Block::Table(Table {
                        style: BoxStyle {
                            width: Some(crate::html::Length::Percent(100.0)),
                            ..BoxStyle::default()
                        },
                        rows,
                        spacing: 0.0,
                    }));
                    out.push(Block::Space(10.0));
                }
            }
            "sdt" | "sdtContent" | "customXml" | "smartTag" => word_blocks(n, depth + 1, out),
            _ => {}
        }
    }
}

/// A `.docx` as a document, or `None` when it cannot be read.
pub fn docx(data: &[u8]) -> Option<Doc> {
    let entries = zip_entries(data)?;
    let xml = zip_read(data, &entries, "word/document.xml")?;
    let text = String::from_utf8_lossy(&xml);
    let doc = roxmltree::Document::parse(&text).ok()?;
    let body = doc
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "body")?;
    let mut blocks = Vec::new();
    word_blocks(body, 0, &mut blocks);
    while matches!(blocks.last(), Some(Block::Space(_))) {
        blocks.pop();
    }
    Some(Doc {
        blocks,
        ..Doc::default()
    })
}

// ---------------------------------------------------------------------------------------------
// Excel

/// "BC12" → column 54 (0-based).
fn column_of(r: &str) -> Option<usize> {
    let mut col = 0usize;
    let mut any = false;
    for c in r.chars().take_while(|c| c.is_ascii_alphabetic()) {
        any = true;
        let v = usize::from((c.to_ascii_uppercase() as u8).saturating_sub(b'A')) + 1;
        col = col.checked_mul(26)?.checked_add(v)?;
    }
    (any && col > 0).then(|| col - 1)
}

fn shared_strings(data: &[u8], entries: &[(String, usize, usize, usize)]) -> Vec<String> {
    let Some(xml) = zip_read(data, entries, "xl/sharedStrings.xml") else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&xml);
    let Ok(doc) = roxmltree::Document::parse(&text) else {
        return Vec::new();
    };
    doc.descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "si")
        .map(|si| {
            si.descendants()
                .filter(|n| n.is_element() && n.tag_name().name() == "t")
                .filter_map(|t| t.text())
                .collect::<String>()
        })
        .collect()
}

fn sheet_names(data: &[u8], entries: &[(String, usize, usize, usize)]) -> Vec<String> {
    let Some(xml) = zip_read(data, entries, "xl/workbook.xml") else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&xml);
    let Ok(doc) = roxmltree::Document::parse(&text) else {
        return Vec::new();
    };
    doc.descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "sheet")
        .filter_map(|s| attr(s, "name").map(str::to_string))
        .collect()
}

/// An `.xlsx` as tables, one per sheet (the first few), or `None`.
pub fn xlsx(data: &[u8]) -> Option<Doc> {
    let entries = zip_entries(data)?;
    let strings = shared_strings(data, &entries);
    let names = sheet_names(data, &entries);
    let mut blocks = Vec::new();
    for n in 1..=MAX_SHEETS {
        let Some(xml) = zip_read(data, &entries, &format!("xl/worksheets/sheet{n}.xml")) else {
            break;
        };
        let text = String::from_utf8_lossy(&xml);
        let Ok(doc) = roxmltree::Document::parse(&text) else {
            continue;
        };
        let mut grid: Vec<Vec<(String, bool)>> = Vec::new();
        let mut used_cols = 0usize;
        for row in doc
            .descendants()
            .filter(|x| x.is_element() && x.tag_name().name() == "row")
            .take(MAX_ROWS)
        {
            let mut cells: Vec<(String, bool)> = Vec::new();
            for c in row
                .children()
                .filter(|x| x.is_element() && x.tag_name().name() == "c")
            {
                let col = attr(c, "r").and_then(column_of).unwrap_or(cells.len());
                if col >= MAX_COLS {
                    continue;
                }
                let kind = attr(c, "t").unwrap_or("n");
                let v = child(c, "v").and_then(|v| v.text()).unwrap_or("");
                let (value, numeric) = match kind {
                    "s" => (
                        v.trim()
                            .parse::<usize>()
                            .ok()
                            .and_then(|i| strings.get(i).cloned())
                            .unwrap_or_default(),
                        false,
                    ),
                    "inlineStr" => (
                        c.descendants()
                            .filter(|t| t.is_element() && t.tag_name().name() == "t")
                            .filter_map(|t| t.text())
                            .collect(),
                        false,
                    ),
                    "b" => ((if v == "1" { "VERO" } else { "FALSO" }).to_string(), false),
                    "str" | "e" => (v.to_string(), false),
                    _ => (number(v), true),
                };
                while cells.len() < col {
                    cells.push((String::new(), false));
                }
                cells.push((value, numeric));
            }
            while matches!(cells.last(), Some((s, _)) if s.is_empty()) {
                cells.pop();
            }
            used_cols = used_cols.max(cells.len());
            grid.push(cells);
        }
        while matches!(grid.last(), Some(r) if r.is_empty()) {
            grid.pop();
        }
        if grid.is_empty() {
            continue;
        }
        let title = names
            .get(n - 1)
            .cloned()
            .unwrap_or_else(|| format!("Foglio {n}"));
        blocks.push(heading(&title, 16.0));
        blocks.push(Block::Space(6.0));
        let rows = grid
            .into_iter()
            .enumerate()
            .map(|(ri, mut cells)| {
                cells.resize(used_cols.max(1), (String::new(), false));
                Row {
                    cells: cells
                        .into_iter()
                        .map(|(text, numeric)| {
                            let style = TextStyle {
                                size: 13.0,
                                bold: ri == 0,
                                ..TextStyle::default()
                            };
                            let blocks = if text.is_empty() {
                                Vec::new()
                            } else {
                                vec![para(
                                    vec![Inline::Text {
                                        text,
                                        style,
                                        link: None,
                                    }],
                                    if numeric { Align::Right } else { Align::Left },
                                )]
                            };
                            cell(blocks, ri == 0)
                        })
                        .collect(),
                }
            })
            .collect();
        blocks.push(Block::Table(Table {
            style: BoxStyle::default(),
            rows,
            spacing: 0.0,
        }));
        blocks.push(Block::Space(18.0));
    }
    while matches!(blocks.last(), Some(Block::Space(_))) {
        blocks.pop();
    }
    (!blocks.is_empty()).then(|| Doc {
        blocks,
        ..Doc::default()
    })
}

/// Excel stores numbers in full precision: show them like a person would (Italian commas).
fn number(v: &str) -> String {
    match v.trim().parse::<f64>() {
        Ok(x) if x.fract().abs() < 1e-9 && x.abs() < 1e15 => format!("{x:.0}"),
        Ok(x) => {
            let s = format!("{x:.4}");
            let s = s.trim_end_matches('0').trim_end_matches('.');
            s.replace('.', ",")
        }
        Err(_) => v.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal stored (uncompressed) zip with the given entries.
    fn zip(files: &[(&str, &str)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, body) in files {
            let offset = out.len() as u32;
            out.extend_from_slice(&[0x50, 0x4b, 0x03, 0x04, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            out.extend_from_slice(&0u32.to_le_bytes()); // crc (not checked)
            out.extend_from_slice(&(body.len() as u32).to_le_bytes());
            out.extend_from_slice(&(body.len() as u32).to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(body.as_bytes());
            central
                .extend_from_slice(&[0x50, 0x4b, 0x01, 0x02, 20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            central.extend_from_slice(&0u32.to_le_bytes());
            central.extend_from_slice(&(body.len() as u32).to_le_bytes());
            central.extend_from_slice(&(body.len() as u32).to_le_bytes());
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            central.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            central.extend_from_slice(&offset.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }
        let cd_offset = out.len() as u32;
        out.extend_from_slice(&central);
        out.extend_from_slice(&[0x50, 0x4b, 0x05, 0x06, 0, 0, 0, 0]);
        out.extend_from_slice(&(files.len() as u16).to_le_bytes());
        out.extend_from_slice(&(files.len() as u16).to_le_bytes());
        out.extend_from_slice(&(central.len() as u32).to_le_bytes());
        out.extend_from_slice(&cd_offset.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }

    #[test]
    fn reads_docx_and_xlsx() {
        let doc = r#"<w:document xmlns:w="w"><w:body>
            <w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Accordo</w:t></w:r></w:p>
            <w:p><w:r><w:rPr><w:b/></w:rPr><w:t>Parti:</w:t></w:r><w:r><w:t xml:space="preserve"> Rossi Srl</w:t></w:r></w:p>
            <w:tbl><w:tr><w:tc><w:p><w:r><w:t>A</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>B</w:t></w:r></w:p></w:tc></w:tr></w:tbl>
            </w:body></w:document>"#;
        let d = docx(&zip(&[("word/document.xml", doc)])).unwrap();
        let text = d.to_text();
        assert!(
            text.contains("Accordo") && text.contains("Parti: Rossi Srl") && text.contains('B'),
            "{text}"
        );
        assert!(d.blocks.iter().any(|b| matches!(b, Block::Table(_))));

        let ss = r#"<sst><si><t>Nome</t></si><si><t>Importo</t></si></sst>"#;
        let sheet = r#"<worksheet><sheetData>
            <row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>1</v></c></row>
            <row r="2"><c r="A2" t="inlineStr"><is><t>Wallbox</t></is></c><c r="C2"><v>1950.5</v></c></row>
            </sheetData></worksheet>"#;
        let wb = r#"<workbook><sheets><sheet name="Listino"/></sheets></workbook>"#;
        let x = xlsx(&zip(&[
            ("xl/sharedStrings.xml", ss),
            ("xl/workbook.xml", wb),
            ("xl/worksheets/sheet1.xml", sheet),
        ]))
        .unwrap();
        let text = x.to_text();
        assert!(
            text.contains("Listino") && text.contains("Importo") && text.contains("1950,5"),
            "{text}"
        );
        assert_eq!(column_of("AB3"), Some(27));
        assert!(docx(b"not a zip").is_none());
        assert!(xlsx(&[0x50, 0x4b, 0x05, 0x06]).is_none());
    }
}
