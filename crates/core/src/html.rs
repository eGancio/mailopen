//! HTML mail → a small document the GUI draws natively: paragraphs of styled runs with links,
//! images, lists, quotes, tables and coloured boxes.
//!
//! A safe subset of HTML and CSS: no scripts, no forms, nothing is fetched here. Styles come from
//! the `style` attribute, from simple selectors in `<style>` (`tag`, `.class`, `tag.class`,
//! `#id`: Outlook's `p.MsoNormal {margin:0}` lives there), and from the presentational attributes
//! mail still relies on (`bgcolor`, `align`, `width`, `cellpadding`, `<font>`).
//!
//! Mail is hostile input: nesting and work are capped, and nothing here panics.

use std::collections::HashSet;

use html5ever::tendril::TendrilSink;
use html5ever::{ParseOpts, parse_document};
use markup5ever_rcdom::{Handle, NodeData, RcDom};

/// Structural nesting (boxes, tables, lists) deeper than this is flattened into its parent.
pub const MAX_DEPTH: usize = 24;
/// Element nesting deeper than this is read as plain text, without recursion.
const MAX_ELEMENT_DEPTH: usize = 160;
/// Elements visited at most; a pathological message stops rendering there.
const MAX_ELEMENTS: usize = 80_000;
/// HTML bytes parsed at most.
const MAX_HTML: usize = 4 * 1024 * 1024;
/// Text size when the message sets none, CSS px.
pub const BASE_SIZE: f32 = 15.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Rgba {
        Rgba { r, g, b, a: 255 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum VAlign {
    Top,
    #[default]
    Middle,
    Bottom,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Length {
    Px(f32),
    Percent(f32),
}

impl Length {
    /// Resolve against the width available.
    pub fn of(self, available: f32) -> f32 {
        match self {
            Length::Px(p) => p,
            Length::Percent(p) => available * p / 100.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextStyle {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub mono: bool,
    /// `None` = the reader's text colour (or the link colour for links).
    pub color: Option<Rgba>,
    pub background: Option<Rgba>,
    /// CSS px.
    pub size: f32,
    /// Multiple of the size, when the message sets one.
    pub line_height: Option<f32>,
}

impl Default for TextStyle {
    fn default() -> Self {
        TextStyle {
            bold: false,
            italic: false,
            underline: false,
            strike: false,
            mono: false,
            color: None,
            background: None,
            size: BASE_SIZE,
            line_height: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    /// `cid:…`, `data:…` or an `http(s)` URL.
    pub src: String,
    pub alt: String,
    pub width: Option<Length>,
    /// CSS px.
    pub height: Option<f32>,
    pub link: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Inline {
    Text {
        text: String,
        style: TextStyle,
        link: Option<String>,
    },
    Image(Image),
    Break,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Paragraph {
    pub inlines: Vec<Inline>,
    pub align: Align,
    /// Whitespace kept as written (`<pre>`, plain text).
    pub pre: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Border {
    pub width: f32,
    pub color: Rgba,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct BoxStyle {
    pub background: Option<Rgba>,
    pub border_top: Option<Border>,
    pub border_right: Option<Border>,
    pub border_bottom: Option<Border>,
    pub border_left: Option<Border>,
    /// Top, right, bottom, left, CSS px.
    pub padding: [f32; 4],
    pub radius: f32,
    pub width: Option<Length>,
    pub max_width: Option<Length>,
    pub min_height: Option<f32>,
    /// Where the box sits in a wider container.
    pub align: Align,
    /// The whole box is a link (a button made of `<a style="background…; padding…">`).
    pub link: Option<String>,
}

impl BoxStyle {
    pub fn has_border(&self) -> bool {
        self.border_top.is_some()
            || self.border_right.is_some()
            || self.border_bottom.is_some()
            || self.border_left.is_some()
    }

    /// Whether drawing it as a box changes anything.
    fn is_visible(&self) -> bool {
        // 1px paddings (the default cellpadding) and full widths only add nesting.
        let narrows = |l: Option<Length>| match l {
            Some(Length::Percent(p)) => p < 99.0,
            Some(Length::Px(_)) => true,
            None => false,
        };
        self.background.is_some()
            || self.has_border()
            || self.padding.iter().any(|p| *p > 1.5)
            || narrows(self.width)
            || narrows(self.max_width)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    pub style: BoxStyle,
    pub valign: VAlign,
    pub colspan: u16,
    pub blocks: Vec<Block>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Row {
    pub cells: Vec<Cell>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Table {
    pub style: BoxStyle,
    pub rows: Vec<Row>,
    /// Gap between cells, CSS px.
    pub spacing: f32,
}

impl Table {
    /// Columns, counting colspans.
    pub fn columns(&self) -> usize {
        self.rows
            .iter()
            .map(|r| r.cells.iter().map(|c| usize::from(c.colspan.max(1))).sum())
            .max()
            .unwrap_or(0)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Para(Paragraph),
    /// Vertical gap, CSS px.
    Space(f32),
    Rule(Option<Rgba>),
    List {
        ordered: bool,
        start: u32,
        marker: bool,
        items: Vec<Vec<Block>>,
    },
    Quote(Vec<Block>),
    Table(Table),
    Box {
        style: BoxStyle,
        blocks: Vec<Block>,
    },
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Doc {
    pub blocks: Vec<Block>,
    /// The page colour (`<body bgcolor>`), when the message sets one.
    pub background: Option<Rgba>,
    /// Images on other servers (blocked until the user allows them).
    pub remote_images: usize,
    /// `cid:` references, lowercased, without angle brackets.
    pub cids: Vec<String>,
    /// The hidden "preheader" senders write for the list preview, when there is one.
    pub preheader: String,
}

impl Doc {
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// Plain text, for quoting in replies and for the list preview.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        text_of_blocks(&self.blocks, &mut out, "");
        tidy_text(&out)
    }
}

// ---------------------------------------------------------------------------------------------
// Entry points

/// Parse an HTML message.
pub fn parse(html: &str) -> Doc {
    let html = match html.char_indices().nth(MAX_HTML) {
        Some((cut, _)) => html.get(..cut).unwrap_or(html),
        None => html,
    };
    let dom = parse_document(RcDom::default(), ParseOpts::default()).one(html);
    let mut css = String::new();
    collect_styles(&dom.document, &mut css);
    let sheet = Sheet::parse(&css);
    let mut p = Parser {
        sheet,
        budget: MAX_ELEMENTS,
        remote_images: 0,
        cids: Vec::new(),
        background: None,
        preheader: String::new(),
        visible: 0,
    };
    let mut b = Builder::default();
    let inh = Inherit::default();
    p.children(&dom.document, &inh, &mut b, 0, 0);
    let blocks = simplify(b.finish());
    Doc {
        blocks,
        background: p.background,
        remote_images: p.remote_images,
        cids: p.cids,
        preheader: p.preheader,
    }
}

/// Collapse whitespace and drop the invisible fillers senders pad previews with.
pub fn clean_line(s: &str) -> String {
    let visible: String = s
        .chars()
        .filter(|c| {
            !matches!(
                c,
                '\u{200b}'
                    | '\u{200c}'
                    | '\u{200d}'
                    | '\u{2060}'
                    | '\u{feff}'
                    | '\u{ad}'
                    | '\u{34f}'
            )
        })
        .map(|c| if c == '\u{a0}' { ' ' } else { c })
        .collect();
    visible.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A plain-text message as a document: paragraphs, `>` quotes, clickable URLs.
pub fn from_plain(text: &str) -> Doc {
    let lines: Vec<&str> = text.lines().collect();
    Doc {
        blocks: plain_blocks(&lines, 0),
        ..Doc::default()
    }
}

fn plain_blocks(lines: &[&str], depth: usize) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    let mut para: Vec<Inline> = Vec::new();
    let mut i = 0usize;
    let flush = |para: &mut Vec<Inline>, out: &mut Vec<Block>| {
        if !para.is_empty() {
            out.push(Block::Para(Paragraph {
                inlines: std::mem::take(para),
                align: Align::Left,
                pre: true,
            }));
        }
    };
    while let Some(line) = lines.get(i) {
        let trimmed = line.trim_start();
        if depth < MAX_DEPTH && trimmed.starts_with('>') {
            flush(&mut para, &mut out);
            let mut quoted = Vec::new();
            while let Some(l) = lines.get(i) {
                let Some(rest) = l.trim_start().strip_prefix('>') else {
                    break;
                };
                quoted.push(rest.strip_prefix(' ').unwrap_or(rest));
                i = i.saturating_add(1);
            }
            out.push(Block::Quote(plain_blocks(&quoted, depth.saturating_add(1))));
            continue;
        }
        if line.trim().is_empty() {
            flush(&mut para, &mut out);
            push_space(&mut out, 10.0);
        } else {
            if !para.is_empty() {
                para.push(Inline::Break);
            }
            linkify(line.trim_end(), &TextStyle::default(), &mut para);
        }
        i = i.saturating_add(1);
    }
    flush(&mut para, &mut out);
    trim_spaces(&mut out);
    out
}

/// Split `text` into runs, making `http(s)://` and `www.` URLs links.
fn linkify(text: &str, style: &TextStyle, out: &mut Vec<Inline>) {
    let mut rest = text;
    while !rest.is_empty() {
        let found = ["https://", "http://", "www."]
            .iter()
            .filter_map(|p| rest.find(p))
            .filter(|i| {
                // `www.` only at a word start.
                *i == 0
                    || rest
                        .get(..*i)
                        .and_then(|b| b.chars().last())
                        .is_some_and(|c| !c.is_alphanumeric() && c != '.' && c != '/')
            })
            .min();
        let Some(start) = found else {
            push_text(out, rest, style, None);
            break;
        };
        let (before, from) = rest.split_at(start);
        push_text(out, before, style, None);
        let end = from
            .find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\''))
            .unwrap_or(from.len());
        let mut url = from.get(..end).unwrap_or(from);
        // Trailing punctuation is the sentence's, not the link's.
        while let Some(c) = url.chars().last() {
            if matches!(c, '.' | ',' | ';' | ':' | ')' | ']' | '!' | '?') {
                url = url
                    .get(..url.len().saturating_sub(c.len_utf8()))
                    .unwrap_or("");
            } else {
                break;
            }
        }
        if url.len() <= "https://".len() {
            push_text(out, from.get(..end.max(1)).unwrap_or(from), style, None);
            rest = from.get(end.max(1)..).unwrap_or("");
            continue;
        }
        let href = if url.starts_with("www.") {
            format!("https://{url}")
        } else {
            url.to_string()
        };
        let mut link_style = style.clone();
        link_style.underline = true;
        push_text(out, url, &link_style, Some(href));
        rest = from.get(url.len()..).unwrap_or("");
    }
}

fn push_text(out: &mut Vec<Inline>, text: &str, style: &TextStyle, link: Option<String>) {
    if text.is_empty() {
        return;
    }
    if let Some(Inline::Text {
        text: t,
        style: s,
        link: l,
    }) = out.last_mut()
        && s == style
        && *l == link
    {
        t.push_str(text);
        return;
    }
    out.push(Inline::Text {
        text: text.to_string(),
        style: style.clone(),
        link,
    });
}

/// The bytes of a `data:` URI (base64 or percent-encoded).
pub fn data_uri_bytes(uri: &str) -> Option<Vec<u8>> {
    let rest = uri.strip_prefix("data:")?;
    let (meta, data) = rest.split_once(',')?;
    if meta.to_ascii_lowercase().ends_with(";base64") {
        let clean: Vec<u8> = data.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
        mail_parser::decoders::base64::base64_decode(&clean)
    } else {
        Some(percent_decode(data))
    }
}

pub fn percent_decode(s: &str) -> Vec<u8> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while let Some(&b) = bytes.get(i) {
        if b == b'%'
            && let (Some(h), Some(l)) = (bytes.get(i + 1), bytes.get(i + 2))
            && let (Some(h), Some(l)) = (hex(*h), hex(*l))
        {
            out.push(h * 16 + l);
            i = i.saturating_add(3);
            continue;
        }
        out.push(b);
        i = i.saturating_add(1);
    }
    out
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// CSS

/// A compound selector: `tag`, `.class`, `tag.class`, `#id`, `tag#id`.
#[derive(Clone, Debug, Default)]
struct Selector {
    tag: Option<String>,
    classes: Vec<String>,
    id: Option<String>,
}

impl Selector {
    fn parse(s: &str) -> Option<Selector> {
        let s = s.trim();
        if s.is_empty()
            || s.contains(|c: char| {
                c.is_whitespace() || matches!(c, '>' | '+' | '~' | ':' | '[' | '*' | '(' | ')')
            })
        {
            return None;
        }
        let mut sel = Selector::default();
        let mut cur = String::new();
        let mut kind = 't';
        for c in s.chars().chain(std::iter::once('\0')) {
            if c == '.' || c == '#' || c == '\0' {
                if !cur.is_empty() {
                    let v = cur.to_ascii_lowercase();
                    match kind {
                        '.' => sel.classes.push(v),
                        '#' => sel.id = Some(v),
                        _ => sel.tag = Some(v),
                    }
                }
                cur.clear();
                kind = c;
            } else {
                cur.push(c);
            }
        }
        (sel.tag.is_some() || !sel.classes.is_empty() || sel.id.is_some()).then_some(sel)
    }

    fn specificity(&self) -> u32 {
        let ids = u32::from(self.id.is_some());
        let classes = u32::try_from(self.classes.len()).unwrap_or(u32::MAX);
        let tags = u32::from(self.tag.is_some());
        ids.saturating_mul(10_000)
            .saturating_add(classes.saturating_mul(100))
            .saturating_add(tags)
    }

    fn matches(&self, tag: &str, classes: &[String], id: Option<&str>) -> bool {
        self.tag.as_deref().is_none_or(|t| t == tag)
            && self.id.as_deref().is_none_or(|i| Some(i) == id)
            && self.classes.iter().all(|c| classes.contains(c))
    }
}

#[derive(Clone, Debug)]
struct Rule {
    selector: Selector,
    decls: Vec<(String, String)>,
    order: usize,
}

#[derive(Clone, Debug, Default)]
struct Sheet {
    rules: Vec<Rule>,
}

impl Sheet {
    fn parse(css: &str) -> Sheet {
        let css = strip_css_comments(css);
        let mut rules = Vec::new();
        let mut rest = css.as_str();
        while let Some(open) = rest.find('{') {
            let head = rest.get(..open).unwrap_or("").trim();
            let Some(body_and_after) = rest.get(open + 1..) else {
                break;
            };
            if head.starts_with('@') {
                // @media, @font-face, @supports…: skip the whole (possibly nested) block.
                let mut depth = 1usize;
                let mut end = body_and_after.len();
                for (i, c) in body_and_after.char_indices() {
                    match c {
                        '{' => depth = depth.saturating_add(1),
                        '}' => {
                            depth = depth.saturating_sub(1);
                            if depth == 0 {
                                end = i + 1;
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                rest = body_and_after.get(end..).unwrap_or("");
                continue;
            }
            let close = body_and_after.find('}').unwrap_or(body_and_after.len());
            let body = body_and_after.get(..close).unwrap_or("");
            let decls = parse_decls(body);
            if !decls.is_empty() {
                for sel in head.split(',').filter_map(Selector::parse) {
                    rules.push(Rule {
                        selector: sel,
                        decls: decls.clone(),
                        order: rules.len(),
                    });
                }
            }
            rest = body_and_after.get(close.saturating_add(1)..).unwrap_or("");
            if rules.len() > 4000 {
                break;
            }
        }
        Sheet { rules }
    }

    fn matching(&self, tag: &str, classes: &[String], id: Option<&str>) -> Vec<&Rule> {
        let mut hits: Vec<&Rule> = self
            .rules
            .iter()
            .filter(|r| r.selector.matches(tag, classes, id))
            .collect();
        hits.sort_by_key(|r| (r.selector.specificity(), r.order));
        hits
    }
}

fn strip_css_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(i) = rest.find("/*") {
        out.push_str(rest.get(..i).unwrap_or(""));
        match rest
            .get(i + 2..)
            .and_then(|r| r.find("*/").map(|j| i + 2 + j + 2))
        {
            Some(end) => rest = rest.get(end..).unwrap_or(""),
            None => {
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out.replace("<!--", "").replace("-->", "")
}

/// `a: b; c: d !important` → [("a","b"), ("c","d")], names lowercased.
fn parse_decls(s: &str) -> Vec<(String, String)> {
    s.split(';')
        .filter_map(|d| {
            let (k, v) = d.split_once(':')?;
            let k = k.trim().to_ascii_lowercase();
            let v = v.trim();
            let v = v
                .strip_suffix("!important")
                .map(str::trim_end)
                .unwrap_or(v)
                .to_string();
            (!k.is_empty() && !v.is_empty()).then_some((k, v))
        })
        .collect()
}

/// The declarations that apply to one element, last one wins.
#[derive(Clone, Debug, Default)]
struct Css {
    decls: Vec<(String, String)>,
}

impl Css {
    fn get(&self, name: &str) -> Option<&str> {
        self.decls
            .iter()
            .rev()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    fn set(&mut self, name: &str, value: impl Into<String>) {
        self.decls.push((name.to_string(), value.into()));
    }

    /// One side of a 1–4 value shorthand (`padding`, `margin`): 0 top, 1 right, 2 bottom, 3 left.
    fn side(&self, shorthand: &str, side: usize) -> Option<&str> {
        const NAMES: [&str; 4] = ["top", "right", "bottom", "left"];
        let long = format!("{shorthand}-{}", NAMES.get(side)?);
        // The later of the long form and the shorthand wins.
        let pos_long = self.decls.iter().rposition(|(k, _)| *k == long);
        let pos_short = self.decls.iter().rposition(|(k, _)| k == shorthand);
        match (pos_long, pos_short) {
            (Some(l), Some(s)) if l > s => self.decls.get(l).map(|(_, v)| v.as_str()),
            (Some(l), None) => self.decls.get(l).map(|(_, v)| v.as_str()),
            (_, Some(s)) => {
                let v = self.decls.get(s).map(|(_, v)| v.as_str())?;
                let parts: Vec<&str> = v.split_whitespace().collect();
                let idx = match (parts.len(), side) {
                    (1, _) => 0,
                    (2, 0 | 2) => 0,
                    (2, _) => 1,
                    (3, 0) => 0,
                    (3, 1 | 3) => 1,
                    (3, _) => 2,
                    (_, s) => s,
                };
                parts.get(idx).copied()
            }
            (None, None) => None,
        }
    }
}

fn parse_color(v: &str) -> Option<Rgba> {
    let v = v.trim().to_ascii_lowercase();
    if let Some(hexs) = v.strip_prefix('#') {
        let digits: Vec<u8> = hexs.bytes().filter_map(hex).collect();
        if digits.len() != hexs.len() {
            return None;
        }
        return match digits.as_slice() {
            [r, g, b] => Some(Rgba::rgb(r * 17, g * 17, b * 17)),
            [r, g, b, _] => Some(Rgba::rgb(r * 17, g * 17, b * 17)),
            [r1, r2, g1, g2, b1, b2] | [r1, r2, g1, g2, b1, b2, _, _] => {
                Some(Rgba::rgb(r1 * 16 + r2, g1 * 16 + g2, b1 * 16 + b2))
            }
            _ => None,
        };
    }
    if let Some(inner) = v
        .strip_prefix("rgba(")
        .or_else(|| v.strip_prefix("rgb("))
        .and_then(|s| s.strip_suffix(')'))
    {
        let parts: Vec<f32> = inner
            .split([',', ' ', '/'])
            .filter(|s| !s.is_empty())
            .filter_map(|s| {
                let s = s.trim();
                match s.strip_suffix('%') {
                    Some(p) => p.parse::<f32>().ok().map(|p| p * 2.55),
                    None => s.parse::<f32>().ok(),
                }
            })
            .collect();
        let c = |i: usize| parts.get(i).map(|x| x.clamp(0.0, 255.0) as u8);
        let alpha = parts.get(3).copied().unwrap_or(1.0);
        if alpha <= 0.05 {
            return None;
        }
        return Some(Rgba::rgb(c(0)?, c(1)?, c(2)?));
    }
    let named = match v.as_str() {
        "black" => Rgba::rgb(0, 0, 0),
        "white" => Rgba::rgb(255, 255, 255),
        "red" => Rgba::rgb(255, 0, 0),
        "green" => Rgba::rgb(0, 128, 0),
        "blue" => Rgba::rgb(0, 0, 255),
        "navy" => Rgba::rgb(0, 0, 128),
        "gray" | "grey" => Rgba::rgb(128, 128, 128),
        "darkgray" | "darkgrey" => Rgba::rgb(169, 169, 169),
        "lightgray" | "lightgrey" => Rgba::rgb(211, 211, 211),
        "silver" => Rgba::rgb(192, 192, 192),
        "whitesmoke" => Rgba::rgb(245, 245, 245),
        "gainsboro" => Rgba::rgb(220, 220, 220),
        "maroon" => Rgba::rgb(128, 0, 0),
        "purple" => Rgba::rgb(128, 0, 128),
        "teal" => Rgba::rgb(0, 128, 128),
        "olive" => Rgba::rgb(128, 128, 0),
        "orange" => Rgba::rgb(255, 165, 0),
        "yellow" => Rgba::rgb(255, 255, 0),
        "lime" => Rgba::rgb(0, 255, 0),
        "aqua" | "cyan" => Rgba::rgb(0, 255, 255),
        "fuchsia" | "magenta" => Rgba::rgb(255, 0, 255),
        "darkblue" => Rgba::rgb(0, 0, 139),
        "darkred" => Rgba::rgb(139, 0, 0),
        "darkgreen" => Rgba::rgb(0, 100, 0),
        "dimgray" | "dimgrey" => Rgba::rgb(105, 105, 105),
        _ => return None,
    };
    Some(named)
}

/// A CSS length in px. `em` against `font`, `%` against `percent_of` (when there is one).
fn parse_px(v: &str, font: f32, percent_of: Option<f32>) -> Option<f32> {
    let v = v.trim().to_ascii_lowercase();
    let num = |s: &str| s.trim().parse::<f32>().ok().filter(|x| x.is_finite());
    let px = if let Some(n) = v.strip_suffix("px") {
        num(n)?
    } else if let Some(n) = v.strip_suffix("pt") {
        num(n)? * 4.0 / 3.0
    } else if let Some(n) = v.strip_suffix("rem") {
        num(n)? * 16.0
    } else if let Some(n) = v.strip_suffix("em") {
        num(n)? * font
    } else if let Some(n) = v.strip_suffix("ex") {
        num(n)? * font * 0.5
    } else if let Some(n) = v.strip_suffix("cm") {
        num(n)? * 96.0 / 2.54
    } else if let Some(n) = v.strip_suffix("mm") {
        num(n)? * 96.0 / 25.4
    } else if let Some(n) = v.strip_suffix("in") {
        num(n)? * 96.0
    } else if let Some(n) = v.strip_suffix('%') {
        num(n)? * percent_of? / 100.0
    } else {
        num(&v)?
    };
    Some(px.clamp(-4000.0, 4000.0))
}

fn parse_length(v: &str, font: f32) -> Option<Length> {
    let t = v.trim();
    if let Some(p) = t.strip_suffix('%') {
        return p
            .trim()
            .parse::<f32>()
            .ok()
            .filter(|x| x.is_finite() && *x > 0.0)
            .map(|p| Length::Percent(p.min(100.0)));
    }
    parse_px(t, font, None).filter(|p| *p > 0.0).map(Length::Px)
}

fn font_size(v: &str, parent: f32) -> Option<f32> {
    let size = match v.trim().to_ascii_lowercase().as_str() {
        "xx-small" => 9.0,
        "x-small" => 10.0,
        "small" => 13.0,
        "medium" => 16.0,
        "large" => 18.0,
        "x-large" => 24.0,
        "xx-large" => 32.0,
        "smaller" => parent * 0.83,
        "larger" => parent * 1.2,
        other => parse_px(other, parent, Some(parent))?,
    };
    Some(size.clamp(0.0, 64.0))
}

/// `<font size=1..7>` in px.
fn font_tag_size(v: &str, parent: f32) -> Option<f32> {
    let t = v.trim();
    let n: i32 = if let Some(r) = t.strip_prefix('+') {
        3 + r.parse::<i32>().ok()?
    } else if let Some(r) = t.strip_prefix('-') {
        3 - r.parse::<i32>().ok()?
    } else {
        t.parse().ok()?
    };
    let _ = parent;
    Some(match n.clamp(1, 7) {
        1 => 10.0,
        2 => 13.0,
        3 => 16.0,
        4 => 18.0,
        5 => 24.0,
        6 => 32.0,
        _ => 48.0,
    })
}

fn parse_border(v: &str, font: f32) -> Option<Border> {
    let mut width = None;
    let mut color = None;
    let mut style_none = false;
    for part in v.split_whitespace() {
        let p = part.to_ascii_lowercase();
        match p.as_str() {
            "none" | "hidden" => style_none = true,
            "solid" | "dashed" | "dotted" | "double" | "groove" | "ridge" | "inset" | "outset" => {}
            "thin" => width = Some(1.0),
            "medium" => width = Some(3.0),
            "thick" => width = Some(5.0),
            _ => {
                if let Some(w) = parse_px(&p, font, None) {
                    width = Some(w);
                } else if let Some(c) = parse_color(&p) {
                    color = Some(c);
                }
            }
        }
    }
    let width = width.unwrap_or(if style_none { 0.0 } else { 1.0 });
    (!style_none && width > 0.0).then(|| Border {
        width: width.min(12.0),
        color: color.unwrap_or(Rgba::rgb(0xCC, 0xCC, 0xCC)),
    })
}

fn parse_align(v: &str) -> Option<Align> {
    match v.trim().to_ascii_lowercase().as_str() {
        "left" | "start" | "justify" => Some(Align::Left),
        "center" | "middle" | "-webkit-center" | "-moz-center" => Some(Align::Center),
        "right" | "end" => Some(Align::Right),
        _ => None,
    }
}

fn parse_valign(v: &str) -> Option<VAlign> {
    match v.trim().to_ascii_lowercase().as_str() {
        "top" | "text-top" => Some(VAlign::Top),
        "middle" | "center" => Some(VAlign::Middle),
        "bottom" | "text-bottom" | "baseline" => Some(VAlign::Bottom),
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// DOM walk

#[derive(Clone, Debug, Default)]
struct Inherit {
    style: TextStyle,
    align: Align,
    link: Option<String>,
    pre: bool,
}

/// Collects inline content into paragraphs and paragraphs into blocks.
#[derive(Default)]
struct Builder {
    blocks: Vec<Block>,
    line: Vec<Inline>,
    line_align: Align,
    line_pre: bool,
}

impl Builder {
    fn text(&mut self, raw: &str, inh: &Inherit) {
        if inh.style.size <= 0.5 {
            return;
        }
        if inh.pre {
            self.start_line(inh);
            for (i, part) in raw.split('\n').enumerate() {
                if i > 0 {
                    self.line.push(Inline::Break);
                }
                let part = part.trim_end_matches('\r');
                push_text(&mut self.line, part, &inh.style, inh.link.clone());
            }
            return;
        }
        let mut out = String::with_capacity(raw.len());
        let mut last_space = self.ends_with_space();
        for c in raw.chars() {
            match c {
                '\u{200b}' | '\u{200c}' | '\u{200d}' | '\u{2060}' | '\u{feff}' | '\u{ad}'
                | '\u{34f}' => {}
                '\u{a0}' => {
                    out.push('\u{a0}');
                    last_space = false;
                }
                c if c.is_whitespace() => {
                    if !last_space {
                        out.push(' ');
                        last_space = true;
                    }
                }
                c if c.is_control() => {}
                c => {
                    out.push(c);
                    last_space = false;
                }
            }
        }
        if out.is_empty() {
            return;
        }
        if out == " " && self.line.is_empty() {
            return;
        }
        self.start_line(inh);
        push_text(&mut self.line, &out, &inh.style, inh.link.clone());
    }

    fn ends_with_space(&self) -> bool {
        match self.line.last() {
            None | Some(Inline::Break) => true,
            Some(Inline::Text { text, .. }) => text.ends_with(' '),
            Some(Inline::Image(_)) => false,
        }
    }

    fn start_line(&mut self, inh: &Inherit) {
        if self.line.is_empty() {
            self.line_align = inh.align;
            self.line_pre = inh.pre;
        }
    }

    fn inline(&mut self, i: Inline, inh: &Inherit) {
        self.start_line(inh);
        self.line.push(i);
    }

    fn flush(&mut self) {
        if self.line.is_empty() {
            return;
        }
        let mut inlines = std::mem::take(&mut self.line);
        // One trailing <br> is invisible in a browser.
        if matches!(inlines.last(), Some(Inline::Break)) {
            inlines.pop();
        }
        if !self.line_pre {
            trim_inlines(&mut inlines);
        }
        let has_image = inlines.iter().any(|i| matches!(i, Inline::Image(_)));
        let visible = inlines.iter().any(|i| match i {
            Inline::Text { text, .. } => text.chars().any(|c| !c.is_whitespace()),
            Inline::Image(_) => true,
            Inline::Break => false,
        });
        if !visible && !has_image {
            // `<p>&nbsp;</p>` and `<br><br>` are blank lines on purpose.
            let size = inlines
                .iter()
                .find_map(|i| match i {
                    Inline::Text { style, .. } => Some(style.size),
                    _ => None,
                })
                .unwrap_or(BASE_SIZE);
            let nbsp = inlines.iter().any(|i| match i {
                Inline::Text { text, .. } => text.contains('\u{a0}'),
                _ => false,
            });
            let breaks = inlines
                .iter()
                .filter(|i| matches!(i, Inline::Break))
                .count();
            if nbsp || breaks > 0 {
                let lines = breaks.max(1) as f32;
                push_space(&mut self.blocks, size * 1.3 * lines);
            }
            return;
        }
        self.blocks.push(Block::Para(Paragraph {
            inlines,
            align: self.line_align,
            pre: self.line_pre,
        }));
    }

    fn space(&mut self, px: f32) {
        self.flush();
        push_space(&mut self.blocks, px);
    }

    fn block(&mut self, b: Block) {
        self.flush();
        self.blocks.push(b);
    }

    fn finish(mut self) -> Vec<Block> {
        self.flush();
        trim_spaces(&mut self.blocks);
        self.blocks
    }
}

fn push_space(blocks: &mut Vec<Block>, px: f32) {
    let px = px.clamp(0.0, 80.0);
    if px < 1.0 {
        return;
    }
    if let Some(Block::Space(prev)) = blocks.last_mut() {
        *prev = prev.max(px);
    } else {
        blocks.push(Block::Space(px));
    }
}

fn trim_spaces(blocks: &mut Vec<Block>) {
    while matches!(blocks.last(), Some(Block::Space(_))) {
        blocks.pop();
    }
    while matches!(blocks.first(), Some(Block::Space(_))) {
        blocks.remove(0);
    }
}

/// Drop leading and trailing spaces of a paragraph and around line breaks.
fn trim_inlines(inlines: &mut Vec<Inline>) {
    let mut at_line_start = true;
    for i in 0..inlines.len() {
        let next_is_break = matches!(inlines.get(i + 1), Some(Inline::Break) | None);
        match inlines.get_mut(i) {
            Some(Inline::Text { text, .. }) => {
                if at_line_start {
                    *text = text.trim_start_matches(' ').to_string();
                }
                if next_is_break {
                    *text = text.trim_end_matches(' ').to_string();
                }
                at_line_start = text.is_empty() && at_line_start;
            }
            Some(Inline::Break) => at_line_start = true,
            Some(Inline::Image(_)) => at_line_start = false,
            None => {}
        }
    }
    inlines.retain(|i| !matches!(i, Inline::Text { text, .. } if text.is_empty()));
}

struct Parser {
    sheet: Sheet,
    budget: usize,
    remote_images: usize,
    cids: Vec<String>,
    background: Option<Rgba>,
    preheader: String,
    /// Visible characters met so far (a preheader comes before any).
    visible: usize,
}

struct Element<'a> {
    tag: String,
    attrs: Vec<(String, String)>,
    node: &'a Handle,
}

impl Element<'_> {
    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

fn element(node: &Handle) -> Option<Element<'_>> {
    let NodeData::Element { name, attrs, .. } = &node.data else {
        return None;
    };
    let tag = name.local.as_ref().to_ascii_lowercase();
    let attrs = attrs
        .borrow()
        .iter()
        .map(|a| {
            (
                a.name.local.as_ref().to_ascii_lowercase(),
                a.value.to_string(),
            )
        })
        .collect();
    Some(Element { tag, attrs, node })
}

/// Text of every `<style>` element.
fn collect_styles(node: &Handle, out: &mut String) {
    let mut stack = vec![node.clone()];
    let mut seen = 0usize;
    while let Some(n) = stack.pop() {
        seen = seen.saturating_add(1);
        if seen > MAX_ELEMENTS {
            break;
        }
        if let Some(el) = element(&n)
            && el.tag == "style"
        {
            for c in n.children.borrow().iter() {
                if let NodeData::Text { contents } = &c.data {
                    out.push_str(&contents.borrow());
                    out.push('\n');
                }
            }
            continue;
        }
        for c in n.children.borrow().iter().rev() {
            stack.push(c.clone());
        }
        if out.len() > 512 * 1024 {
            break;
        }
    }
}

/// Every text node below `node`, iteratively (for content nested too deep to walk).
fn flat_text(node: &Handle) -> String {
    let mut out = String::new();
    let mut stack = vec![node.clone()];
    while let Some(n) = stack.pop() {
        if let NodeData::Text { contents } = &n.data {
            out.push_str(&contents.borrow());
            out.push(' ');
        }
        for c in n.children.borrow().iter().rev() {
            stack.push(c.clone());
        }
        if out.len() > 256 * 1024 {
            break;
        }
    }
    out
}

const SKIP: &[&str] = &[
    "head", "style", "script", "title", "meta", "link", "noscript", "template", "xml", "svg",
    "math", "object", "embed", "iframe", "frame", "frameset", "input", "select", "textarea",
    "audio", "video", "canvas", "map", "area", "base", "param", "source", "track", "wbr",
];

const BLOCKS: &[&str] = &[
    "p",
    "div",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "center",
    "section",
    "article",
    "header",
    "footer",
    "main",
    "nav",
    "aside",
    "address",
    "figure",
    "figcaption",
    "form",
    "fieldset",
    "legend",
    "dl",
    "dt",
    "dd",
    "details",
    "summary",
    "body",
    "html",
    "li",
    "caption",
    "hgroup",
];

impl Parser {
    fn children(
        &mut self,
        node: &Handle,
        inh: &Inherit,
        b: &mut Builder,
        depth: usize,
        edepth: usize,
    ) {
        let kids: Vec<Handle> = node.children.borrow().iter().cloned().collect();
        for c in &kids {
            if self.budget == 0 {
                return;
            }
            self.node(c, inh, b, depth, edepth);
        }
    }

    fn node(&mut self, node: &Handle, inh: &Inherit, b: &mut Builder, depth: usize, edepth: usize) {
        match &node.data {
            NodeData::Text { contents } => {
                let text = contents.borrow();
                if inh.style.size > 0.5 {
                    self.visible = self
                        .visible
                        .saturating_add(text.chars().filter(|c| c.is_alphanumeric()).count());
                }
                b.text(&text, inh);
            }
            NodeData::Element { .. } => {
                self.budget = self.budget.saturating_sub(1);
                if edepth >= MAX_ELEMENT_DEPTH {
                    b.text(&flat_text(node), inh);
                    return;
                }
                if let Some(el) = element(node) {
                    self.element(&el, inh, b, depth, edepth.saturating_add(1));
                }
            }
            NodeData::Document => self.children(node, inh, b, depth, edepth),
            _ => {}
        }
    }

    /// The CSS that applies to `el`: presentational attributes, then the style sheet, then the
    /// `style` attribute.
    fn css(&self, el: &Element<'_>, inh: &Inherit) -> Css {
        let mut css = Css::default();
        if let Some(c) = el.attr("bgcolor") {
            css.set("background-color", c);
        }
        match el.tag.as_str() {
            "font" => {
                if let Some(c) = el.attr("color") {
                    css.set("color", c);
                }
                if let Some(s) = el
                    .attr("size")
                    .and_then(|s| font_tag_size(s, inh.style.size))
                {
                    css.set("font-size", format!("{s}px"));
                }
            }
            "table" | "img" => {
                if let Some(a) = el.attr("align") {
                    css.set("-mo-box-align", a);
                }
            }
            _ => {
                if let Some(a) = el.attr("align") {
                    css.set("text-align", a);
                }
            }
        }
        for name in ["width", "height"] {
            if let Some(v) = el.attr(name) {
                let v = v.trim();
                if v.ends_with('%') || v.ends_with("px") {
                    css.set(name, v);
                } else if !v.is_empty() {
                    css.set(name, format!("{v}px"));
                }
            }
        }
        if let Some(v) = el.attr("valign") {
            css.set("vertical-align", v);
        }
        if el.tag == "body"
            && let Some(c) = el.attr("text")
        {
            css.set("color", c);
        }
        if !self.sheet.rules.is_empty() {
            let classes: Vec<String> = el
                .attr("class")
                .map(|c| {
                    c.split_whitespace()
                        .map(|s| s.to_ascii_lowercase())
                        .collect()
                })
                .unwrap_or_default();
            let id = el.attr("id").map(|s| s.to_ascii_lowercase());
            for rule in self.sheet.matching(&el.tag, &classes, id.as_deref()) {
                css.decls.extend(rule.decls.iter().cloned());
            }
        }
        if let Some(s) = el.attr("style") {
            css.decls.extend(parse_decls(s));
        }
        css
    }

    fn element(
        &mut self,
        el: &Element<'_>,
        inh: &Inherit,
        b: &mut Builder,
        depth: usize,
        edepth: usize,
    ) {
        let tag = el.tag.as_str();
        if SKIP.contains(&tag) || tag.starts_with("v:") || tag.starts_with("w:") {
            return;
        }
        let css = self.css(el, inh);
        if is_hidden(&css, inh.style.size) {
            if self.preheader.is_empty() && self.visible == 0 {
                let t: String = clean_line(&flat_text(el.node)).chars().take(300).collect();
                if t.chars().filter(|c| c.is_alphanumeric()).count() >= 4 {
                    self.preheader = t;
                }
            }
            return;
        }
        let mut inh2 = inherit(&css, inh, el);
        if tag == "body" {
            self.background = css
                .get("background-color")
                .or_else(|| css.get("background"))
                .and_then(background_color);
        }
        let display = css.get("display").map(|d| d.trim().to_ascii_lowercase());
        let display_block = matches!(
            display.as_deref(),
            Some("block" | "inline-block" | "table" | "flex" | "list-item")
        );
        match tag {
            "br" => b.inline(Inline::Break, inh),
            "img" => self.image(el, &css, inh, b, display_block),
            "hr" => {
                b.space(6.0);
                let color = css
                    .get("border-top")
                    .or_else(|| css.get("border"))
                    .and_then(|v| parse_border(v, inh.style.size))
                    .map(|bd| bd.color)
                    .or_else(|| css.get("background-color").and_then(parse_color))
                    .or_else(|| css.get("color").and_then(parse_color));
                b.block(Block::Rule(color));
                b.space(6.0);
            }
            "table" => {
                if depth >= MAX_DEPTH {
                    b.flush();
                    self.children(el.node, &inh2, b, depth, edepth);
                    b.flush();
                    return;
                }
                let margins = margins(&css, inh.style.size);
                b.space(margins.0);
                if let Some(t) = self.table(el, &css, &inh2, depth, edepth) {
                    b.block(t);
                }
                b.space(margins.1);
            }
            "ul" | "ol" => {
                let (top, bottom) = margins_or(&css, inh.style.size, 0.6, 0.6);
                b.space(top);
                if depth >= MAX_DEPTH {
                    self.children(el.node, &inh2, b, depth, edepth);
                    b.flush();
                } else {
                    let list = self.list(el, &css, &inh2, depth, edepth);
                    b.block(list);
                }
                b.space(bottom);
            }
            "blockquote" => {
                let (top, bottom) = margins_or(&css, inh.style.size, 0.6, 0.6);
                b.space(top);
                if depth >= MAX_DEPTH {
                    self.children(el.node, &inh2, b, depth, edepth);
                    b.flush();
                } else {
                    let mut child = Builder::default();
                    self.children(el.node, &inh2, &mut child, depth.saturating_add(1), edepth);
                    let blocks = child.finish();
                    if !blocks.is_empty() {
                        b.block(Block::Quote(blocks));
                    }
                }
                b.space(bottom);
            }
            "pre" => {
                inh2.pre = true;
                inh2.style.mono = true;
                let (top, bottom) = margins_or(&css, inh.style.size, 0.6, 0.6);
                b.space(top);
                self.children(el.node, &inh2, b, depth, edepth);
                b.space(bottom);
            }
            "a" if display_block || has_button_look(&css) => {
                self.block_element(el, &css, inh, inh2, b, depth, edepth, (0.0, 0.0));
            }
            _ if BLOCKS.contains(&tag) || display_block => {
                let default = default_margins(tag);
                let m = margins_or(&css, inh2.style.size, default.0, default.1);
                if tag == "center" {
                    inh2.align = Align::Center;
                }
                self.block_element(el, &css, inh, inh2, b, depth, edepth, m);
            }
            _ => self.children(el.node, &inh2, b, depth, edepth),
        }
    }

    /// A block element: a paragraph boundary, and a box when it draws something.
    #[allow(clippy::too_many_arguments)]
    fn block_element(
        &mut self,
        el: &Element<'_>,
        css: &Css,
        inh: &Inherit,
        inh2: Inherit,
        b: &mut Builder,
        depth: usize,
        edepth: usize,
        margins: (f32, f32),
    ) {
        b.space(margins.0);
        let mut style = box_style(css, inh, &inh2);
        if el.tag == "a" {
            style.link = inh2.link.clone();
        }
        if style.is_visible() && depth < MAX_DEPTH && el.tag != "body" && el.tag != "html" {
            let mut child = Builder::default();
            self.children(el.node, &inh2, &mut child, depth.saturating_add(1), edepth);
            let blocks = child.finish();
            if !blocks.is_empty() || style.min_height.is_some() {
                b.block(Block::Box { style, blocks });
            }
        } else {
            b.flush();
            self.children(el.node, &inh2, b, depth, edepth);
            b.flush();
            if let Some(h) = style.min_height
                && b.blocks.last().is_none_or(|l| matches!(l, Block::Space(_)))
            {
                push_space(&mut b.blocks, h);
            }
        }
        b.space(margins.1);
    }

    fn image(&mut self, el: &Element<'_>, css: &Css, inh: &Inherit, b: &mut Builder, block: bool) {
        let src = el.attr("src").unwrap_or("").trim().to_string();
        let width = css
            .get("width")
            .and_then(|v| parse_length(v, inh.style.size));
        let height = css
            .get("height")
            .and_then(|v| parse_px(v, inh.style.size, None))
            .filter(|h| *h > 0.0);
        let tiny = |l: Option<Length>| matches!(l, Some(Length::Px(p)) if p <= 3.0);
        if tiny(width) || height.is_some_and(|h| h <= 3.0) {
            return; // tracking pixel or spacer
        }
        let lower = src.to_ascii_lowercase();
        if let Some(cid) = lower.strip_prefix("cid:") {
            let cid = cid.trim_matches(['<', '>']).to_string();
            if !self.cids.contains(&cid) {
                self.cids.push(cid);
            }
        } else if lower.starts_with("http://")
            || lower.starts_with("https://")
            || lower.starts_with("//")
        {
            self.remote_images = self.remote_images.saturating_add(1);
        } else if !lower.starts_with("data:image/") {
            return;
        }
        let src = if lower.starts_with("//") {
            format!("https:{src}")
        } else {
            src
        };
        let img = Image {
            src,
            alt: el.attr("alt").unwrap_or("").trim().to_string(),
            width,
            height,
            link: inh.link.clone(),
        };
        let box_align = css
            .get("-mo-box-align")
            .and_then(parse_align)
            .or_else(|| margin_auto(css));
        if block || box_align.is_some() {
            b.flush();
            let mut inh_img = inh.clone();
            if let Some(a) = box_align {
                inh_img.align = a;
            }
            b.inline(Inline::Image(img), &inh_img);
            b.flush();
        } else {
            b.inline(Inline::Image(img), inh);
        }
    }

    fn list(
        &mut self,
        el: &Element<'_>,
        css: &Css,
        inh: &Inherit,
        depth: usize,
        edepth: usize,
    ) -> Block {
        let ordered = el.tag == "ol";
        let start = el
            .attr("start")
            .and_then(|s| s.trim().parse::<u32>().ok())
            .unwrap_or(1);
        let marker = css
            .get("list-style-type")
            .or_else(|| css.get("list-style"))
            .is_none_or(|v| !v.to_ascii_lowercase().contains("none"));
        let mut items = Vec::new();
        let kids: Vec<Handle> = el.node.children.borrow().iter().cloned().collect();
        let mut loose = Builder::default();
        for k in &kids {
            if self.budget == 0 {
                break;
            }
            match element(k) {
                Some(li) if li.tag == "li" => {
                    let blocks = loose.finish();
                    if !blocks.is_empty() {
                        items.push(blocks);
                    }
                    loose = Builder::default();
                    self.budget = self.budget.saturating_sub(1);
                    let li_css = self.css(&li, inh);
                    if is_hidden(&li_css, inh.style.size) {
                        continue;
                    }
                    let li_inh = inherit(&li_css, inh, &li);
                    let mut child = Builder::default();
                    self.children(
                        li.node,
                        &li_inh,
                        &mut child,
                        depth.saturating_add(1),
                        edepth.saturating_add(1),
                    );
                    items.push(child.finish());
                }
                _ => self.node(k, inh, &mut loose, depth.saturating_add(1), edepth),
            }
        }
        let blocks = loose.finish();
        if !blocks.is_empty() {
            items.push(blocks);
        }
        Block::List {
            ordered,
            start,
            marker,
            items,
        }
    }

    fn table(
        &mut self,
        el: &Element<'_>,
        css: &Css,
        inh: &Inherit,
        depth: usize,
        edepth: usize,
    ) -> Option<Block> {
        let font = inh.style.size;
        let cellpadding = el
            .attr("cellpadding")
            .and_then(|v| v.trim().parse::<f32>().ok())
            .unwrap_or(1.0)
            .clamp(0.0, 40.0);
        let spacing = el
            .attr("cellspacing")
            .and_then(|v| v.trim().parse::<f32>().ok())
            .or_else(|| {
                css.get("border-collapse")
                    .filter(|v| v.contains("collapse"))
                    .map(|_| 0.0)
            })
            .or_else(|| {
                css.get("border-spacing")
                    .and_then(|v| parse_px(v, font, None))
            })
            .unwrap_or(2.0)
            .clamp(0.0, 20.0);
        let grid = el
            .attr("border")
            .and_then(|v| v.trim().parse::<f32>().ok())
            .filter(|w| *w > 0.0)
            .map(|_| Border {
                width: 1.0,
                color: Rgba::rgb(0xCC, 0xCC, 0xCC),
            });
        let mut style = box_style(css, inh, inh);
        style.align = css
            .get("-mo-box-align")
            .and_then(parse_align)
            .or_else(|| margin_auto(css))
            .unwrap_or(inh.align);
        // A table's padding does not apply in mail clients; keep its background and border.
        style.padding = [0.0; 4];
        let mut rows = Vec::new();
        let mut stack: Vec<Handle> = el.node.children.borrow().iter().rev().cloned().collect();
        while let Some(n) = stack.pop() {
            if self.budget == 0 {
                break;
            }
            let Some(child) = element(&n) else { continue };
            match child.tag.as_str() {
                "thead" | "tbody" | "tfoot" => {
                    let rcss = self.css(&child, inh);
                    if is_hidden(&rcss, font) {
                        continue;
                    }
                    for k in n.children.borrow().iter().rev() {
                        stack.push(k.clone());
                    }
                }
                "tr" => {
                    self.budget = self.budget.saturating_sub(1);
                    if let Some(row) = self.row(
                        &child,
                        inh,
                        cellpadding,
                        grid,
                        depth,
                        edepth.saturating_add(1),
                    ) {
                        rows.push(row);
                    }
                }
                "caption" => {}
                _ => {}
            }
        }
        if rows.is_empty() {
            return None;
        }
        // A one-cell table is a box.
        if let [row] = rows.as_slice()
            && let [cell] = row.cells.as_slice()
        {
            let mut s = cell.style.clone();
            s.width = style.width.or(s.width);
            s.max_width = style.max_width.or(s.max_width);
            s.align = style.align;
            if s.background.is_none() {
                s.background = style.background;
            }
            if !s.has_border() {
                s.border_top = style.border_top;
                s.border_right = style.border_right;
                s.border_bottom = style.border_bottom;
                s.border_left = style.border_left;
            }
            if s.radius == 0.0 {
                s.radius = style.radius;
            }
            return Some(Block::Box {
                style: s,
                blocks: cell.blocks.clone(),
            });
        }
        Some(Block::Table(Table {
            style,
            rows,
            spacing,
        }))
    }

    fn row(
        &mut self,
        tr: &Element<'_>,
        inh: &Inherit,
        cellpadding: f32,
        grid: Option<Border>,
        depth: usize,
        edepth: usize,
    ) -> Option<Row> {
        let rcss = self.css(tr, inh);
        if is_hidden(&rcss, inh.style.size) {
            return None;
        }
        let row_inh = inherit(&rcss, inh, tr);
        let row_bg = rcss
            .get("background-color")
            .or_else(|| rcss.get("background"))
            .and_then(background_color);
        let row_valign = rcss.get("vertical-align").and_then(parse_valign);
        let mut cells = Vec::new();
        let kids: Vec<Handle> = tr.node.children.borrow().iter().cloned().collect();
        for k in &kids {
            if self.budget == 0 {
                break;
            }
            let Some(td) = element(k) else { continue };
            if td.tag != "td" && td.tag != "th" {
                continue;
            }
            self.budget = self.budget.saturating_sub(1);
            let ccss = self.css(&td, &row_inh);
            if is_hidden(&ccss, row_inh.style.size) {
                continue;
            }
            let mut cinh = inherit(&ccss, &row_inh, &td);
            if td.tag == "th" {
                cinh.style.bold = cinh.style.bold || ccss.get("font-weight").is_none();
                if ccss.get("text-align").is_none() {
                    cinh.align = Align::Center;
                }
            }
            let mut style = box_style(&ccss, &row_inh, &cinh);
            let has_pad = [
                "padding",
                "padding-top",
                "padding-right",
                "padding-bottom",
                "padding-left",
            ]
            .iter()
            .any(|p| ccss.get(p).is_some());
            if !has_pad {
                style.padding = [cellpadding; 4];
            }
            if style.background.is_none() {
                style.background = row_bg;
            }
            if !style.has_border()
                && let Some(g) = grid
            {
                style.border_top = Some(g);
                style.border_right = Some(g);
                style.border_bottom = Some(g);
                style.border_left = Some(g);
            }
            style.min_height = ccss
                .get("height")
                .and_then(|v| parse_px(v, cinh.style.size, None))
                .filter(|h| *h > 0.0)
                .map(|h| h.min(400.0));
            let valign = ccss
                .get("vertical-align")
                .and_then(parse_valign)
                .or(row_valign)
                .unwrap_or_default();
            let colspan = td
                .attr("colspan")
                .and_then(|v| v.trim().parse::<u16>().ok())
                .unwrap_or(1)
                .clamp(1, 24);
            let mut child = Builder::default();
            self.children(td.node, &cinh, &mut child, depth.saturating_add(1), edepth);
            cells.push(Cell {
                style,
                valign,
                colspan,
                blocks: child.finish(),
            });
        }
        (!cells.is_empty()).then_some(Row { cells })
    }
}

fn is_hidden(css: &Css, font: f32) -> bool {
    let lower = |k: &str| css.get(k).map(|v| v.trim().to_ascii_lowercase());
    if lower("display").as_deref() == Some("none")
        || lower("visibility").as_deref() == Some("hidden")
        || lower("mso-hide").as_deref() == Some("all")
    {
        return true;
    }
    if lower("opacity")
        .and_then(|o| o.parse::<f32>().ok())
        .is_some_and(|o| o <= 0.01)
    {
        return true;
    }
    // Preheaders: max-height:0 / height:0 / width:0 with overflow:hidden.
    let zero = |k: &str| {
        css.get(k)
            .and_then(|v| parse_px(v, font, Some(0.0)))
            .is_some_and(|p| p <= 0.5)
    };
    let overflow_hidden = lower("overflow").is_some_and(|o| o.contains("hidden"));
    overflow_hidden && (zero("max-height") || zero("height") || zero("max-width") || zero("width"))
}

fn background_color(v: &str) -> Option<Rgba> {
    // `background: #fff url(x) no-repeat` → the colour part.
    parse_color(v).or_else(|| v.split_whitespace().find_map(parse_color))
}

fn has_button_look(css: &Css) -> bool {
    css.get("background-color")
        .or_else(|| css.get("background"))
        .and_then(background_color)
        .is_some()
        && (css.get("padding").is_some()
            || css.get("padding-top").is_some()
            || css.get("padding-left").is_some())
}

/// `margin: 0 auto` (and friends) centre a box.
fn margin_auto(css: &Css) -> Option<Align> {
    let l = css
        .side("margin", 3)
        .map(|v| v.trim().eq_ignore_ascii_case("auto"));
    let r = css
        .side("margin", 1)
        .map(|v| v.trim().eq_ignore_ascii_case("auto"));
    match (l, r) {
        (Some(true), Some(true)) => Some(Align::Center),
        (Some(true), _) => Some(Align::Right),
        _ => None,
    }
}

fn default_margins(tag: &str) -> (f32, f32) {
    // In ems.
    match tag {
        "p" | "dl" | "figure" => (1.0, 1.0),
        "h1" => (0.67, 0.67),
        "h2" => (0.83, 0.83),
        "h3" => (1.0, 1.0),
        "h4" | "h5" | "h6" => (1.0, 1.0),
        _ => (0.0, 0.0),
    }
}

fn margins_or(css: &Css, font: f32, top_em: f32, bottom_em: f32) -> (f32, f32) {
    let get = |side: usize, def: f32| {
        css.side("margin", side)
            .and_then(|v| parse_px(v, font, Some(0.0)))
            .unwrap_or(def * font)
            .max(0.0)
    };
    // Mail margins are often generous; cap them so a message stays readable.
    (get(0, top_em).min(48.0), get(2, bottom_em).min(48.0))
}

fn margins(css: &Css, font: f32) -> (f32, f32) {
    margins_or(css, font, 0.0, 0.0)
}

/// Text properties inherited by the children of an element.
fn inherit(css: &Css, inh: &Inherit, el: &Element<'_>) -> Inherit {
    let mut out = inh.clone();
    let s = &mut out.style;
    // The style sheet and attributes first, then the tag's own meaning.
    match el.tag.as_str() {
        "b" | "strong" => s.bold = true,
        "i" | "em" | "cite" | "var" | "dfn" | "address" => s.italic = true,
        "u" | "ins" => s.underline = true,
        "s" | "strike" | "del" => s.strike = true,
        "code" | "tt" | "kbd" | "samp" => s.mono = true,
        "small" => s.size *= 0.85,
        "big" => s.size *= 1.2,
        "sub" | "sup" => s.size *= 0.8,
        "mark" => s.background = Some(Rgba::rgb(0xFF, 0xF3, 0x8A)),
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            s.bold = true;
            s.size = match el.tag.as_str() {
                "h1" => 2.0,
                "h2" => 1.5,
                "h3" => 1.17,
                "h4" => 1.0,
                "h5" => 0.83,
                _ => 0.75,
            } * inh.style.size;
        }
        "a" => {
            out.link = el.attr("href").and_then(sanitize_link);
            if out.link.is_some() {
                s.color = None;
                s.underline = true;
            }
        }
        _ => {}
    }
    let s = &mut out.style;
    // Block backgrounds are boxes, not highlighted text.
    if !BLOCKS.contains(&el.tag.as_str())
        && !matches!(el.tag.as_str(), "td" | "th" | "tr" | "table" | "a")
    {
        if let Some(bg) = css.get("background-color").and_then(parse_color) {
            s.background = Some(bg);
        }
    } else {
        s.background = None;
    }
    if let Some(c) = css.get("color").and_then(parse_color) {
        s.color = Some(c);
    }
    if let Some(sz) = css
        .get("font-size")
        .and_then(|v| font_size(v, inh.style.size))
    {
        s.size = sz;
    }
    if let Some(w) = css.get("font-weight") {
        let w = w.trim().to_ascii_lowercase();
        s.bold = match w.as_str() {
            "bold" | "bolder" => true,
            "normal" | "lighter" => false,
            n => n.parse::<u32>().map(|n| n >= 600).unwrap_or(s.bold),
        };
    }
    if let Some(f) = css.get("font-style") {
        s.italic = f.to_ascii_lowercase().contains("italic") || f.contains("oblique");
    }
    if let Some(d) = css
        .get("text-decoration")
        .or_else(|| css.get("text-decoration-line"))
    {
        let d = d.to_ascii_lowercase();
        if d.contains("none") {
            s.underline = false;
            s.strike = false;
        }
        if d.contains("underline") {
            s.underline = true;
        }
        if d.contains("line-through") {
            s.strike = true;
        }
    }
    if let Some(f) = css.get("font-family") {
        let f = f.to_ascii_lowercase();
        if f.contains("monospace") || f.contains("courier") || f.contains("consolas") {
            s.mono = true;
        }
    }
    if let Some(lh) = css.get("line-height") {
        let t = lh.trim();
        let mult = if let Ok(n) = t.parse::<f32>() {
            Some(n)
        } else if t.eq_ignore_ascii_case("normal") {
            None
        } else {
            parse_px(t, s.size, Some(s.size)).and_then(|px| (s.size > 0.0).then(|| px / s.size))
        };
        s.line_height = mult.filter(|m| m.is_finite()).map(|m| m.clamp(0.9, 3.0));
    }
    if let Some(a) = css.get("text-align").and_then(parse_align) {
        out.align = a;
    }
    if let Some(ws) = css.get("white-space") {
        let ws = ws.to_ascii_lowercase();
        if ws.starts_with("pre") {
            out.pre = true;
        }
    }
    out
}

fn sanitize_link(href: &str) -> Option<String> {
    let h = href.trim();
    let lower = h.to_ascii_lowercase();
    (lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:"))
        .then(|| h.to_string())
}

fn box_style(css: &Css, inh: &Inherit, inner: &Inherit) -> BoxStyle {
    let font = inner.style.size;
    let border = |side: &str| {
        let long = format!("border-{side}");
        css.get(&long)
            .or_else(|| css.get("border"))
            .and_then(|v| parse_border(v, font))
            .or_else(|| {
                // border-width/border-color/border-style longhands.
                let w = css
                    .get(&format!("border-{side}-width"))
                    .or_else(|| css.get("border-width"))
                    .and_then(|v| parse_px(v, font, None))?;
                let style = css
                    .get(&format!("border-{side}-style"))
                    .or_else(|| css.get("border-style"))
                    .unwrap_or("none");
                if style.contains("none") || style.contains("hidden") || w <= 0.0 {
                    return None;
                }
                let color = css
                    .get(&format!("border-{side}-color"))
                    .or_else(|| css.get("border-color"))
                    .and_then(parse_color)
                    .unwrap_or(Rgba::rgb(0xCC, 0xCC, 0xCC));
                Some(Border {
                    width: w.min(12.0),
                    color,
                })
            })
    };
    let pad = |side: usize| {
        css.side("padding", side)
            .and_then(|v| parse_px(v, font, Some(0.0)))
            .unwrap_or(0.0)
            .clamp(0.0, 80.0)
    };
    // `width:100%` is kept: on a table it means "fill", unlike no width ("shrink to fit").
    let width = css.get("width").and_then(|v| parse_length(v, font));
    let max_width = css
        .get("max-width")
        .and_then(|v| parse_length(v, font))
        .filter(|l| !matches!(l, Length::Percent(p) if *p >= 99.0));
    let min_height = css
        .get("height")
        .or_else(|| css.get("min-height"))
        .and_then(|v| parse_px(v, font, None))
        .filter(|h| *h > 0.0)
        .map(|h| h.min(400.0));
    BoxStyle {
        background: css
            .get("background-color")
            .or_else(|| css.get("background"))
            .and_then(background_color),
        border_top: border("top"),
        border_right: border("right"),
        border_bottom: border("bottom"),
        border_left: border("left"),
        padding: [pad(0), pad(1), pad(2), pad(3)],
        radius: css
            .get("border-radius")
            .and_then(|v| {
                v.split_whitespace()
                    .next()
                    .and_then(|r| parse_px(r, font, None))
            })
            .unwrap_or(0.0)
            .clamp(0.0, 40.0),
        width,
        max_width,
        min_height,
        align: margin_auto(css).unwrap_or(inh.align),
        link: None,
    }
}

/// Flatten boxes that draw nothing and unwrap single-child wrappers, so the GUI nests less.
fn simplify(blocks: Vec<Block>) -> Vec<Block> {
    let mut out = Vec::with_capacity(blocks.len());
    for b in blocks {
        match b {
            Block::Box { style, blocks } if !style.is_visible() && style.min_height.is_none() => {
                out.extend(simplify(blocks));
            }
            Block::Box { style, blocks } => out.push(Block::Box {
                style,
                blocks: simplify(blocks),
            }),
            Block::Table(mut t) => {
                for r in &mut t.rows {
                    for c in &mut r.cells {
                        c.blocks = simplify(std::mem::take(&mut c.blocks));
                    }
                }
                out.push(Block::Table(t));
            }
            Block::Quote(q) => out.push(Block::Quote(simplify(q))),
            Block::List {
                ordered,
                start,
                marker,
                items,
            } => out.push(Block::List {
                ordered,
                start,
                marker,
                items: items.into_iter().map(simplify).collect(),
            }),
            other => out.push(other),
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Plain text

fn text_of_inlines(inlines: &[Inline], out: &mut String, prefix: &str) {
    for i in inlines {
        match i {
            Inline::Text { text, .. } => out.push_str(&text.replace('\u{a0}', " ")),
            Inline::Break => {
                out.push('\n');
                out.push_str(prefix);
            }
            Inline::Image(img) => {
                if !img.alt.is_empty() && img.link.is_none() && img.alt.len() < 40 {
                    let lower = img.alt.to_ascii_lowercase();
                    if !lower.ends_with(".png")
                        && !lower.ends_with(".jpg")
                        && !lower.ends_with(".gif")
                    {
                        out.push_str(&img.alt);
                    }
                }
            }
        }
    }
}

fn text_of_blocks(blocks: &[Block], out: &mut String, prefix: &str) {
    for b in blocks {
        match b {
            Block::Para(p) => {
                out.push_str(prefix);
                text_of_inlines(&p.inlines, out, prefix);
                out.push('\n');
            }
            Block::Space(_) => {
                out.push_str(prefix.trim_end());
                out.push('\n');
            }
            Block::Rule(_) => {
                out.push_str(prefix);
                out.push_str("----------\n");
            }
            Block::List {
                ordered,
                start,
                marker,
                items,
            } => {
                for (n, item) in items.iter().enumerate() {
                    let mark = match (*ordered, *marker) {
                        (_, false) => String::new(),
                        (true, true) => format!("{}. ", u64::from(*start).saturating_add(n as u64)),
                        (false, true) => "- ".to_string(),
                    };
                    let mut inner = String::new();
                    text_of_blocks(item, &mut inner, "");
                    for (j, line) in inner.trim_end().lines().enumerate() {
                        out.push_str(prefix);
                        if j == 0 {
                            out.push_str(&mark);
                        } else {
                            out.push_str(&" ".repeat(mark.chars().count()));
                        }
                        out.push_str(line);
                        out.push('\n');
                    }
                }
            }
            Block::Quote(q) => {
                let p = format!("{prefix}> ");
                text_of_blocks(q, out, &p);
            }
            Block::Table(t) => {
                for r in &t.rows {
                    for c in &r.cells {
                        text_of_blocks(&c.blocks, out, prefix);
                    }
                }
            }
            Block::Box { blocks, .. } => text_of_blocks(blocks, out, prefix),
        }
    }
}

/// Trim lines, at most one blank line in a row.
fn tidy_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut blank = 0usize;
    for line in s.lines() {
        let line = line.trim_end();
        let is_blank = line.trim_start_matches(['>', ' ']).is_empty();
        if is_blank {
            blank = blank.saturating_add(1);
            if blank > 1 || out.is_empty() {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(line);
        out.push('\n');
    }
    out.trim_end().to_string()
}

/// Every image `src` in the document (to know which inline parts are used).
pub fn image_sources(blocks: &[Block], out: &mut HashSet<String>) {
    for b in blocks {
        match b {
            Block::Para(p) => {
                for i in &p.inlines {
                    if let Inline::Image(img) = i {
                        out.insert(img.src.clone());
                    }
                }
            }
            Block::List { items, .. } => {
                for item in items {
                    image_sources(item, out);
                }
            }
            Block::Quote(q) | Block::Box { blocks: q, .. } => image_sources(q, out),
            Block::Table(t) => {
                for r in &t.rows {
                    for c in &r.cells {
                        image_sources(&c.blocks, out);
                    }
                }
            }
            Block::Space(_) | Block::Rule(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paras(doc: &Doc) -> Vec<String> {
        let mut out = Vec::new();
        fn walk(bs: &[Block], out: &mut Vec<String>) {
            for b in bs {
                match b {
                    Block::Para(p) => {
                        let mut s = String::new();
                        text_of_inlines(&p.inlines, &mut s, "");
                        out.push(s);
                    }
                    Block::Box { blocks, .. } | Block::Quote(blocks) => walk(blocks, out),
                    Block::Table(t) => {
                        for r in &t.rows {
                            for c in &r.cells {
                                walk(&c.blocks, out);
                            }
                        }
                    }
                    Block::List { items, .. } => {
                        for i in items {
                            walk(i, out);
                        }
                    }
                    _ => {}
                }
            }
        }
        walk(&doc.blocks, &mut out);
        out
    }

    #[test]
    fn paragraphs_whitespace_and_breaks() {
        let d = parse("<p>Ciao   <b>mondo</b>\n bello</p><div>riga<br>due<br></div>");
        assert_eq!(paras(&d), vec!["Ciao mondo bello", "riga\ndue"]);
        let Some(Block::Para(p)) = d.blocks.first() else {
            panic!("{:?}", d.blocks)
        };
        assert!(p.inlines.iter().any(
            |i| matches!(i, Inline::Text { text, style, .. } if text == "mondo" && style.bold)
        ));
    }

    #[test]
    fn hidden_preheader_and_tracking_pixel_are_dropped() {
        let d = parse(
            "<div style=\"display:none;max-height:0\">anteprima nascosta</div>\
             <span style=\"max-height:0px;overflow:hidden\">altro</span>\
             <p>Visibile</p><img src=\"https://t.example/p.gif\" width=\"1\" height=\"1\">",
        );
        assert_eq!(paras(&d), vec!["Visibile"]);
        assert_eq!(d.remote_images, 0);
    }

    #[test]
    fn links_images_and_cids() {
        let d = parse(
            "<a href=\"https://x.it/a\">vai</a> <a href=\"javascript:alert(1)\">no</a>\
             <img src=\"cid:<LOGO@x>\" alt=\"logo\"><img src=\"https://x.it/i.png\" width=\"40\">",
        );
        assert_eq!(d.cids, vec!["logo@x".to_string()]);
        assert_eq!(d.remote_images, 1);
        let Some(Block::Para(p)) = d.blocks.first() else {
            panic!()
        };
        let links: Vec<_> = p
            .inlines
            .iter()
            .filter_map(|i| match i {
                Inline::Text { link: Some(l), .. } => Some(l.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(links, vec!["https://x.it/a"]);
    }

    #[test]
    fn style_sheet_simple_selectors() {
        let d = parse(
            "<style>p.MsoNormal{margin:0cm;color:#ff0000} .big{font-size:24px} td p{color:blue}</style>\
             <p class=\"MsoNormal\">uno</p><p class=MsoNormal>due</p><span class=big>grande</span>",
        );
        // margin:0 → no Space between the two paragraphs
        assert!(matches!(
            d.blocks.as_slice(),
            [Block::Para(_), Block::Para(_), Block::Para(_)]
        ));
        let Some(Block::Para(p)) = d.blocks.first() else {
            panic!()
        };
        assert!(
            matches!(p.inlines.first(), Some(Inline::Text { style, .. }) if style.color == Some(Rgba::rgb(255,0,0)))
        );
        let Some(Block::Para(p)) = d.blocks.get(2) else {
            panic!()
        };
        assert!(
            matches!(p.inlines.first(), Some(Inline::Text { style, .. }) if (style.size - 24.0).abs() < 0.1)
        );
    }

    #[test]
    fn layout_tables_become_boxes_and_tables() {
        let d = parse(
            "<table width=\"600\" align=\"center\" bgcolor=\"#ffffff\"><tr><td style=\"padding:20px\">\
             <table><tr><td width=\"30\">A</td><td>B</td></tr></table></td></tr></table>",
        );
        let Some(Block::Box { style, blocks }) = d.blocks.first() else {
            panic!("{:?}", d.blocks)
        };
        assert_eq!(style.width, Some(Length::Px(600.0)));
        assert_eq!(style.align, Align::Center);
        assert_eq!(style.padding, [20.0; 4]);
        let Some(Block::Table(t)) = blocks.first() else {
            panic!("{blocks:?}")
        };
        assert_eq!(t.columns(), 2);
    }

    #[test]
    fn buttons_and_colors() {
        let d = parse(
            "<a href=\"https://b.it\" style=\"background-color:#0071c2;color:#fff;padding:10px 20px;border-radius:4px\">Gestisci</a>",
        );
        let Some(Block::Box { style, blocks }) = d.blocks.first() else {
            panic!("{:?}", d.blocks)
        };
        assert_eq!(style.background, Some(Rgba::rgb(0, 0x71, 0xc2)));
        assert_eq!(style.padding, [10.0, 20.0, 10.0, 20.0]);
        assert!(matches!(blocks.first(), Some(Block::Para(_))));
        assert_eq!(parse_color("rgb(10, 20, 30)"), Some(Rgba::rgb(10, 20, 30)));
        assert_eq!(parse_color("#abc"), Some(Rgba::rgb(0xaa, 0xbb, 0xcc)));
        assert_eq!(parse_color("transparent"), None);
    }

    #[test]
    fn nbsp_paragraph_is_a_blank_line() {
        let d = parse(
            "<p style=\"margin:0\">a</p><p style=\"margin:0\">&nbsp;</p><p style=\"margin:0\">b</p>",
        );
        assert!(matches!(
            d.blocks.as_slice(),
            [Block::Para(_), Block::Space(_), Block::Para(_)]
        ));
    }

    #[test]
    fn text_for_quoting() {
        let d =
            parse("<p>Ciao,</p><ul><li>uno</li><li>due</li></ul><blockquote>citato</blockquote>");
        assert_eq!(d.to_text(), "Ciao,\n\n- uno\n- due\n\n> citato");
    }

    #[test]
    fn plain_text_links_and_quotes() {
        let d = from_plain("Vedi https://example.it/x. Grazie\n\n> sotto\n> ancora");
        let Some(Block::Para(p)) = d.blocks.first() else {
            panic!()
        };
        assert!(p.inlines.iter().any(
            |i| matches!(i, Inline::Text { link: Some(l), .. } if l == "https://example.it/x")
        ));
        assert!(matches!(d.blocks.last(), Some(Block::Quote(_))));
    }

    #[test]
    fn data_uris() {
        assert_eq!(
            data_uri_bytes("data:image/png;base64,aGk="),
            Some(b"hi".to_vec())
        );
        assert_eq!(
            data_uri_bytes("data:text/plain,a%20b"),
            Some(b"a b".to_vec())
        );
        assert_eq!(data_uri_bytes("nope"), None);
    }

    #[test]
    fn garbage_and_deep_nesting_never_panic() {
        for s in [
            "",
            "<",
            "<table><tr><td><table>",
            "<p style=\"color:#zzzzzz;font-size:-3e99px;margin:auto auto auto\">x</p>",
            "<style>@media{{{</style><p>x",
            "<font size=\"+99\">x</font><img src=\"cid:\"><a href=>y</a>",
        ] {
            let _ = parse(s);
        }
        let deep = "<div style=\"padding:1px\">".repeat(5000) + "fondo";
        let d = parse(&deep);
        assert!(d.to_text().contains("fondo"));
        let tables = "<table><tr><td style=\"background:#eee\">".repeat(400) + "giù";
        assert!(parse(&tables).to_text().contains("giù"));
        let spans = "<b>".repeat(20_000) + "x";
        assert!(parse(&spans).to_text().contains('x'));
    }
}
