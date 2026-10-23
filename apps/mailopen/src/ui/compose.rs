//! The compose window: its own OS window (an egui viewport).
//! Subject on top, "A" with Cc/Bcc at the right, "Da" as a chip, the body, actions at the
//! bottom. Recipient fields suggest addresses from the address book while typing.

use mailopen_core::{Attachment, Contacts};

use crate::app::MailApp;
use crate::ui::icons::{self, Icon};
use crate::ui::{files, format, theme};

impl ComposeState {
    /// Add files from disk; problems go to `error`.
    fn attach_paths(&mut self, paths: &[std::path::PathBuf]) {
        for p in paths {
            match files::read_attachment(p) {
                Ok(a) => self.attachments.push(a),
                Err(e) => self.error = Some(e),
            }
        }
        let total: usize = self.attachments.iter().map(|a| a.size()).sum();
        if total > mailopen_core::smtp_client::MAX_ATTACHMENTS {
            self.error = Some(format!(
                "Allegati troppo grandi ({}): il massimo è 25 MB.",
                format::size(u32::try_from(total).unwrap_or(u32::MAX))
            ));
        }
    }
}

/// The attachments of the message being written, each with a remove button.
fn attachment_chips(ui: &mut egui::Ui, list: &mut Vec<Attachment>) {
    if list.is_empty() {
        return;
    }
    let t = theme::tokens(ui);
    let mut remove = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
        for (i, a) in list.iter().enumerate() {
            egui::Frame::NONE
                .fill(t.surface_alt)
                .stroke(egui::Stroke::new(1.0, t.border))
                .corner_radius(egui::CornerRadius::same(8))
                .inner_margin(egui::Margin {
                    left: 6,
                    right: 2,
                    top: 3,
                    bottom: 3,
                })
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        crate::ui::viewer::file_badge(ui, a, 22.0);
                        let name: String = a.name.chars().take(40).collect();
                        ui.label(egui::RichText::new(name).size(12.5));
                        ui.label(
                            egui::RichText::new(format::size(
                                u32::try_from(a.size()).unwrap_or(u32::MAX),
                            ))
                            .color(t.muted)
                            .size(11.5),
                        );
                        if icons::button(ui, Icon::Close, 9.0, t.muted, "Togli").clicked() {
                            remove = Some(i);
                        }
                    });
                });
        }
    });
    if let Some(i) = remove
        && i < list.len()
    {
        list.remove(i);
    }
}

#[derive(Clone, Debug, Default)]
pub struct ComposeState {
    pub account_index: usize,
    pub to: String,
    pub cc: String,
    pub bcc: String,
    pub show_cc: bool,
    pub show_bcc: bool,
    pub subject: String,
    pub body: String,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub sending: bool,
    pub error: Option<String>,
    pub note: Option<String>,
    pub open: bool,
    /// First frame: focus the right field.
    pub fresh: bool,
    /// Current selection in the body, in chars (start, end).
    pub selection: (usize, usize),
    pub attachments: Vec<Attachment>,
}

impl ComposeState {
    pub fn new(account_index: usize) -> Self {
        ComposeState {
            account_index,
            open: true,
            fresh: true,
            ..Default::default()
        }
    }
}

enum Action {
    Send,
    Close,
}

impl MailApp {
    pub fn compose_window(&mut self, ctx: &egui::Context) {
        if self.compose.is_none() {
            return;
        }
        let title = match self.compose.as_ref().map(|c| c.in_reply_to.is_some()) {
            Some(true) => "Risposta",
            _ => "Nuovo messaggio",
        };
        let id = egui::ViewportId::from_hash_of("mailopen-compose");
        let builder = egui::ViewportBuilder::default()
            .with_title(title)
            .with_inner_size([820.0, 700.0])
            .with_min_inner_size([560.0, 460.0])
            .with_icon(crate::icon::window_icon());
        let action = ctx.show_viewport_immediate(id, builder, |ui, class| {
            if class == egui::ViewportClass::EmbeddedWindow {
                // No native windows here (headless harness): egui draws a floating window for us.
                ui.set_min_size(egui::vec2(760.0, 620.0));
                return egui::Frame::NONE
                    .inner_margin(egui::Margin::symmetric(16, 10))
                    .show(ui, |ui| self.compose_body(ui))
                    .inner;
            }
            let close_requested = ui.input(|i| i.viewport().close_requested());
            let mut action = egui::CentralPanel::default()
                .frame(
                    egui::Frame::NONE
                        .fill(theme::tokens(ui).surface)
                        .inner_margin(egui::Margin::symmetric(22, 16)),
                )
                .show(ui, |ui| self.compose_body(ui))
                .inner;
            if close_requested {
                action = Some(Action::Close);
            }
            action
        });
        match action {
            Some(Action::Send) => self.send_compose(),
            Some(Action::Close) => self.compose = None,
            None => {}
        }
    }

    fn compose_body(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let t = theme::tokens(ui);
        let accounts = self.config.accounts.clone();
        let contacts = &self.contacts;
        let c = self.compose.as_mut()?;
        let mut action = None;
        let fresh = c.fresh;
        c.fresh = false;
        let body_id = egui::Id::unique("compose-body");
        ui.add_enabled_ui(!c.sending, |ui| {
            // Subject: big, no box.
            let subject = ui.add(
                egui::TextEdit::singleline(&mut c.subject)
                    .font(theme::bold_font(24.0))
                    .hint_text(
                        egui::RichText::new("Oggetto")
                            .family(theme::bold_family())
                            .size(24.0),
                    )
                    .frame(egui::Frame::NONE)
                    .margin(egui::Margin::symmetric(0, 6))
                    .desired_width(f32::INFINITY),
            );
            if fresh && c.in_reply_to.is_none() {
                subject.request_focus();
            }
            ui.add_space(6.0);
            // A
            ui.horizontal(|ui| {
                ui.label(theme::bold("A", 14.0));
                ui.add_space(14.0);
                let links_w = 90.0;
                let w = ui.available_width() - links_w;
                let r = recipient_field(ui, "to", &mut c.to, "Destinatario", w, contacts);
                if fresh && c.in_reply_to.is_some() {
                    r.request_focus();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if !c.show_bcc && link(ui, "Bcc").clicked() {
                        c.show_bcc = true;
                    }
                    if !c.show_cc && link(ui, "Cc").clicked() {
                        c.show_cc = true;
                    }
                });
            });
            thin_line(ui, t.border);
            if c.show_cc {
                ui.horizontal(|ui| {
                    ui.label(theme::bold("Cc", 14.0));
                    ui.add_space(8.0);
                    let w = ui.available_width() - 8.0;
                    recipient_field(ui, "cc", &mut c.cc, "Copia conoscenza", w, contacts);
                });
                thin_line(ui, t.border);
            }
            if c.show_bcc {
                ui.horizontal(|ui| {
                    ui.label(theme::bold("Bcc", 14.0));
                    ui.add_space(2.0);
                    let w = ui.available_width() - 8.0;
                    recipient_field(ui, "bcc", &mut c.bcc, "Copia nascosta", w, contacts);
                });
                thin_line(ui, t.border);
            }
            // Da: chip with a menu
            ui.horizontal(|ui| {
                ui.label(theme::bold("Da", 14.0));
                ui.add_space(8.0);
                let current = accounts.get(c.account_index);
                let chip_text = current
                    .map(|a| format!("{}  <{}>", a.name, a.email))
                    .unwrap_or_else(|| "nessun account".into());
                let resp = chip_button(ui, &chip_text, t.accent_soft);
                egui::Popup::menu(&resp).show(|ui| {
                    ui.set_min_width(resp.rect.width());
                    for (i, a) in accounts.iter().enumerate() {
                        if ui
                            .selectable_label(
                                i == c.account_index,
                                format!("{}  <{}>", a.name, a.email),
                            )
                            .clicked()
                        {
                            c.account_index = i;
                            ui.close();
                        }
                    }
                });
            });
            thin_line(ui, t.border);
            ui.add_space(8.0);
            if let Some(n) = &c.note {
                ui.label(egui::RichText::new(n).color(t.muted).size(12.0));
            }
            attachment_chips(ui, &mut c.attachments);
            if !c.attachments.is_empty() {
                ui.add_space(6.0);
            }
            // Files dropped on the window are attached.
            let dropped: Vec<std::path::PathBuf> = ui.input(|i| {
                i.raw
                    .dropped_files
                    .iter()
                    .map(|f| f.path().to_path_buf())
                    .filter(|p| !p.as_os_str().is_empty())
                    .collect()
            });
            if !dropped.is_empty() {
                c.attach_paths(&dropped);
            }
            // Body
            let bottom = 104.0;
            let h = (ui.available_height() - bottom).max(160.0);
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), h), egui::Sense::hover());
            let mut body_ui = ui.new_child(egui::UiBuilder::new().max_rect(rect));
            egui::Frame::NONE
                .fill(t.surface_alt)
                .corner_radius(egui::CornerRadius::same(8))
                .inner_margin(egui::Margin::same(12))
                .show(&mut body_ui, |ui| {
                    ui.set_min_size(rect.size() - egui::vec2(24.0, 24.0));
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            let tokens = t;
                            let mut layouter =
                                |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
                                    let job = crate::ui::markup_view::layout(
                                        buf.as_str(),
                                        wrap,
                                        &tokens,
                                        14.0,
                                    );
                                    ui.fonts_mut(|f| f.layout_job(job))
                                };
                            let out = egui::TextEdit::multiline(&mut c.body)
                                .id(body_id)
                                .hint_text("Cosa vuoi dire oggi?")
                                .frame(egui::Frame::NONE)
                                .desired_rows(10)
                                .desired_width(f32::INFINITY)
                                .layouter(&mut layouter)
                                .show(ui);
                            if let Some(r) = out.cursor_range {
                                let r = r.as_sorted_char_range();
                                c.selection = (r.start.0, r.end.0);
                            }
                        });
                });
            // Formatting toolbar (markup, rendered to HTML when sending)
            ui.add_space(6.0);
            let mut edit: Option<Edit> = None;
            ui.horizontal(|ui| {
                let muted = t.muted;
                for (label, tip, e) in [
                    ("H1", "Titolo", Edit::Line("# ")),
                    ("H2", "Sottotitolo", Edit::Line("## ")),
                    ("B", "Grassetto", Edit::Wrap("**", "**")),
                    ("I", "Corsivo", Edit::Wrap("*", "*")),
                    ("•", "Elenco puntato", Edit::Line("- ")),
                    ("1.", "Elenco numerato", Edit::Numbered),
                    ("Link", "Collegamento", Edit::Link),
                    ("Tx", "Togli formattazione", Edit::Clear),
                ] {
                    let rich = if label == "B" {
                        egui::RichText::new(label)
                            .family(theme::bold_family())
                            .size(13.0)
                            .color(muted)
                    } else if label == "I" {
                        egui::RichText::new(label).italics().size(13.0).color(muted)
                    } else {
                        egui::RichText::new(label).size(13.0).color(muted)
                    };
                    if ui
                        .add(
                            egui::Button::new(rich)
                                .frame(false)
                                .min_size(egui::vec2(30.0, 26.0)),
                        )
                        .on_hover_text(tip)
                        .clicked()
                    {
                        edit = Some(e);
                    }
                }
                ui.label(
                    egui::RichText::new("Seleziona il testo e premi un pulsante.")
                        .color(t.muted)
                        .size(11.0),
                );
            });
            if let Some(e) = edit {
                let (a, b) = c.selection;
                let (text, sel) = apply_edit(&c.body, a, b, e);
                c.body = text;
                c.selection = sel;
                if let Some(mut st) = egui::TextEdit::load_state(ui.ctx(), body_id) {
                    st.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                        egui::text::CCursor::new(sel.0),
                        egui::text::CCursor::new(sel.1),
                    )));
                    egui::TextEdit::store_state(ui.ctx(), body_id, st);
                }
                ui.memory_mut(|m| m.request_focus(body_id));
            }
        });
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let can_send = !c.sending && !c.to.trim().is_empty() && !accounts.is_empty();
            let btn = egui::Button::new(
                egui::RichText::new("Invia")
                    .color(egui::Color32::WHITE)
                    .family(theme::bold_family())
                    .size(14.0),
            )
            .fill(if can_send { t.accent } else { t.border })
            .corner_radius(egui::CornerRadius::same(8))
            .min_size(egui::vec2(84.0, 34.0));
            if ui.add_enabled(can_send, btn).clicked() {
                action = Some(Action::Send);
            }
            if c.sending {
                ui.spinner();
                ui.label(egui::RichText::new("invio…").color(t.muted));
            }
            if let Some(e) = &c.error {
                ui.label(egui::RichText::new(e).color(theme::ERROR).size(12.5));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icons::button(ui, Icon::Trash, 16.0, t.muted, "Scarta il messaggio").clicked() {
                    action = Some(Action::Close);
                }
                if icons::button(
                    ui,
                    Icon::Attachment,
                    16.0,
                    t.muted,
                    "Allega file (o trascinali nella finestra)",
                )
                .clicked()
                {
                    let paths = files::pick_files();
                    c.attach_paths(&paths);
                }
            });
        });
        action
    }
}

fn thin_line(ui: &mut egui::Ui, color: egui::Color32) {
    let r = ui.available_rect_before_wrap();
    let y = ui.cursor().top() + 2.0;
    ui.painter().line_segment(
        [egui::pos2(r.left(), y), egui::pos2(r.right(), y)],
        egui::Stroke::new(1.0, color),
    );
    ui.add_space(6.0);
}

fn link(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let t = theme::tokens(ui);
    ui.add(
        egui::Label::new(
            egui::RichText::new(text)
                .family(theme::bold_family())
                .size(13.0)
                .color(t.muted),
        )
        .sense(egui::Sense::click()),
    )
    .on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn chip_button(ui: &mut egui::Ui, text: &str, fill: egui::Color32) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(
        text.to_string(),
        egui::FontId::proportional(13.0),
        theme::tokens(ui).text,
    );
    let size = egui::vec2(galley.size().x + 36.0, 26.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        p.rect_filled(rect, egui::CornerRadius::same(13), fill);
        p.galley(
            egui::pos2(rect.left() + 12.0, rect.center().y - galley.size().y / 2.0),
            galley,
            theme::tokens(ui).text,
        );
        // small chevron
        let cx = rect.right() - 14.0;
        let cy = rect.center().y - 2.0;
        let s = egui::Stroke::new(1.5, theme::tokens(ui).muted);
        p.line_segment([egui::pos2(cx - 4.0, cy), egui::pos2(cx, cy + 4.0)], s);
        p.line_segment([egui::pos2(cx, cy + 4.0), egui::pos2(cx + 4.0, cy)], s);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Text before the last comma (kept) and the token being typed.
fn split_last_token(text: &str) -> (String, String) {
    match text.rfind([',', ';']) {
        Some(i) => {
            let head = text.get(..=i).unwrap_or("").to_string();
            let tail = text.get(i + 1..).unwrap_or("").to_string();
            (format!("{} ", head.trim_end()), tail)
        }
        None => (String::new(), text.to_string()),
    }
}

/// A frameless address field with a suggestion list from the address book.
fn recipient_field(
    ui: &mut egui::Ui,
    id: &str,
    text: &mut String,
    hint: &str,
    width: f32,
    contacts: &Contacts,
) -> egui::Response {
    let resp = ui.add(
        egui::TextEdit::singleline(text)
            .id(egui::Id::unique(("recipient", id)))
            .hint_text(hint)
            .frame(egui::Frame::NONE)
            .margin(egui::Margin::symmetric(4, 6))
            .desired_width(width),
    );
    let (head, token) = split_last_token(text);
    let token = token.trim().to_string();
    if resp.has_focus() && !token.is_empty() {
        let hits = contacts.suggest(&token, 6);
        if !hits.is_empty() {
            let enter =
                ui.input(|i| i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::Tab));
            let mut pick = if enter { hits.first().cloned() } else { None };
            let area = egui::Area::new(egui::Id::unique(("recipient-popup", id)))
                .order(egui::Order::Foreground)
                .fixed_pos(resp.rect.left_bottom() + egui::vec2(0.0, 2.0));
            area.show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_min_width(resp.rect.width().min(420.0));
                    for h in &hits {
                        let label = if h.name.is_empty() {
                            h.email.clone()
                        } else {
                            format!("{}   {}", h.name, h.email)
                        };
                        if ui.selectable_label(false, label).clicked() {
                            pick = Some(h.clone());
                        }
                    }
                });
            });
            if let Some(h) = pick {
                *text = format!("{head}{}, ", h.display());
                resp.request_focus();
            }
        }
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::split_last_token;

    #[test]
    fn splits_tokens() {
        assert_eq!(split_last_token("ann"), ("".into(), "ann".into()));
        assert_eq!(
            split_last_token("Bo <bo@x.it>, ann"),
            ("Bo <bo@x.it>, ".into(), " ann".into())
        );
        assert_eq!(split_last_token("a@b.it;"), ("a@b.it; ".into(), "".into()));
    }
}

/// What a toolbar button does to the body text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Edit {
    Wrap(&'static str, &'static str),
    Line(&'static str),
    Numbered,
    Link,
    Clear,
}

fn char_to_byte(s: &str, idx: usize) -> usize {
    s.char_indices().nth(idx).map(|(b, _)| b).unwrap_or(s.len())
}

/// Apply an edit to `text` with the selection `[a, b)` in chars. Returns the new text and the
/// new selection (chars).
fn apply_edit(text: &str, a: usize, b: usize, edit: Edit) -> (String, (usize, usize)) {
    let n = text.chars().count();
    let (a, b) = (a.min(n), b.min(n));
    let (a, b) = if a <= b { (a, b) } else { (b, a) };
    let (ba, bb) = (char_to_byte(text, a), char_to_byte(text, b));
    let before = text.get(..ba).unwrap_or("");
    let sel = text.get(ba..bb).unwrap_or("");
    let after = text.get(bb..).unwrap_or("");
    match edit {
        Edit::Wrap(pre, post) => {
            let word = if sel.is_empty() { "testo" } else { sel };
            let out = format!("{before}{pre}{word}{post}{after}");
            let start = a + pre.chars().count();
            (out, (start, start + word.chars().count()))
        }
        Edit::Link => {
            let (label, url) = if sel.starts_with("http") {
                ("link", sel)
            } else if sel.is_empty() {
                ("testo", "https://")
            } else {
                (sel, "https://")
            };
            let out = format!("{before}[{label}]({url}){after}");
            let start = a + 1 + label.chars().count() + 2;
            (out, (start, start + url.chars().count()))
        }
        Edit::Line(_) | Edit::Numbered => {
            // Whole lines of the selection.
            let line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
            let line_end = after.find('\n').map(|i| bb + i).unwrap_or(text.len());
            let block = text.get(line_start..line_end).unwrap_or("");
            let mut rebuilt = String::new();
            for (i, line) in block.split('\n').enumerate() {
                if i > 0 {
                    rebuilt.push('\n');
                }
                let p = match edit {
                    Edit::Numbered => format!("{}. ", i + 1),
                    Edit::Line(p) => p.to_string(),
                    _ => String::new(),
                };
                rebuilt.push_str(&p);
                rebuilt.push_str(strip_line_markup(line));
            }
            let head = text.get(..line_start).unwrap_or("");
            let tail = text.get(line_end..).unwrap_or("");
            let out = format!("{head}{rebuilt}{tail}");
            let s = head.chars().count();
            (out, (s, s + rebuilt.chars().count()))
        }
        Edit::Clear => {
            let cleared = if sel.is_empty() {
                strip_all_markup(text)
            } else {
                format!("{before}{}{after}", strip_inline_markup(sel))
            };
            let len = cleared.chars().count();
            (cleared, (a.min(len), a.min(len)))
        }
    }
}

fn strip_line_markup(line: &str) -> &str {
    let t = line.trim_start();
    for p in ["## ", "# ", "- ", "* "] {
        if let Some(r) = t.strip_prefix(p) {
            return r;
        }
    }
    let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0
        && let Some(r) = t.get(digits..).and_then(|r| r.strip_prefix(". "))
    {
        return r;
    }
    t
}

fn strip_inline_markup(s: &str) -> String {
    s.replace("**", "").replace('*', "")
}

fn strip_all_markup(s: &str) -> String {
    s.lines()
        .map(|l| strip_inline_markup(strip_line_markup(l)))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod edit_tests {
    use super::*;

    #[test]
    fn wraps_lines_and_clears() {
        assert_eq!(
            apply_edit("ciao mondo", 5, 10, Edit::Wrap("**", "**")),
            ("ciao **mondo**".into(), (7, 12))
        );
        assert_eq!(
            apply_edit("", 0, 0, Edit::Wrap("*", "*")),
            ("*testo*".into(), (1, 6))
        );
        assert_eq!(
            apply_edit("uno\ndue", 0, 7, Edit::Line("- ")),
            ("- uno\n- due".into(), (0, 11))
        );
        assert_eq!(
            apply_edit("- uno\n- due", 0, 10, Edit::Numbered).0,
            "1. uno\n2. due"
        );
        assert_eq!(apply_edit("# T\n**b** *i*", 0, 0, Edit::Clear).0, "T\nb i");
        assert_eq!(apply_edit("x", 0, 1, Edit::Link).0, "[x](https://)");
        assert_eq!(apply_edit("àè", 1, 2, Edit::Wrap("**", "**")).0, "à**è**");
        let _ = apply_edit("abc", 9, 2, Edit::Clear);
    }
}
