//! Sorting mail: read/unread, flag, categories, move to a folder. The context menu of a message,
//! the category chips and the category editor of Impostazioni.

use mailopen_core::{Category, Labels, MessageSummary};

use crate::app::{MailApp, MsgKey};
use crate::ui::theme;

/// Colour of a flagged message's mark.
pub const FLAG: egui::Color32 = egui::Color32::from_rgb(0xE5, 0x48, 0x3E);

pub enum Organize {
    ToggleSeen,
    ToggleFlag,
    Category(String),
    ManageCategories,
    Move(String),
    Delete,
}

impl MailApp {
    pub fn organize(&mut self, key: MsgKey, action: Organize) {
        match action {
            Organize::ToggleSeen => self.toggle_seen(&key),
            Organize::ToggleFlag => self.toggle_flag(&key),
            Organize::Category(id) => self.toggle_category(&key, &id),
            Organize::ManageCategories => {
                self.settings_open = true;
                self.settings_categories = true;
            }
            Organize::Move(to) => self.move_message(key, to),
            Organize::Delete => self.delete_message(key),
        }
    }
}

pub fn category_color(c: &Category) -> egui::Color32 {
    theme::account_color(c.color)
}

/// The right-click menu of a message. Rows whose place on the server is unknown (moved by hand
/// to another folder, not reloaded yet) only offer the categories.
pub fn message_menu(ui: &mut egui::Ui, app: &MailApp, m: &MessageSummary) -> Option<Organize> {
    let mut out = None;
    let live = m.uid != 0;
    if live {
        let seen = if m.seen {
            "Segna come non letto"
        } else {
            "Segna come letto"
        };
        if ui.button(seen).clicked() {
            out = Some(Organize::ToggleSeen);
            ui.close();
        }
        let flag = if m.flagged {
            "Rimuovi contrassegno"
        } else {
            "Contrassegna"
        };
        if ui.button(flag).clicked() {
            out = Some(Organize::ToggleFlag);
            ui.close();
        }
        ui.separator();
    }
    ui.menu_button("Categoria", |ui| {
        if let Some(a) = category_items(ui, &app.labels, m) {
            out = Some(a);
        }
    });
    let targets = if live {
        app.move_targets(&m.account_id, &m.folder)
    } else {
        Vec::new()
    };
    if !targets.is_empty() {
        ui.menu_button("Sposta in", |ui| {
            if let Some(to) = move_items(ui, &targets) {
                out = Some(Organize::Move(to));
            }
        });
    }
    if live {
        ui.separator();
        if ui.button("Elimina").clicked() {
            out = Some(Organize::Delete);
            ui.close();
        }
    }
    out
}

/// One checkbox per category (several can be ticked without closing the menu).
pub fn category_items(ui: &mut egui::Ui, labels: &Labels, m: &MessageSummary) -> Option<Organize> {
    let mut out = None;
    if labels.categories.is_empty() {
        ui.label(
            egui::RichText::new("Nessuna categoria")
                .color(theme::tokens(ui).muted)
                .size(12.0),
        );
    }
    for c in &labels.categories {
        let mut on = labels.has(m, &c.id);
        ui.horizontal(|ui| {
            dot(ui, category_color(c));
            if ui.checkbox(&mut on, c.name.as_str()).clicked() {
                out = Some(Organize::Category(c.id.clone()));
            }
        });
    }
    ui.separator();
    if ui.button("Gestisci categorie…").clicked() {
        out = Some(Organize::ManageCategories);
        ui.close();
    }
    out
}

/// One button per folder; returns the folder picked.
pub fn move_items(ui: &mut egui::Ui, targets: &[(String, String)]) -> Option<String> {
    let mut out = None;
    egui::ScrollArea::vertical()
        .max_height(360.0)
        .show(ui, |ui| {
            for (name, label) in targets {
                if ui.button(label.as_str()).on_hover_text(name).clicked() {
                    out = Some(name.clone());
                    ui.close();
                }
            }
        });
    out
}

fn dot(ui: &mut egui::Ui, color: egui::Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
    ui.painter()
        .rect_filled(rect.shrink(1.0), egui::CornerRadius::same(2), color);
}

/// A category pill painted at `left` (vertically centred on `center_y`). Returns its width,
/// 0 when it does not fit in `max_w`.
pub fn paint_chip(
    p: &egui::Painter,
    left: f32,
    center_y: f32,
    name: &str,
    color: egui::Color32,
    on_accent: bool,
    max_w: f32,
) -> f32 {
    let (fill, text) = if on_accent {
        (egui::Color32::from_white_alpha(46), egui::Color32::WHITE)
    } else {
        (color.gamma_multiply(0.16), color)
    };
    let galley = p.layout_no_wrap(name.to_string(), theme::bold_font(10.5), text);
    let w = galley.size().x + 12.0;
    if w > max_w {
        return 0.0;
    }
    let rect = egui::Rect::from_min_size(egui::pos2(left, center_y - 8.0), egui::vec2(w, 16.0));
    p.rect_filled(rect, egui::CornerRadius::same(8), fill);
    p.galley(
        egui::pos2(rect.left() + 6.0, rect.center().y - galley.size().y / 2.0),
        galley,
        text,
    );
    w
}

/// Chips as a row of widgets (reading pane).
pub fn chips(ui: &mut egui::Ui, cats: &[&Category]) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        for c in cats {
            let galley = ui.painter().layout_no_wrap(
                c.name.clone(),
                theme::bold_font(10.5),
                category_color(c),
            );
            let (rect, _) = ui.allocate_exact_size(
                egui::vec2(galley.size().x + 12.0, 18.0),
                egui::Sense::hover(),
            );
            paint_chip(
                ui.painter(),
                rect.left(),
                rect.center().y,
                &c.name,
                category_color(c),
                false,
                f32::INFINITY,
            );
        }
    });
}

/// Category list in Impostazioni: name, colour, keywords. Returns true when anything changed.
pub fn editor(ui: &mut egui::Ui, labels: &mut Labels) -> bool {
    let t = theme::tokens(ui);
    let mut changed = false;
    let mut remove: Option<String> = None;
    ui.label(
        egui::RichText::new(
            "Un messaggio entra in una categoria quando l'oggetto contiene una delle parole \
             chiave (anche come inizio di parola: «ordin» trova «ordine» e «ordini»), o quando \
             lo metti tu con il tasto destro. Le parole con @ guardano il mittente \
             («@fornitore.it»). Senza parole chiave la categoria si usa solo a mano.",
        )
        .color(t.muted)
        .size(11.5),
    );
    ui.add_space(6.0);
    for c in &mut labels.categories {
        ui.horizontal(|ui| {
            if color_button(ui, c) {
                changed = true;
            }
            if ui
                .add(
                    egui::TextEdit::singleline(&mut c.name)
                        .desired_width(130.0)
                        .hint_text("Nome"),
                )
                .changed()
            {
                changed = true;
            }
            let mut words = c.keywords_text();
            if ui
                .add(
                    egui::TextEdit::singleline(&mut words)
                        .desired_width(330.0)
                        .hint_text("parole chiave, separate da virgola"),
                )
                .changed()
            {
                c.set_keywords_text(&words);
                changed = true;
            }
            if ui
                .small_button("Elimina")
                .on_hover_text("Elimina la categoria (i messaggi restano dove sono)")
                .clicked()
            {
                remove = Some(c.id.clone());
            }
        });
    }
    if let Some(id) = remove {
        labels.remove_category(&id);
        changed = true;
    }
    ui.add_space(4.0);
    if ui.button("+ Nuova categoria").clicked() {
        labels.add_category("Nuova categoria");
        changed = true;
    }
    changed
}

/// Colour swatch that cycles through the palette when clicked.
fn color_button(ui: &mut egui::Ui, c: &mut Category) -> bool {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        ui.painter()
            .circle_filled(rect.center(), 8.0, category_color(c));
        if resp.hovered() {
            ui.painter().circle_stroke(
                rect.center(),
                10.0,
                egui::Stroke::new(1.5, theme::tokens(ui).muted),
            );
        }
    }
    let clicked = resp.on_hover_text("Cambia colore").clicked();
    if clicked {
        c.color = c.color.wrapping_add(1) % 8;
    }
    clicked
}
