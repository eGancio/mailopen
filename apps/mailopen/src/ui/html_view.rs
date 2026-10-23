//! Draw a [`Doc`] with egui: paragraphs as selectable text with clickable links, images
//! (inline parts, `data:`, remote ones only when allowed), lists, quotes, tables and coloured
//! boxes. Immediate mode: laid out every frame, egui caches the text layout.

use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::Arc;

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontFamily, FontId, Rect, Sense, Stroke, Ui, UiBuilder, Vec2, pos2, vec2};
use mailopen_core::MessageBody;
use mailopen_core::html::{
    Align, Block, BoxStyle, Doc, Image, Inline, Paragraph, Rgba, Table, TextStyle, VAlign,
};

use crate::ui::images::{ImageCache, ImgState, Source};
use crate::ui::theme;

/// Inline images are decoded at most this large (pixels per side).
const INLINE_MAX_SIDE: u32 = 1600;

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub text: Color32,
    pub link: Color32,
    pub muted: Color32,
    pub rule: Color32,
    pub quote: Color32,
    pub placeholder: Color32,
}

impl Palette {
    /// Mail HTML is designed for white paper: draw it on white in both themes.
    pub fn paper() -> Palette {
        Palette {
            text: Color32::from_rgb(0x1F, 0x29, 0x37),
            link: Color32::from_rgb(0x1A, 0x5F, 0xD0),
            muted: Color32::from_rgb(0x6B, 0x72, 0x80),
            rule: Color32::from_rgb(0xD9, 0xDC, 0xE1),
            quote: Color32::from_rgb(0xC8, 0xCD, 0xD5),
            placeholder: Color32::from_rgb(0xEE, 0xF0, 0xF3),
        }
    }

    /// Plain-text mail follows the window theme.
    pub fn themed(ui: &Ui) -> Palette {
        let t = theme::tokens(ui);
        Palette {
            text: t.text,
            link: t.accent,
            muted: t.muted,
            rule: t.border,
            quote: t.border,
            placeholder: t.hover,
        }
    }
}

pub struct View<'a> {
    pub images: &'a mut ImageCache,
    pub body: &'a MessageBody,
    /// Cache key prefix of this message's inline images.
    pub key: String,
    pub allow_remote: bool,
    pub palette: Palette,
    /// The link the user clicked this frame.
    pub clicked: Option<String>,
    /// Layout settled this frame (vertical centring uses last frame's sizes).
    repaint: bool,
    /// Intrinsic widths of tables already measured this frame, by address.
    memo: std::collections::HashMap<usize, (f32, f32)>,
}

impl<'a> View<'a> {
    pub fn new(
        images: &'a mut ImageCache,
        body: &'a MessageBody,
        key: String,
        allow_remote: bool,
        palette: Palette,
    ) -> Self {
        View {
            images,
            body,
            key,
            allow_remote,
            palette,
            clicked: None,
            repaint: false,
            memo: std::collections::HashMap::new(),
        }
    }
}

/// Draw the document in the available width.
pub fn show(ui: &mut Ui, doc: &Doc, v: &mut View<'_>) {
    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
    blocks(ui, &doc.blocks, v, 0);
    if v.repaint {
        ui.ctx().request_repaint();
    }
}

pub fn color(c: Rgba) -> Color32 {
    Color32::from_rgb(c.r, c.g, c.b)
}

fn blocks(ui: &mut Ui, bs: &[Block], v: &mut View<'_>, depth: usize) {
    for b in bs {
        block(ui, b, v, depth);
    }
}

fn block(ui: &mut Ui, b: &Block, v: &mut View<'_>, depth: usize) {
    match b {
        Block::Para(p) => para(ui, p, v),
        Block::Space(px) => {
            ui.add_space(*px);
        }
        Block::Rule(c) => {
            let w = ui.available_width();
            let (rect, _) = ui.allocate_exact_size(vec2(w, 11.0), Sense::hover());
            let col = c.map(color).unwrap_or(v.palette.rule);
            ui.painter()
                .hline(rect.x_range(), rect.center().y, Stroke::new(1.0, col));
        }
        Block::List {
            ordered,
            start,
            marker,
            items,
        } => list(ui, *ordered, *start, *marker, items, v, depth),
        Block::Quote(q) => quote(ui, q, v, depth),
        Block::Table(t) => table(ui, t, v, depth),
        Block::Box {
            style,
            blocks: inner,
        } => {
            boxed(ui, style, v, |ui, v| {
                blocks(ui, inner, v, depth.saturating_add(1))
            });
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Text

fn text_format(style: &TextStyle, is_link: bool, p: &Palette) -> TextFormat {
    let size = style.size.clamp(8.0, 48.0);
    let family = if style.mono {
        FontFamily::Monospace
    } else if style.bold {
        theme::bold_family()
    } else {
        FontFamily::Proportional
    };
    let col = style
        .color
        .map(color)
        .unwrap_or(if is_link { p.link } else { p.text });
    TextFormat {
        font_id: FontId::new(size, family),
        color: col,
        italics: style.italic,
        underline: if style.underline {
            Stroke::new(1.0, col)
        } else {
            Stroke::NONE
        },
        strikethrough: if style.strike {
            Stroke::new(1.0, col)
        } else {
            Stroke::NONE
        },
        background: style.background.map(color).unwrap_or(Color32::TRANSPARENT),
        line_height: Some(size * style.line_height.unwrap_or(1.4).clamp(1.0, 2.6)),
        valign: egui::Align::BOTTOM,
        ..Default::default()
    }
}

/// One layout job for a run of text inlines, and the char ranges that are links.
fn text_job(inlines: &[Inline], p: &Palette) -> (LayoutJob, Vec<(Range<usize>, String)>) {
    let mut job = LayoutJob::default();
    let mut links = Vec::new();
    let mut chars = 0usize;
    let mut last = text_format(&TextStyle::default(), false, p);
    for i in inlines {
        match i {
            Inline::Text { text, style, link } => {
                let fmt = text_format(style, link.is_some(), p);
                // Some faces lack U+00A0; it lays out like a space anyway.
                let t = text.replace('\u{a0}', " ");
                let n = t.chars().count();
                if let Some(l) = link {
                    links.push((chars..chars.saturating_add(n), l.clone()));
                }
                chars = chars.saturating_add(n);
                last = fmt.clone();
                job.append(&t, 0.0, fmt);
            }
            Inline::Break => {
                job.append("\n", 0.0, last.clone());
                chars = chars.saturating_add(1);
            }
            Inline::Image(_) => {}
        }
    }
    (job, links)
}

/// A selectable label whose link ranges react to hover and click.
fn rich_label(ui: &mut Ui, job: LayoutJob, links: &[(Range<usize>, String)], v: &mut View<'_>) {
    if job.text.is_empty() {
        return;
    }
    let sense = if links.is_empty() {
        Sense::hover()
    } else {
        Sense::click()
    };
    let (pos, galley, mut response) = egui::Label::new(job)
        .selectable(true)
        .sense(sense)
        .layout_in_ui(ui);
    if !links.is_empty()
        && let Some(p) = response.hover_pos()
    {
        let rel = p - pos;
        let on_text = galley
            .rows
            .iter()
            .any(|r| r.rect().expand(1.0).contains(rel.to_pos2()));
        if on_text {
            let cursor = galley.cursor_from_pos(rel);
            let caret = galley.pos_from_cursor(cursor);
            // The character under the pointer is the one before the caret when the pointer is
            // left of it.
            let idx = if rel.x < caret.center().x {
                cursor.index.0.saturating_sub(1)
            } else {
                cursor.index.0
            };
            if let Some((_, url)) = links.iter().find(|(r, _)| r.contains(&idx)) {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                if response.clicked() {
                    v.clicked = Some(url.clone());
                }
                response = response.on_hover_text_at_pointer(url.as_str());
            }
        }
    }
    if ui.is_rect_visible(response.rect) {
        egui::text_selection::LabelSelectionState::label_text_selection(
            ui,
            &response,
            pos,
            galley,
            v.palette.text,
            Stroke::NONE,
        );
    }
}

fn egui_align(a: Align) -> egui::Align {
    match a {
        Align::Left => egui::Align::Min,
        Align::Center => egui::Align::Center,
        Align::Right => egui::Align::Max,
    }
}

fn para(ui: &mut Ui, p: &Paragraph, v: &mut View<'_>) {
    let has_image = p.inlines.iter().any(|i| matches!(i, Inline::Image(_)));
    if !has_image {
        let (job, links) = text_job(&p.inlines, &v.palette);
        ui.with_layout(egui::Layout::top_down(egui_align(p.align)), |ui| {
            rich_label(ui, job, &links, v);
        });
        return;
    }
    let only_images = p.inlines.iter().all(|i| match i {
        Inline::Image(_) | Inline::Break => true,
        Inline::Text { text, .. } => text.trim().is_empty(),
    });
    if only_images {
        image_rows(ui, p, v);
        return;
    }
    // Text and images in one flow.
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        let mut run: Vec<Inline> = Vec::new();
        for i in &p.inlines {
            match i {
                Inline::Image(img) => {
                    if !run.is_empty() {
                        let (job, links) = text_job(&run, &v.palette);
                        rich_label(ui, job, &links, v);
                        run.clear();
                    }
                    image(ui, img, v);
                }
                Inline::Break => {
                    if !run.is_empty() {
                        let (job, links) = text_job(&run, &v.palette);
                        rich_label(ui, job, &links, v);
                        run.clear();
                    }
                    ui.end_row();
                }
                other => run.push(other.clone()),
            }
        }
        if !run.is_empty() {
            let (job, links) = text_job(&run, &v.palette);
            rich_label(ui, job, &links, v);
        }
    });
}

/// A paragraph of images only (logos, banners, icon rows), aligned as a whole.
fn image_rows(ui: &mut Ui, p: &Paragraph, v: &mut View<'_>) {
    let avail = ui.available_width();
    let mut rows: Vec<Vec<&Image>> = vec![Vec::new()];
    for i in &p.inlines {
        match i {
            Inline::Image(img) => {
                if let Some(r) = rows.last_mut() {
                    r.push(img);
                }
            }
            Inline::Break => rows.push(Vec::new()),
            Inline::Text { .. } => {}
        }
    }
    for row in rows.iter().filter(|r| !r.is_empty()) {
        let sizes: Vec<Vec2> = row.iter().map(|img| planned_size(img, v, avail)).collect();
        let total: f32 = sizes.iter().map(|s| s.x).sum();
        if total > avail + 0.5 {
            ui.horizontal_wrapped(|ui| {
                for img in row {
                    image(ui, img, v);
                }
            });
            continue;
        }
        let offset = match p.align {
            Align::Left => 0.0,
            Align::Center => ((avail - total) / 2.0).max(0.0),
            Align::Right => (avail - total).max(0.0),
        };
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            ui.add_space(offset);
            for img in row {
                image(ui, img, v);
            }
        });
    }
}

// ---------------------------------------------------------------------------------------------
// Images

enum Resolved {
    State(ImgState),
    Blocked,
}

fn hash_str(s: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

fn resolve(img: &Image, v: &mut View<'_>) -> Resolved {
    let src = img.src.as_str();
    let lower = src.get(..5).unwrap_or("").to_ascii_lowercase();
    if lower.starts_with("cid:") {
        let key = format!("{}#{}", v.key, src.to_ascii_lowercase());
        let part = v.body.inline_part(src);
        Resolved::State(v.images.get(&key, INLINE_MAX_SIDE, || {
            part.map(|a| Source::Bytes(Arc::clone(&a.data)))
        }))
    } else if lower.starts_with("data:") {
        let key = format!("data#{:x}", hash_str(src));
        Resolved::State(v.images.get(&key, INLINE_MAX_SIDE, || {
            mailopen_core::html::data_uri_bytes(src).map(|b| Source::Bytes(Arc::from(b)))
        }))
    } else if v.allow_remote {
        Resolved::State(
            v.images
                .get(src, INLINE_MAX_SIDE, || Some(Source::Url(src.to_string()))),
        )
    } else {
        Resolved::Blocked
    }
}

/// The size an image will take: the message's width/height, else the picture's own, scaled
/// down to fit.
fn image_size(img: &Image, natural: Option<Vec2>, avail: f32) -> Vec2 {
    let w = img.width.map(|l| l.of(avail)).filter(|w| *w > 0.0);
    let h = img.height.filter(|h| *h > 0.0);
    let ratio = natural
        .filter(|n| n.x > 0.0 && n.y > 0.0)
        .map(|n| n.y / n.x);
    let size = match (w, h, natural, ratio) {
        (Some(w), Some(h), _, _) => vec2(w, h),
        (Some(w), None, _, Some(r)) => vec2(w, w * r),
        (None, Some(h), _, Some(r)) => vec2(h / r, h),
        (None, None, Some(n), _) => n,
        (Some(w), None, _, None) => vec2(w, (w * 0.4).min(120.0)),
        (None, Some(h), _, None) => vec2(h, h),
        _ => vec2(0.0, 0.0),
    };
    if size.x > avail && size.x > 0.0 {
        vec2(avail, size.y * avail / size.x)
    } else {
        size
    }
}

fn planned_size(img: &Image, v: &mut View<'_>, avail: f32) -> Vec2 {
    match resolve(img, v) {
        Resolved::State(ImgState::Ready(_, n)) => image_size(img, Some(n), avail),
        Resolved::State(ImgState::Failed) => {
            if img.alt.is_empty() {
                Vec2::ZERO
            } else {
                vec2((img.alt.chars().count() as f32 * 7.0).min(avail), 18.0)
            }
        }
        _ => image_size(img, None, avail),
    }
}

fn image(ui: &mut Ui, img: &Image, v: &mut View<'_>) {
    let avail = ui.available_width().max(1.0);
    match resolve(img, v) {
        Resolved::State(ImgState::Ready(id, natural)) => {
            let size = image_size(img, Some(natural), avail);
            if size.x < 1.0 || size.y < 1.0 {
                return;
            }
            let sense = if img.link.is_some() {
                Sense::click()
            } else {
                Sense::hover()
            };
            let resp = ui.add(
                egui::Image::new(egui::load::SizedTexture::new(id, size))
                    .fit_to_exact_size(size)
                    .sense(sense),
            );
            if let Some(link) = &img.link {
                let resp = resp
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text_at_pointer(link.as_str());
                if resp.clicked() {
                    v.clicked = Some(link.clone());
                }
            } else if !img.alt.is_empty() {
                resp.on_hover_text(img.alt.as_str());
            }
        }
        Resolved::State(ImgState::Loading) => {
            let size = image_size(img, None, avail);
            if size.x >= 1.0 && size.y >= 1.0 {
                let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                ui.painter()
                    .rect_filled(rect, egui::CornerRadius::same(2), v.palette.placeholder);
            }
        }
        Resolved::State(ImgState::Failed) => alt_text(ui, img, v, avail),
        Resolved::Blocked => {
            let size = image_size(img, None, avail);
            if size.x >= 24.0 && size.y >= 16.0 {
                let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
                let p = ui.painter();
                p.rect_filled(rect, egui::CornerRadius::same(3), v.palette.placeholder);
                if !img.alt.is_empty() && rect.height() >= 14.0 {
                    let galley = p.layout(
                        img.alt.clone(),
                        FontId::proportional(11.5),
                        v.palette.muted,
                        (rect.width() - 8.0).max(8.0),
                    );
                    let at = rect.center() - galley.size() / 2.0;
                    p.with_clip_rect(rect).galley(at, galley, v.palette.muted);
                }
                resp.on_hover_text("Immagine remota bloccata");
            } else {
                alt_text(ui, img, v, avail);
            }
        }
    }
}

fn alt_text(ui: &mut Ui, img: &Image, v: &View<'_>, avail: f32) {
    let alt = img.alt.trim();
    // File names as alt text ("[lock.png]") say nothing.
    let lower = alt.to_ascii_lowercase();
    if alt.is_empty()
        || lower.ends_with(".png")
        || lower.ends_with(".jpg")
        || lower.ends_with(".gif")
        || lower.ends_with(".jpeg")
    {
        return;
    }
    let galley = ui.painter().layout(
        alt.to_string(),
        FontId::proportional(12.0),
        v.palette.muted,
        avail,
    );
    let (rect, _) = ui.allocate_exact_size(galley.size(), Sense::hover());
    ui.painter().galley(rect.min, galley, v.palette.muted);
}

// ---------------------------------------------------------------------------------------------
// Boxes, lists, quotes, tables

fn box_width(style: &BoxStyle, avail: f32) -> f32 {
    let mut w = style.width.map(|l| l.of(avail)).unwrap_or(avail);
    if let Some(m) = style.max_width {
        w = w.min(m.of(avail));
    }
    w.clamp(1.0, avail.max(1.0))
}

fn align_offset(a: Align, avail: f32, w: f32) -> f32 {
    match a {
        Align::Left => 0.0,
        Align::Center => ((avail - w) / 2.0).max(0.0),
        Align::Right => (avail - w).max(0.0),
    }
}

/// Border widths: top, right, bottom, left.
fn borders(s: &BoxStyle) -> [f32; 4] {
    [
        s.border_top.map(|b| b.width).unwrap_or(0.0),
        s.border_right.map(|b| b.width).unwrap_or(0.0),
        s.border_bottom.map(|b| b.width).unwrap_or(0.0),
        s.border_left.map(|b| b.width).unwrap_or(0.0),
    ]
}

fn paint_box(ui: &Ui, idx: egui::layers::ShapeIdx, rect: Rect, s: &BoxStyle) {
    let radius = egui::CornerRadius::same(s.radius.clamp(0.0, 40.0) as u8);
    if let Some(bg) = s.background {
        ui.painter()
            .set(idx, egui::Shape::rect_filled(rect, radius, color(bg)));
    }
    let p = ui.painter();
    let sides = [s.border_top, s.border_right, s.border_bottom, s.border_left];
    let all_same = sides.iter().all(|b| *b == s.border_top) && s.border_top.is_some();
    if all_same && let Some(b) = s.border_top {
        p.rect_stroke(
            rect,
            radius,
            Stroke::new(b.width, color(b.color)),
            egui::StrokeKind::Inside,
        );
        return;
    }
    let half = |w: f32| w / 2.0;
    if let Some(b) = s.border_top {
        let y = rect.top() + half(b.width);
        p.hline(rect.x_range(), y, Stroke::new(b.width, color(b.color)));
    }
    if let Some(b) = s.border_bottom {
        let y = rect.bottom() - half(b.width);
        p.hline(rect.x_range(), y, Stroke::new(b.width, color(b.color)));
    }
    if let Some(b) = s.border_left {
        let x = rect.left() + half(b.width);
        p.vline(x, rect.y_range(), Stroke::new(b.width, color(b.color)));
    }
    if let Some(b) = s.border_right {
        let x = rect.right() - half(b.width);
        p.vline(x, rect.y_range(), Stroke::new(b.width, color(b.color)));
    }
}

/// Draw `content` inside a box placed at `x`, `top`, `w` wide; returns its height.
#[allow(clippy::too_many_arguments)]
fn box_at(
    ui: &mut Ui,
    s: &BoxStyle,
    x: f32,
    top: f32,
    w: f32,
    min_h: f32,
    salt: egui::Id,
    v: &mut View<'_>,
    content: impl FnOnce(&mut Ui, &mut View<'_>),
) -> (Rect, egui::layers::ShapeIdx) {
    let idx = ui.painter().add(egui::Shape::Noop);
    let [bt, br, bb, bl] = borders(s);
    let pad = s.padding;
    let inner_w = (w - pad[1] - pad[3] - bl - br).max(1.0);
    let inner = Rect::from_min_size(
        pos2(x + bl + pad[3], top + bt + pad[0]),
        vec2(inner_w, f32::INFINITY),
    );
    let mut child = ui.new_child(
        UiBuilder::new()
            .id_salt(salt)
            .max_rect(inner)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    content(&mut child, v);
    let used = child.min_rect();
    let content_h = (used.bottom() - inner.top()).max(0.0);
    let h = (content_h + pad[0] + pad[2] + bt + bb)
        .max(s.min_height.unwrap_or(0.0))
        .max(min_h);
    (Rect::from_min_size(pos2(x, top), vec2(w, h)), idx)
}

fn boxed(
    ui: &mut Ui,
    s: &BoxStyle,
    v: &mut View<'_>,
    content: impl FnOnce(&mut Ui, &mut View<'_>),
) {
    let avail = ui.available_width();
    let w = box_width(s, avail);
    let left = ui.cursor().left();
    let x = left + align_offset(s.align, avail, w);
    let top = ui.cursor().top();
    let salt = ui.next_auto_id();
    ui.skip_ahead_auto_ids(1);
    let (rect, idx) = box_at(ui, s, x, top, w, 0.0, salt, v, content);
    paint_box(ui, idx, rect, s);
    if let Some(link) = &s.link {
        let resp = ui
            .interact(rect, salt.with("link"), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text_at_pointer(link.as_str());
        if resp.clicked() {
            v.clicked = Some(link.clone());
        }
    }
    ui.advance_cursor_after_rect(Rect::from_min_size(
        pos2(left, top),
        vec2(avail, rect.height()),
    ));
}

#[allow(clippy::too_many_arguments)]
fn list(
    ui: &mut Ui,
    ordered: bool,
    start: u32,
    marker: bool,
    items: &[Vec<Block>],
    v: &mut View<'_>,
    depth: usize,
) {
    let indent = if marker { 24.0 } else { 0.0 };
    for (n, item) in items.iter().enumerate() {
        let avail = ui.available_width();
        let left = ui.cursor().left();
        let top = ui.cursor().top();
        let salt = ui.next_auto_id();
        ui.skip_ahead_auto_ids(1);
        let s = BoxStyle::default();
        let (rect, _) = box_at(
            ui,
            &s,
            left + indent,
            top,
            (avail - indent).max(1.0),
            0.0,
            salt,
            v,
            |ui, v| blocks(ui, item, v, depth.saturating_add(1)),
        );
        if marker {
            let size = first_size(item).clamp(10.0, 28.0);
            let label = if ordered {
                format!("{}.", u64::from(start).saturating_add(n as u64))
            } else {
                "•".to_string()
            };
            let line = size * 1.4;
            ui.painter().text(
                pos2(left + indent - 7.0, top + line / 2.0),
                egui::Align2::RIGHT_CENTER,
                label,
                FontId::proportional(size),
                v.palette.text,
            );
        }
        ui.advance_cursor_after_rect(Rect::from_min_size(
            pos2(left, top),
            vec2(avail, rect.height() + 3.0),
        ));
    }
}

fn first_size(bs: &[Block]) -> f32 {
    for b in bs {
        if let Block::Para(p) = b {
            for i in &p.inlines {
                if let Inline::Text { style, .. } = i {
                    return style.size;
                }
            }
        }
    }
    mailopen_core::html::BASE_SIZE
}

fn quote(ui: &mut Ui, q: &[Block], v: &mut View<'_>, depth: usize) {
    let avail = ui.available_width();
    let left = ui.cursor().left();
    let top = ui.cursor().top();
    let salt = ui.next_auto_id();
    ui.skip_ahead_auto_ids(1);
    let s = BoxStyle::default();
    let indent = 14.0;
    let (rect, _) = box_at(
        ui,
        &s,
        left + indent,
        top,
        (avail - indent).max(1.0),
        0.0,
        salt,
        v,
        |ui, v| blocks(ui, q, v, depth.saturating_add(1)),
    );
    ui.painter().vline(
        left + 3.0,
        rect.y_range(),
        Stroke::new(3.0, v.palette.quote),
    );
    ui.advance_cursor_after_rect(Rect::from_min_size(
        pos2(left, top),
        vec2(avail, rect.height()),
    ));
}

// --- Intrinsic widths (what CSS calls min-content and max-content), CSS px.

fn para_intrinsic(ui: &Ui, p: &Paragraph, v: &mut View<'_>) -> (f32, f32) {
    let mut min: f32 = 0.0;
    let mut images: f32 = 0.0;
    let mut texts: Vec<Inline> = Vec::new();
    for i in &p.inlines {
        match i {
            Inline::Text { text, style, .. } => {
                let k = if style.bold { 0.62 } else { 0.56 };
                let longest = text
                    .split_whitespace()
                    .map(|w| w.chars().count())
                    .max()
                    .unwrap_or(0) as f32;
                min = min.max(longest * style.size * k);
                texts.push(i.clone());
            }
            Inline::Image(img) => {
                let w = match img.width {
                    Some(mailopen_core::html::Length::Px(px)) => px,
                    Some(mailopen_core::html::Length::Percent(_)) => 0.0,
                    None => match resolve(img, v) {
                        Resolved::State(ImgState::Ready(_, n)) => match img.height {
                            Some(h) if n.y > 0.0 => h * n.x / n.y,
                            _ => n.x,
                        },
                        _ => img.height.unwrap_or(0.0),
                    },
                };
                min = min.max(w);
                images += w;
            }
            Inline::Break => texts.push(Inline::Break),
        }
    }
    let text_w = if texts.iter().any(|i| matches!(i, Inline::Text { .. })) {
        let (mut job, _) = text_job(&texts, &v.palette);
        job.wrap.max_width = f32::INFINITY;
        ui.fonts_mut(|f| f.layout_job(job)).size().x
    } else {
        0.0
    };
    let max = text_w + images;
    (min.min(max.max(min)), max.max(min))
}

fn box_intrinsic(s: &BoxStyle, inner: (f32, f32)) -> (f32, f32) {
    let [bt, br, bb, bl] = borders(s);
    let _ = (bt, bb);
    let extra = s.padding[1] + s.padding[3] + br + bl;
    let (mut min, mut max) = (inner.0 + extra, inner.1 + extra);
    if let Some(mailopen_core::html::Length::Px(w)) = s.width {
        min = min.max(w).min(w.max(min));
        max = w.max(min);
        min = w.min(max);
    }
    if let Some(mailopen_core::html::Length::Px(m)) = s.max_width {
        max = max.min(m);
        min = min.min(m);
    }
    (min, max)
}

fn intrinsic(ui: &Ui, bs: &[Block], v: &mut View<'_>, depth: usize) -> (f32, f32) {
    if depth > mailopen_core::html::MAX_DEPTH + 8 {
        return (0.0, 0.0);
    }
    let mut out: (f32, f32) = (0.0, 0.0);
    for b in bs {
        let (a, z) = match b {
            Block::Para(p) => para_intrinsic(ui, p, v),
            Block::Space(_) | Block::Rule(_) => (0.0, 0.0),
            Block::List { items, marker, .. } => {
                let indent = if *marker { 24.0 } else { 0.0 };
                let mut m: (f32, f32) = (0.0, 0.0);
                for item in items {
                    let (a, z) = intrinsic(ui, item, v, depth + 1);
                    m = (m.0.max(a + indent), m.1.max(z + indent));
                }
                m
            }
            Block::Quote(q) => {
                let (a, z) = intrinsic(ui, q, v, depth + 1);
                (a + 14.0, z + 14.0)
            }
            Block::Table(t) => table_intrinsic(ui, t, v, depth + 1),
            Block::Box {
                style,
                blocks: inner,
            } => {
                let m = intrinsic(ui, inner, v, depth + 1);
                box_intrinsic(style, m)
            }
        };
        out = (out.0.max(a), out.1.max(z));
    }
    out
}

/// Per column: (min, max, declared width). Cells spanning several columns are left out.
fn column_metrics(
    ui: &Ui,
    t: &Table,
    cols: usize,
    width: f32,
    v: &mut View<'_>,
    depth: usize,
) -> Vec<(f32, f32, Option<f32>)> {
    let mut m: Vec<(f32, f32, Option<f32>)> = vec![(0.0, 0.0, None); cols];
    for row in &t.rows {
        let mut c = 0usize;
        for cell in &row.cells {
            let span = usize::from(cell.colspan.max(1));
            if span == 1 {
                let inner = intrinsic(ui, &cell.blocks, v, depth + 1);
                let (mut a, mut z) = box_intrinsic(&cell.style, inner);
                let declared = cell.style.width.map(|l| l.of(width));
                if let Some(d) = declared {
                    a = a.min(d.max(a));
                    z = d.max(a);
                }
                if let Some(slot) = m.get_mut(c) {
                    slot.0 = slot.0.max(a);
                    slot.1 = slot.1.max(z);
                    if let Some(d) = declared {
                        slot.2 = Some(slot.2.map_or(d, |f| f.max(d)));
                    }
                }
            }
            c = c.saturating_add(span);
        }
    }
    m
}

fn table_intrinsic(ui: &Ui, t: &Table, v: &mut View<'_>, depth: usize) -> (f32, f32) {
    let key = t as *const Table as usize;
    if let Some(m) = v.memo.get(&key) {
        return *m;
    }
    let cols = t.columns().max(1);
    let sp = t.spacing.min(8.0) * (cols as f32 + 1.0);
    let out = match t.style.width {
        Some(mailopen_core::html::Length::Px(w)) => (w, w),
        _ => {
            let m = column_metrics(ui, t, cols, 0.0, v, depth);
            let min: f32 = m.iter().map(|c| c.0).sum::<f32>() + sp;
            let max: f32 = m.iter().map(|c| c.2.unwrap_or(c.1).max(c.0)).sum::<f32>() + sp;
            box_intrinsic(&t.style, (min, max.max(min)))
        }
    };
    v.memo.insert(key, out);
    out
}

/// Column widths for `inner` (spacing removed), the way browsers lay out auto tables: declared
/// widths first, then each column's preferred (max-content) width, never below its minimum;
/// left-over room goes to the columns in proportion to what they asked for.
fn column_widths(metrics: &[(f32, f32, Option<f32>)], inner: f32, fill: bool) -> Vec<f32> {
    let want: Vec<f32> = metrics
        .iter()
        .map(|(min, max, decl)| decl.unwrap_or(*max).max(*min))
        .collect();
    let mins: Vec<f32> = metrics.iter().map(|m| m.0).collect();
    let want_sum: f32 = want.iter().sum();
    let min_sum: f32 = mins.iter().sum();
    if want_sum <= inner {
        if !fill || want_sum <= 0.0 {
            return want;
        }
        // Spread the extra room: to columns without a declared width when there are some.
        let extra = inner - want_sum;
        let auto_sum: f32 = metrics
            .iter()
            .zip(&want)
            .filter(|(m, _)| m.2.is_none())
            .map(|(_, w)| *w)
            .sum();
        return metrics
            .iter()
            .zip(&want)
            .map(|(m, w)| {
                if auto_sum > 0.0 {
                    if m.2.is_none() {
                        w + extra * w / auto_sum
                    } else {
                        *w
                    }
                } else {
                    w + extra * w / want_sum
                }
            })
            .collect();
    }
    if min_sum < inner && want_sum > min_sum {
        // Between the minimum and the preferred widths.
        let k = (inner - min_sum) / (want_sum - min_sum);
        return mins
            .iter()
            .zip(&want)
            .map(|(a, w)| a + (w - a) * k)
            .collect();
    }
    // Not even the minimums fit: scale them down.
    let k = if min_sum > 0.0 { inner / min_sum } else { 0.0 };
    mins.iter().map(|a| a * k).collect()
}

fn table(ui: &mut Ui, t: &Table, v: &mut View<'_>, depth: usize) {
    let cols = t.columns().max(1);
    let avail = ui.available_width();
    let left = ui.cursor().left();
    let top = ui.cursor().top();
    let sp = t.spacing.min(8.0);
    let fill = t.style.width.is_some();
    let mut width = box_width(&t.style, avail);
    let inner = (width - sp * (cols as f32 + 1.0)).max(1.0);
    let metrics = column_metrics(ui, t, cols, width, v, depth);
    let min_sum: f32 = metrics.iter().map(|m| m.0).sum();
    // Columns that cannot fit even at their narrowest go one under the other, as on a phone.
    let stack = cols > 1 && min_sum > inner + 1.0;
    let widths = column_widths(&metrics, inner, fill);
    if !fill && !stack {
        // A table without a width is as wide as its columns.
        let used: f32 = widths.iter().sum::<f32>() + sp * (cols as f32 + 1.0);
        width = used.min(width);
    }
    let x0 = left + align_offset(t.style.align, avail, width);
    let table_idx = ui.painter().add(egui::Shape::Noop);
    let tid = ui.next_auto_id();
    ui.skip_ahead_auto_ids(1);
    let mut y = top + sp;
    for (ri, row) in t.rows.iter().enumerate() {
        if stack {
            for (ci, cell) in row.cells.iter().enumerate() {
                if cell.blocks.is_empty() {
                    continue;
                }
                let salt = tid.with((ri, ci));
                let (rect, idx) = box_at(ui, &cell.style, x0, y, width, 0.0, salt, v, |ui, v| {
                    blocks(ui, &cell.blocks, v, depth.saturating_add(1));
                });
                paint_box(ui, idx, rect, &cell.style);
                y = rect.bottom() + sp;
            }
            continue;
        }
        let mut x = x0 + sp;
        let mut col = 0usize;
        let mut placed: Vec<(Rect, egui::layers::ShapeIdx, usize, egui::Id, f32)> = Vec::new();
        let mut row_h: f32 = 0.0;
        for (ci, cell) in row.cells.iter().enumerate() {
            let span = usize::from(cell.colspan.max(1));
            let w: f32 = widths.iter().skip(col).take(span).sum::<f32>()
                + sp * (span.saturating_sub(1) as f32);
            col = col.saturating_add(span);
            if w < 0.5 {
                continue;
            }
            let id = tid.with((ri, ci));
            // Vertical alignment uses last frame's heights (immediate mode).
            let (prev_content, prev_row): (f32, f32) =
                ui.data(|d| d.get_temp(id)).unwrap_or((0.0, 0.0));
            let slack = (prev_row - prev_content).max(0.0);
            let offset = match cell.valign {
                VAlign::Top => 0.0,
                VAlign::Middle => slack / 2.0,
                VAlign::Bottom => slack,
            };
            let (rect, idx) = box_at(ui, &cell.style, x, y, w, 0.0, id, v, |ui, v| {
                if offset > 0.0 {
                    ui.add_space(offset);
                }
                blocks(ui, &cell.blocks, v, depth.saturating_add(1));
            });
            let natural = (rect.height() - offset).max(0.0);
            row_h = row_h.max(natural);
            placed.push((rect, idx, ci, id, natural));
            x += w + sp;
        }
        for (rect, idx, ci, id, natural) in placed {
            let full = Rect::from_min_size(rect.min, vec2(rect.width(), row_h));
            if let Some(cell) = row.cells.get(ci) {
                paint_box(ui, idx, full, &cell.style);
            }
            let prev: Option<(f32, f32)> = ui.data(|d| d.get_temp(id));
            let now = (natural, row_h);
            if prev.is_none_or(|p| (p.0 - now.0).abs() > 0.5 || (p.1 - now.1).abs() > 0.5) {
                ui.data_mut(|d| d.insert_temp(id, now));
                v.repaint = true;
            }
        }
        y += row_h + sp;
    }
    let rect = Rect::from_min_max(pos2(x0, top), pos2(x0 + width, y));
    paint_box(ui, table_idx, rect, &t.style);
    ui.advance_cursor_after_rect(Rect::from_min_max(pos2(left, top), pos2(left + avail, y)));
}
