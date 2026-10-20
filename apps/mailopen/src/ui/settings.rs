//! Settings window: theme, refresh interval, messages per folder.

use mailopen_core::{Account, SignatureKind, ThemePref};

use crate::app::MailApp;
use crate::ui::theme;

impl MailApp {
    /// Flip between light and dark (from "system" it goes to the opposite of what is shown).
    pub fn toggle_theme(&mut self, ctx: &egui::Context, dark_now: bool) {
        self.config.theme = if dark_now {
            ThemePref::Light
        } else {
            ThemePref::Dark
        };
        theme::apply_pref(ctx, self.config.theme);
        self.save_settings();
    }

    pub fn save_settings(&mut self) {
        if std::env::var_os("MAILOPEN_DEMO").is_none()
            && let Err(e) = self.config.save()
        {
            self.set_status(format!("Impostazioni non salvate: {e}"), true);
        }
    }

    pub fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }
        let mut open = true;
        let mut changed = false;
        let mut close = false;
        egui::Window::new("Impostazioni")
            .id(egui::Id::unique("settings"))
            .collapsible(false)
            .resizable(false)
            .fixed_size([660.0, (ctx.content_rect().height() - 90.0).clamp(420.0, 900.0)])
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .open(&mut open)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.label(theme::bold("Aspetto", 13.0));
                        ui.horizontal(|ui| {
                            for (pref, label) in [
                                (ThemePref::System, "Come il sistema"),
                                (ThemePref::Light, "Chiaro"),
                                (ThemePref::Dark, "Scuro"),
                            ] {
                                if ui
                                    .radio_value(&mut self.config.theme, pref, label)
                                    .changed()
                                {
                                    changed = true;
                                }
                            }
                        });
                        ui.add_space(8.0);
                        ui.label(
                            egui::RichText::new("Stile")
                                .color(theme::tokens(ui).muted)
                                .size(12.0),
                        );
                        if let Some(Some(name)) =
                            style_picker(ui, "style-general", Some(&self.config.style), false)
                        {
                            self.config.style = name;
                            changed = true;
                        }
                        ui.add_space(10.0);
                        ui.label(theme::bold("Posta", 13.0));
                        if ui
                            .checkbox(
                                &mut self.config.sender_logos,
                                "Mostra il logo dei mittenti (Gravatar dell'indirizzo, poi icona del dominio)",
                            )
                            .changed()
                        {
                            changed = true;
                        }
                        egui::Grid::new("settings-grid")
                            .num_columns(2)
                            .spacing([12.0, 8.0])
                            .show(ui, |ui| {
                                ui.label("Aggiorna ogni (secondi)");
                                if ui
                                    .add(
                                        egui::DragValue::new(&mut self.config.refresh_seconds)
                                            .range(20..=3600),
                                    )
                                    .changed()
                                {
                                    changed = true;
                                }
                                ui.end_row();
                                ui.label("Messaggi per cartella");
                                if ui
                                    .add(
                                        egui::DragValue::new(&mut self.config.messages_per_folder)
                                            .range(10..=500),
                                    )
                                    .changed()
                                {
                                    changed = true;
                                }
                                ui.end_row();
                            });
                        ui.add_space(10.0);
                        ui.label(theme::bold("Account", 13.0));
                        for (i, a) in self.config.accounts.iter_mut().enumerate() {
                            ui.add_space(6.0);
                            ui.label(theme::bold(format!("{} <{}>", a.name, a.email), 12.5));
                            ui.label(
                                egui::RichText::new("Stile quando questo account è selezionato")
                                    .color(theme::tokens(ui).muted)
                                    .size(11.5),
                            );
                            if let Some(pick) =
                                style_picker(ui, &format!("style-{i}"), a.style.as_deref(), true)
                            {
                                a.style = pick;
                                changed = true;
                            }
                            ui.add_space(4.0);
                            if signature_editor(ui, a) {
                                changed = true;
                            }
                        }
                        ui.add_space(10.0);
                        if let Ok(p) = mailopen_core::Config::path() {
                            ui.label(
                                egui::RichText::new(format!("Configurazione: {}", p.display()))
                                    .color(theme::tokens(ui).muted)
                                    .size(11.5),
                            );
                        }
                        ui.add_space(6.0);
                        if ui.button("Chiudi").clicked() {
                            close = true;
                        }
                    });
            });
        if changed {
            theme::apply_pref(ctx, self.config.theme);
            self.save_settings();
        }
        if close || !open {
            self.settings_open = false;
        }
    }
}

/// Row of colour swatches. Returns `Some(choice)` when clicked: `Some(Some(name))` for a style,
/// `Some(None)` for "like the general one" (only when `allow_general`).
pub fn style_picker(
    ui: &mut egui::Ui,
    id: &str,
    current: Option<&str>,
    allow_general: bool,
) -> Option<Option<String>> {
    let mut picked: Option<Option<String>> = None;
    let t = theme::tokens(ui);
    ui.horizontal_wrapped(|ui| {
        if allow_general {
            let selected = current.is_none();
            let label = egui::RichText::new("Generale").size(12.0);
            if ui.selectable_label(selected, label).clicked() {
                picked = Some(None);
            }
        }
        for s in theme::STYLES.iter() {
            let selected = current == Some(s.name);
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(26.0, 26.0), egui::Sense::click());
            if ui.is_rect_visible(rect) {
                let p = ui.painter();
                p.circle_filled(rect.center(), 10.0, s.accent);
                if selected {
                    p.circle_stroke(rect.center(), 12.5, egui::Stroke::new(2.0, t.text));
                } else if resp.hovered() {
                    p.circle_stroke(rect.center(), 12.5, egui::Stroke::new(1.5, t.muted));
                }
            }
            if resp.on_hover_text(s.name).clicked() {
                picked = Some(Some(s.name.to_string()));
            }
        }
        let _ = egui::Id::unique(id);
    });
    picked
}

/// Signature: plain editor, or pasted HTML. Returns true when anything changed.
pub fn signature_editor(ui: &mut egui::Ui, a: &mut Account) -> bool {
    let mut changed = false;
    let t = theme::tokens(ui);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Firma").color(t.muted).size(11.5));
        if ui
            .radio_value(
                &mut a.signature_kind,
                SignatureKind::Text,
                "Editor classico",
            )
            .changed()
        {
            changed = true;
        }
        if ui
            .radio_value(
                &mut a.signature_kind,
                SignatureKind::Html,
                "HTML (copia e incolla)",
            )
            .changed()
        {
            changed = true;
        }
    });
    match a.signature_kind {
        SignatureKind::Text => {
            if ui
                .add(
                    egui::TextEdit::multiline(&mut a.signature)
                        .hint_text("Cordiali saluti,\nNome Cognome")
                        .desired_rows(3)
                        .desired_width(560.0),
                )
                .changed()
            {
                changed = true;
            }
        }
        SignatureKind::Html => {
            ui.label(
                egui::RichText::new("Incolla qui il codice HTML della firma. Viene spedita come parte HTML del messaggio; chi legge solo testo vede la versione senza formattazione.")
                    .color(t.muted)
                    .size(11.5),
            );
            if ui
                .add(
                    egui::TextEdit::multiline(&mut a.signature_html)
                        .hint_text("<p><b>Nome Cognome</b><br>Azienda · +39 …</p>")
                        .font(egui::TextStyle::Monospace)
                        .desired_rows(5)
                        .desired_width(560.0),
                )
                .changed()
            {
                changed = true;
            }
        }
    }
    changed
}
