//! The attachment viewer: its own OS window (an egui viewport). A preview (images, PDF pages,
//! text, attached messages) and the actions: save, open with the default app, print, forward.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::Receiver;

use mailopen_core::worker::Wake;
use mailopen_core::{Attachment, FileKind, MessageBody};

use crate::app::{MailApp, MsgKey};
use crate::ui::html_view;
use crate::ui::icons::{self, Icon};
use crate::ui::images::{ImgState, Source};
use crate::ui::pdf::PdfDoc;
use crate::ui::{files, format, theme};

/// Text previews stop after this many bytes.
const MAX_TEXT: usize = 600 * 1024;

pub enum Content {
    Image,
    Pdf(Box<PdfDoc>),
    Text { text: String, mono: bool, cut: bool },
    Message(Box<MessageBody>),
    None,
}

pub struct Viewer {
    /// The message the attachment belongs to (for forwarding).
    pub message: Option<MsgKey>,
    pub attachment: Attachment,
    pub content: Content,
    /// 1.0 = fit the window.
    pub zoom: f32,
    pub status: Option<(String, bool)>,
    pub task: Option<Receiver<Result<String, String>>>,
    pub last_saved: Option<PathBuf>,
    image_key: String,
    /// Shown above the preview (e.g. simplified Office rendering).
    note: Option<&'static str>,
}

impl Viewer {
    pub fn new(message: Option<MsgKey>, attachment: Attachment, wake: Wake) -> Viewer {
        let content = match attachment.kind() {
            FileKind::Image => Content::Image,
            FileKind::Pdf => Content::Pdf(Box::new(PdfDoc::open(
                Arc::clone(&attachment.data),
                1400,
                wake,
            ))),
            FileKind::Text => {
                let cut = attachment.data.len() > MAX_TEXT;
                let bytes = attachment.data.get(..MAX_TEXT).unwrap_or(&attachment.data);
                let mono = matches!(
                    attachment.extension().as_str(),
                    "csv"
                        | "tsv"
                        | "json"
                        | "xml"
                        | "log"
                        | "sql"
                        | "ics"
                        | "vcf"
                        | "ini"
                        | "yaml"
                        | "yml"
                        | "toml"
                );
                Content::Text {
                    text: files::decode_text(bytes),
                    mono,
                    cut,
                }
            }
            FileKind::Message => Content::Message(Box::new(
                mailopen_core::imap_client::parse_body(&attachment.data),
            )),
            FileKind::Document | FileKind::Spreadsheet => {
                let doc = match attachment.extension().as_str() {
                    "docx" => mailopen_core::office::docx(&attachment.data),
                    "xlsx" | "xlsm" => mailopen_core::office::xlsx(&attachment.data),
                    _ => None,
                };
                match doc {
                    Some(doc) => Content::Message(Box::new(MessageBody {
                        doc: Arc::new(doc),
                        is_html: true,
                        ..MessageBody::default()
                    })),
                    None => Content::None,
                }
            }
            _ => Content::None,
        };
        let note = matches!(
            (&content, attachment.kind()),
            (
                Content::Message(_),
                FileKind::Document | FileKind::Spreadsheet
            )
        )
        .then_some("Anteprima semplificata: per impaginazione e formule complete usa «Apri».");
        let image_key = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            attachment.name.hash(&mut h);
            attachment.data.len().hash(&mut h);
            format!("viewer#{:x}", h.finish())
        };
        Viewer {
            message,
            attachment,
            content,
            zoom: 1.0,
            status: None,
            task: None,
            last_saved: None,
            image_key,
            note,
        }
    }
}

pub enum Action {
    Save,
    Open,
    Print,
    Forward,
    Reveal(PathBuf),
    Link(String),
    Close,
}

/// A coloured badge with the file's extension, like a document icon.
pub fn file_badge(ui: &mut egui::Ui, a: &Attachment, height: f32) -> egui::Response {
    let (fill, label) = badge_style(a);
    let w = height * 0.82;
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, height), egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let fold = (w * 0.3).min(10.0);
        let pts = vec![
            rect.left_top(),
            egui::pos2(rect.right() - fold, rect.top()),
            egui::pos2(rect.right(), rect.top() + fold),
            rect.right_bottom(),
            rect.left_bottom(),
        ];
        p.add(egui::Shape::convex_polygon(pts, fill, egui::Stroke::NONE));
        p.add(egui::Shape::convex_polygon(
            vec![
                egui::pos2(rect.right() - fold, rect.top()),
                egui::pos2(rect.right(), rect.top() + fold),
                egui::pos2(rect.right() - fold, rect.top() + fold),
            ],
            egui::Color32::from_white_alpha(110),
            egui::Stroke::NONE,
        ));
        let size = (height * 0.27).clamp(7.0, 13.0);
        p.text(
            egui::pos2(rect.center().x, rect.bottom() - height * 0.3),
            egui::Align2::CENTER_CENTER,
            label,
            theme::bold_font(size),
            egui::Color32::WHITE,
        );
    }
    resp
}

fn badge_style(a: &Attachment) -> (egui::Color32, String) {
    use egui::Color32 as C;
    let ext = a.extension();
    let label = if ext.is_empty() {
        "FILE".to_string()
    } else {
        ext.chars().take(4).collect::<String>().to_uppercase()
    };
    let fill = match a.kind() {
        FileKind::Pdf => C::from_rgb(0xDC, 0x3B, 0x2F),
        FileKind::Image => C::from_rgb(0x8B, 0x5C, 0xF6),
        FileKind::Document => C::from_rgb(0x2B, 0x6C, 0xD9),
        FileKind::Spreadsheet => C::from_rgb(0x1E, 0x8E, 0x4E),
        FileKind::Presentation => C::from_rgb(0xE0, 0x6B, 0x1F),
        FileKind::Archive => C::from_rgb(0x8A, 0x6D, 0x3B),
        FileKind::Text => C::from_rgb(0x55, 0x65, 0x78),
        FileKind::Message => C::from_rgb(0x2F, 0x7B, 0xF5),
        FileKind::Audio | FileKind::Video => C::from_rgb(0xDB, 0x27, 0x77),
        FileKind::Other => C::from_rgb(0x6B, 0x72, 0x80),
    };
    (fill, label)
}

fn tool_button(ui: &mut egui::Ui, icon: Icon, label: &str, tip: &str) -> egui::Response {
    let t = theme::tokens(ui);
    let galley =
        ui.painter()
            .layout_no_wrap(label.to_string(), egui::FontId::proportional(13.0), t.text);
    let size = egui::vec2(galley.size().x + 40.0, 32.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let fill = if resp.hovered() { t.hover } else { t.surface };
        p.rect_filled(rect, egui::CornerRadius::same(8), fill);
        p.rect_stroke(
            rect,
            egui::CornerRadius::same(8),
            egui::Stroke::new(1.0, t.border),
            egui::StrokeKind::Inside,
        );
        icons::at(
            p,
            egui::pos2(rect.left() + 17.0, rect.center().y),
            15.0,
            icon,
            t.text,
        );
        p.galley(
            egui::pos2(rect.left() + 30.0, rect.center().y - galley.size().y / 2.0),
            galley,
            t.text,
        );
    }
    resp.on_hover_text(tip)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn primary_button(ui: &mut egui::Ui, icon: Icon, label: &str, tip: &str) -> egui::Response {
    let t = theme::tokens(ui);
    let galley = ui.painter().layout_no_wrap(
        label.to_string(),
        theme::bold_font(13.0),
        egui::Color32::WHITE,
    );
    let size = egui::vec2(galley.size().x + 42.0, 32.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let fill = if resp.hovered() {
            t.accent.gamma_multiply(0.9)
        } else {
            t.accent
        };
        p.rect_filled(rect, egui::CornerRadius::same(8), fill);
        icons::at(
            p,
            egui::pos2(rect.left() + 18.0, rect.center().y),
            15.0,
            icon,
            egui::Color32::WHITE,
        );
        p.galley(
            egui::pos2(rect.left() + 31.0, rect.center().y - galley.size().y / 2.0),
            galley,
            egui::Color32::WHITE,
        );
    }
    resp.on_hover_text(tip)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

impl MailApp {
    pub fn open_attachment(&mut self, message: Option<MsgKey>, a: Attachment) {
        self.viewer = Some(Viewer::new(message, a, Arc::clone(&self.wake)));
    }

    pub fn viewer_window(&mut self, ctx: &egui::Context) {
        let Some(v) = self.viewer.as_mut() else {
            return;
        };
        if let Content::Pdf(pdf) = &mut v.content {
            pdf.poll(ctx);
        }
        if let Some(rx) = &v.task
            && let Ok(result) = rx.try_recv()
        {
            v.status = Some(match result {
                Ok(s) => (s, false),
                Err(e) => (e, true),
            });
            v.task = None;
        }
        let title = v.attachment.name.clone();
        let id = egui::ViewportId::from_hash_of("mailopen-viewer");
        let builder = egui::ViewportBuilder::default()
            .with_title(title)
            .with_inner_size([980.0, 800.0])
            .with_min_inner_size([520.0, 420.0])
            .with_icon(crate::icon::window_icon());
        let action = ctx.show_viewport_immediate(id, builder, |ui, class| {
            if class == egui::ViewportClass::EmbeddedWindow {
                ui.set_min_size(egui::vec2(820.0, 640.0));
                return egui::Frame::NONE
                    .inner_margin(egui::Margin::symmetric(16, 10))
                    .show(ui, |ui| self.viewer_body(ui))
                    .inner;
            }
            let close_requested = ui.input(|i| i.viewport().close_requested());
            let mut action = egui::CentralPanel::default()
                .frame(
                    egui::Frame::NONE
                        .fill(theme::tokens(ui).surface)
                        .inner_margin(egui::Margin::symmetric(18, 14)),
                )
                .show(ui, |ui| self.viewer_body(ui))
                .inner;
            if close_requested {
                action = Some(Action::Close);
            }
            action
        });
        let Some(action) = action else { return };
        match action {
            Action::Close => self.viewer = None,
            Action::Save => {
                let Some(v) = self.viewer.as_mut() else {
                    return;
                };
                match files::save_dialog(&v.attachment) {
                    Ok(Some(path)) => {
                        v.status = Some((format!("Salvato in {}", path.display()), false));
                        v.last_saved = Some(path);
                    }
                    Ok(None) => {}
                    Err(e) => v.status = Some((e, true)),
                }
            }
            Action::Open => {
                let Some(v) = self.viewer.as_mut() else {
                    return;
                };
                if let Err(e) = files::open_with_default(&v.attachment) {
                    v.status = Some((e, true));
                }
            }
            Action::Print => {
                let Some(v) = self.viewer.as_mut() else {
                    return;
                };
                v.status = Some(("Preparo la stampa…".into(), false));
                v.task = Some(files::print(v.attachment.clone()));
            }
            Action::Forward => {
                let Some(v) = self.viewer.as_ref() else {
                    return;
                };
                let message = v.message.clone();
                let a = v.attachment.clone();
                self.forward_with(message, vec![a]);
            }
            Action::Reveal(p) => files::reveal(&p),
            Action::Link(url) => self.open_link(ctx, &url),
        }
    }

    fn viewer_body(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let t = theme::tokens(ui);
        let mut action = None;
        let (save_key, print_key, esc) = ui.input(|i| {
            (
                i.modifiers.command && i.key_pressed(egui::Key::S),
                i.modifiers.command && i.key_pressed(egui::Key::P),
                i.key_pressed(egui::Key::Escape),
            )
        });
        let images = &mut self.images;
        let v = self.viewer.as_mut()?;
        if save_key {
            action = Some(Action::Save);
        }
        if print_key {
            action = Some(Action::Print);
        }
        if esc {
            action = Some(Action::Close);
        }
        // Header: badge, name and size, actions.
        ui.horizontal(|ui| {
            file_badge(ui, &v.attachment, 40.0);
            ui.add_space(4.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.add(egui::Label::new(theme::bold(v.attachment.name.clone(), 15.0)).truncate());
                ui.label(
                    egui::RichText::new(format!(
                        "{}  ·  {}",
                        format::size(u32::try_from(v.attachment.size()).unwrap_or(u32::MAX)),
                        kind_label(v.attachment.kind())
                    ))
                    .color(t.muted)
                    .size(12.0),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if primary_button(ui, Icon::Download, "Salva", "Salva sul computer (Ctrl+S)")
                    .clicked()
                {
                    action = Some(Action::Save);
                }
                if tool_button(ui, Icon::Print, "Stampa", "Stampa (Ctrl+P)").clicked() {
                    action = Some(Action::Print);
                }
                if tool_button(ui, Icon::OpenExternal, "Apri", "Apri con l'app predefinita")
                    .clicked()
                {
                    action = Some(Action::Open);
                }
                if tool_button(
                    ui,
                    Icon::Forward,
                    "Inoltra",
                    "Inoltra in un nuovo messaggio",
                )
                .clicked()
                {
                    action = Some(Action::Forward);
                }
            });
        });
        if let Some((text, is_error)) = &v.status {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let c = if *is_error { theme::ERROR } else { theme::OK };
                ui.label(egui::RichText::new(text).color(c).size(12.5));
                if v.task.is_some() {
                    ui.spinner();
                }
                if let Some(p) = &v.last_saved
                    && !*is_error
                    && ui
                        .link(egui::RichText::new("Mostra nella cartella").size(12.5))
                        .clicked()
                {
                    action = Some(Action::Reveal(p.clone()));
                }
            });
        }
        ui.add_space(8.0);
        // Zoom row for images and PDFs.
        let zoomable = matches!(v.content, Content::Image | Content::Pdf(_));
        if zoomable {
            ui.horizontal(|ui| {
                let muted = t.muted;
                if icons::button(ui, Icon::ZoomOut, 15.0, muted, "Riduci").clicked() {
                    v.zoom = (v.zoom / 1.25).max(0.2);
                }
                ui.label(
                    egui::RichText::new(format!("{:.0}%", v.zoom * 100.0))
                        .color(muted)
                        .size(12.5),
                );
                if icons::button(ui, Icon::ZoomIn, 15.0, muted, "Ingrandisci").clicked() {
                    v.zoom = (v.zoom * 1.25).min(6.0);
                }
                if icons::button(ui, Icon::Fit, 15.0, muted, "Adatta alla finestra").clicked() {
                    v.zoom = 1.0;
                }
                if let Content::Pdf(pdf) = &v.content
                    && !pdf.sizes.is_empty()
                {
                    ui.add_space(10.0);
                    let n = pdf.sizes.len();
                    ui.label(
                        egui::RichText::new(if n == 1 {
                            "1 pagina".to_string()
                        } else {
                            format!("{n} pagine")
                        })
                        .color(muted)
                        .size(12.5),
                    );
                }
            });
            let zoom_delta = ui.input(|i| i.zoom_delta());
            if (zoom_delta - 1.0).abs() > 0.001 {
                v.zoom = (v.zoom * zoom_delta).clamp(0.2, 6.0);
            }
        }
        if let Some(n) = v.note {
            ui.label(egui::RichText::new(n).color(t.muted).size(12.0));
            ui.add_space(4.0);
        }
        theme::hline(ui);
        // Content.
        let area = ui.available_rect_before_wrap();
        egui::Frame::NONE
            .fill(t.surface_alt)
            .corner_radius(egui::CornerRadius::same(8))
            .show(ui, |ui| {
                ui.set_min_size(area.size());
                match &mut v.content {
                    Content::Image => {
                        let key = v.image_key.clone();
                        let data = Arc::clone(&v.attachment.data);
                        let state = images.get(&key, 4096, || Some(Source::Bytes(data)));
                        image_view(ui, state, v.zoom, area.size());
                    }
                    Content::Pdf(pdf) => pdf_view(ui, pdf, v.zoom),
                    Content::Text { text, mono, cut } => {
                        egui::ScrollArea::both()
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                egui::Frame::NONE
                                    .inner_margin(egui::Margin::same(14))
                                    .show(ui, |ui| {
                                        let rich = egui::RichText::new(text.as_str()).size(13.0);
                                        let rich = if *mono { rich.monospace() } else { rich };
                                        ui.add(egui::Label::new(rich).selectable(true));
                                        if *cut {
                                            ui.add_space(8.0);
                                            ui.label(
                                                egui::RichText::new(
                                                    "Anteprima troncata: apri il file per vederlo tutto.",
                                                )
                                                .color(t.muted)
                                                .size(12.0),
                                            );
                                        }
                                    });
                            });
                    }
                    Content::Message(body) => {
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                egui::Frame::NONE
                                    .fill(egui::Color32::WHITE)
                                    .corner_radius(egui::CornerRadius::same(8))
                                    .inner_margin(egui::Margin::same(18))
                                    .show(ui, |ui| {
                                        ui.set_min_width(ui.available_width());
                                        let palette = html_view::Palette::paper();
                                        let doc = Arc::clone(&body.doc);
                                        let mut view = html_view::View::new(
                                            images,
                                            body,
                                            v.image_key.clone(),
                                            false,
                                            palette,
                                        );
                                        html_view::show(ui, &doc, &mut view);
                                        if let Some(url) = view.clicked {
                                            action = Some(Action::Link(url));
                                        }
                                    });
                            });
                    }
                    Content::None => {
                        ui.vertical_centered(|ui| {
                            ui.add_space(area.height() * 0.28);
                            file_badge(ui, &v.attachment, 72.0);
                            ui.add_space(12.0);
                            ui.label(theme::bold(v.attachment.name.clone(), 15.0));
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new(
                                    "Anteprima non disponibile per questo tipo di file.",
                                )
                                .color(t.muted)
                                .size(13.0),
                            );
                            ui.add_space(14.0);
                            ui.horizontal(|ui| {
                                let w = 230.0;
                                ui.add_space(((ui.available_width() - w) / 2.0).max(0.0));
                                if tool_button(ui, Icon::OpenExternal, "Apri con l'app", "Apri con l'app predefinita").clicked() {
                                    action = Some(Action::Open);
                                }
                                if primary_button(ui, Icon::Download, "Salva", "Salva sul computer").clicked() {
                                    action = Some(Action::Save);
                                }
                            });
                        });
                    }
                }
            });
        action
    }
}

fn kind_label(k: FileKind) -> &'static str {
    match k {
        FileKind::Image => "Immagine",
        FileKind::Pdf => "Documento PDF",
        FileKind::Text => "Testo",
        FileKind::Message => "Messaggio e-mail",
        FileKind::Document => "Documento",
        FileKind::Spreadsheet => "Foglio di calcolo",
        FileKind::Presentation => "Presentazione",
        FileKind::Archive => "Archivio compresso",
        FileKind::Audio => "Audio",
        FileKind::Video => "Video",
        FileKind::Other => "File",
    }
}

fn image_view(ui: &mut egui::Ui, state: ImgState, zoom: f32, area: egui::Vec2) {
    let t = theme::tokens(ui);
    match state {
        ImgState::Ready(id, natural) => {
            let avail = (area - egui::vec2(24.0, 24.0)).max(egui::vec2(10.0, 10.0));
            let fit = (avail.x / natural.x.max(1.0))
                .min(avail.y / natural.y.max(1.0))
                .min(1.0);
            let size = natural * fit * zoom;
            egui::ScrollArea::both()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let pad_x = ((area.x - size.x) / 2.0).max(12.0);
                    let pad_y = ((area.y - size.y) / 2.0).max(12.0);
                    ui.add_space(pad_y);
                    ui.horizontal(|ui| {
                        ui.add_space(pad_x);
                        ui.add(
                            egui::Image::new(egui::load::SizedTexture::new(id, size))
                                .fit_to_exact_size(size),
                        );
                        ui.add_space(pad_x);
                    });
                    ui.add_space(pad_y);
                });
        }
        ImgState::Loading => {
            ui.centered_and_justified(|ui| {
                ui.spinner();
            });
        }
        ImgState::Failed => {
            ui.centered_and_justified(|ui| {
                ui.label(
                    egui::RichText::new(
                        "Immagine non leggibile: prova ad aprirla con l'app predefinita.",
                    )
                    .color(t.muted),
                );
            });
        }
    }
}

fn pdf_view(ui: &mut egui::Ui, pdf: &mut PdfDoc, zoom: f32) {
    let t = theme::tokens(ui);
    if let Some(e) = &pdf.error {
        ui.centered_and_justified(|ui| {
            ui.label(egui::RichText::new(e).color(t.muted));
        });
        return;
    }
    if pdf.sizes.is_empty() {
        ui.centered_and_justified(|ui| {
            ui.spinner();
        });
        return;
    }
    let sizes = pdf.sizes.clone();
    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let avail = ui.available_width();
            let base = (avail - 48.0).clamp(200.0, 1100.0);
            let width = base * zoom;
            let viewport = ui.clip_rect();
            ui.add_space(16.0);
            for (i, (w, h)) in sizes.iter().enumerate() {
                let height = width * h / w.max(1.0);
                let pad = ((avail - width) / 2.0).max(16.0);
                let (row, _) = ui.allocate_exact_size(
                    egui::vec2(width + pad * 2.0, height),
                    egui::Sense::hover(),
                );
                let rect = egui::Rect::from_min_size(
                    egui::pos2(row.left() + pad, row.top()),
                    egui::vec2(width, height),
                );
                let near = rect.intersects(viewport.expand2(egui::vec2(0.0, viewport.height())));
                if near {
                    let p = ui.painter();
                    p.rect_filled(
                        rect.translate(egui::vec2(0.0, 2.0)).expand(1.0),
                        egui::CornerRadius::same(2),
                        egui::Color32::from_black_alpha(28),
                    );
                    p.rect_filled(rect, egui::CornerRadius::same(2), egui::Color32::WHITE);
                    match pdf.page(i) {
                        Some(tex) => {
                            p.image(
                                tex,
                                rect,
                                egui::Rect::from_min_max(
                                    egui::pos2(0.0, 0.0),
                                    egui::pos2(1.0, 1.0),
                                ),
                                egui::Color32::WHITE,
                            );
                        }
                        None => {
                            p.text(
                                rect.center(),
                                egui::Align2::CENTER_CENTER,
                                format!("Pagina {}…", i + 1),
                                egui::FontId::proportional(13.0),
                                egui::Color32::from_gray(150),
                            );
                        }
                    }
                }
                ui.add_space(14.0);
            }
        });
}
