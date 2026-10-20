//! Reading pane: icon toolbar, subject, sender line, body.

use crate::app::MailApp;
use crate::ui::icons::{self, Icon};
use crate::ui::{format, theme};

enum Action {
    Reply,
    ReplyAll,
    Forward,
    Delete,
    ToggleSeen,
    Retry,
}

impl MailApp {
    pub fn reader(&mut self, root: &mut egui::Ui) {
        let mut action: Option<Action> = None;
        egui::CentralPanel::default()
            .frame(
                egui::Frame::NONE
                    .fill(theme::tokens(root).surface)
                    .inner_margin(egui::Margin {
                        left: 18,
                        right: 18,
                        top: 10,
                        bottom: 10,
                    }),
            )
            .show(root, |ui| {
                let Some(summary) = self.selected_summary() else {
                    ui.add_space(ui.available_height() * 0.42);
                    ui.vertical_centered(|ui| {
                        icons::small(ui, Icon::Mails, 40.0, theme::tokens(ui).border);
                        ui.add_space(8.0);
                        ui.label(
                            egui::RichText::new("Seleziona un messaggio")
                                .color(theme::tokens(ui).muted)
                                .size(14.0),
                        );
                    });
                    return;
                };
                let key = summary.key();
                ui.horizontal(|ui| {
                    let seen_tip = if summary.seen {
                        "Segna come non letto"
                    } else {
                        "Segna come letto"
                    };
                    let check_color = if summary.seen {
                        theme::tokens(ui).accent
                    } else {
                        theme::tokens(ui).muted
                    };
                    if icons::button(ui, Icon::Check, 16.0, check_color, seen_tip).clicked() {
                        action = Some(Action::ToggleSeen);
                    }
                    if icons::button(
                        ui,
                        Icon::Trash,
                        16.0,
                        theme::tokens(ui).muted,
                        "Elimina (Canc)",
                    )
                    .clicked()
                    {
                        action = Some(Action::Delete);
                    }
                    ui.add_space(10.0);
                    if icons::button(
                        ui,
                        Icon::Reply,
                        16.0,
                        theme::tokens(ui).muted,
                        "Rispondi (⌘R)",
                    )
                    .clicked()
                    {
                        action = Some(Action::Reply);
                    }
                    if icons::button(
                        ui,
                        Icon::ReplyAll,
                        16.0,
                        theme::tokens(ui).muted,
                        "Rispondi a tutti",
                    )
                    .clicked()
                    {
                        action = Some(Action::ReplyAll);
                    }
                    if icons::button(ui, Icon::Forward, 16.0, theme::tokens(ui).muted, "Inoltra")
                        .clicked()
                    {
                        action = Some(Action::Forward);
                    }
                    ui.add_space(10.0);
                    let folder = self
                        .folders
                        .get(&summary.account_id)
                        .and_then(|fs| fs.iter().find(|f| f.name == summary.folder))
                        .map(|f| f.display())
                        .unwrap_or_else(|| summary.folder.clone());
                    chip(ui, &folder);
                });
                ui.add_space(10.0);
                let subject = if summary.subject.is_empty() {
                    "(senza oggetto)".to_string()
                } else {
                    summary.subject.clone()
                };
                ui.add(egui::Label::new(theme::bold(subject, 22.0)).wrap());
                ui.add_space(12.0);
                let body = self.bodies.get(&key);
                ui.horizontal(|ui| {
                    let logo = self.avatars.get(&summary.from_addr);
                    theme::avatar(
                        ui,
                        &theme::initial(summary.sender()),
                        theme::hash_color(&summary.from_addr.to_lowercase()),
                        14.0,
                        logo,
                    );
                    ui.add_space(4.0);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.horizontal(|ui| {
                            ui.label(theme::bold(summary.sender(), 13.5));
                            if !summary.from_name.is_empty() {
                                ui.label(
                                    egui::RichText::new(&summary.from_addr)
                                        .color(theme::tokens(ui).muted)
                                        .size(12.5),
                                );
                            }
                        });
                        let mut to_line = String::new();
                        if let Some(b) = body {
                            if !b.to.is_empty() {
                                to_line = format!("a {}", b.to.join(", "));
                            }
                            if !b.cc.is_empty() {
                                to_line.push_str(&format!("   cc {}", b.cc.join(", ")));
                            }
                        }
                        if !to_line.is_empty() {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(to_line)
                                        .color(theme::tokens(ui).muted)
                                        .size(12.5),
                                )
                                .truncate(),
                            );
                        }
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                        ui.label(
                            egui::RichText::new(format::long_date(summary.timestamp))
                                .color(theme::tokens(ui).muted)
                                .size(12.0),
                        )
                        .on_hover_text(format::size(summary.size));
                    });
                });
                if let Some(b) = body
                    && !b.attachments.is_empty()
                {
                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| {
                        for a in &b.attachments {
                            ui.horizontal(|ui| {
                                icons::small(ui, Icon::Attachment, 13.0, theme::tokens(ui).muted);
                                ui.label(
                                    egui::RichText::new(a)
                                        .color(theme::tokens(ui).muted)
                                        .size(12.5),
                                );
                            });
                        }
                    });
                }
                ui.add_space(12.0);
                match (body, self.body_errors.get(&key)) {
                    (Some(b), _) => {
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                egui::Frame::NONE
                                    .fill(theme::tokens(ui).surface_alt)
                                    .corner_radius(egui::CornerRadius::same(8))
                                    .inner_margin(egui::Margin::same(16))
                                    .show(ui, |ui| {
                                        ui.set_min_width(ui.available_width());
                                        if b.was_html {
                                            ui.label(
                                                egui::RichText::new(
                                                    "Messaggio HTML mostrato come testo.",
                                                )
                                                .color(theme::tokens(ui).muted)
                                                .size(11.5),
                                            );
                                            ui.add_space(6.0);
                                        }
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(&b.text)
                                                    .size(14.0)
                                                    .color(theme::tokens(ui).text),
                                            )
                                            .selectable(true)
                                            .wrap(),
                                        );
                                    });
                                ui.add_space(24.0);
                            });
                    }
                    (None, Some(err)) => {
                        ui.colored_label(theme::ERROR, err);
                        if ui.button("Riprova").clicked() {
                            action = Some(Action::Retry);
                        }
                    }
                    (None, None) => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(
                                egui::RichText::new("Caricamento…").color(theme::tokens(ui).muted),
                            );
                        });
                    }
                }
            });
        match action {
            Some(Action::Reply) => self.reply(false),
            Some(Action::ReplyAll) => self.reply(true),
            Some(Action::Forward) => self.forward(),
            Some(Action::Delete) => self.delete_selected(),
            Some(Action::ToggleSeen) => self.toggle_seen_selected(),
            Some(Action::Retry) => {
                if let Some((acc, folder, uid)) = self.selected.clone() {
                    self.body_errors.remove(&(acc.clone(), folder.clone(), uid));
                    self.request(&acc, mailopen_core::Request::LoadBody { folder, uid });
                }
            }
            None => {}
        }
    }
}

fn chip(ui: &mut egui::Ui, text: &str) {
    let galley = ui.painter().layout_no_wrap(
        text.to_string(),
        egui::FontId::proportional(11.5),
        theme::tokens(ui).muted,
    );
    let size = egui::vec2(galley.size().x + 16.0, 20.0);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        p.rect_filled(
            rect,
            egui::CornerRadius::same(10),
            theme::tokens(ui).surface_alt,
        );
        p.rect_stroke(
            rect,
            egui::CornerRadius::same(10),
            egui::Stroke::new(1.0, theme::tokens(ui).border),
            egui::StrokeKind::Inside,
        );
        p.galley(
            egui::pos2(rect.left() + 8.0, rect.center().y - galley.size().y / 2.0),
            galley,
            theme::tokens(ui).muted,
        );
    }
}
