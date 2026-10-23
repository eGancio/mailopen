//! Reading pane: icon toolbar, subject, sender line, attachments, body.
//! HTML mail is drawn natively on white paper (it is designed for it, also in dark mode);
//! remote images stay blocked until the user allows them.

use std::sync::Arc;

use mailopen_core::Attachment;

use crate::app::{MailApp, View};
use crate::ui::html_view;
use crate::ui::icons::{self, Icon};
use crate::ui::organize::{self, Organize};
use crate::ui::viewer::file_badge;
use crate::ui::{files, format, theme};

enum Action {
    Reply,
    ReplyAll,
    Forward,
    Delete,
    ToggleSeen,
    Retry,
    Print,
    ShowImages,
    TrustSender,
    OpenAttachment(usize),
    SaveAttachment(usize),
    OpenWithApp(usize),
    PrintAttachment(usize),
    ForwardAttachment(usize),
    SaveAll,
    Link(String),
    Organize(Organize),
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
                        let hint = if self.view == View::Documents {
                            "Seleziona un documento: clic per il messaggio, doppio clic per l'anteprima"
                        } else {
                            "Seleziona un messaggio"
                        };
                        ui.label(
                            egui::RichText::new(hint)
                                .color(theme::tokens(ui).muted)
                                .size(14.0),
                        );
                    });
                    return;
                };
                let key = summary.key();
                ui.horizontal(|ui| {
                    let muted = theme::tokens(ui).muted;
                    let live = summary.uid != 0;
                    let (seen_tip, seen_color) = if summary.seen {
                        ("Segna come non letto (Ctrl+U)", muted)
                    } else {
                        ("Segna come letto (Ctrl+U)", theme::tokens(ui).accent)
                    };
                    if live
                        && icons::button(ui, Icon::Envelope, 16.0, seen_color, seen_tip).clicked()
                    {
                        action = Some(Action::ToggleSeen);
                    }
                    let (flag_tip, flag_color) = if summary.flagged {
                        ("Rimuovi contrassegno", organize::FLAG)
                    } else {
                        ("Contrassegna", muted)
                    };
                    if live && icons::button(ui, Icon::Flag, 16.0, flag_color, flag_tip).clicked()
                    {
                        action = Some(Action::Organize(Organize::ToggleFlag));
                    }
                    let tag = icons::button(ui, Icon::Tag, 16.0, muted, "Categoria");
                    egui::Popup::menu(&tag)
                        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                        .show(|ui| {
                            ui.set_min_width(200.0);
                            if let Some(a) = organize::category_items(ui, &self.labels, &summary) {
                                action = Some(Action::Organize(a));
                            }
                        });
                    let targets = self.move_targets(&summary.account_id, &summary.folder);
                    if live && !targets.is_empty() {
                        let mv = icons::button(ui, Icon::MoveTo, 16.0, muted, "Sposta in…");
                        egui::Popup::menu(&mv).show(|ui| {
                            ui.set_min_width(200.0);
                            if let Some(to) = organize::move_items(ui, &targets) {
                                action = Some(Action::Organize(Organize::Move(to)));
                            }
                        });
                    }
                    if live && icons::button(ui, Icon::Trash, 16.0, muted, "Elimina (Canc)").clicked() {
                        action = Some(Action::Delete);
                    }
                    ui.add_space(10.0);
                    if icons::button(ui, Icon::Reply, 16.0, muted, "Rispondi (Ctrl+R)").clicked() {
                        action = Some(Action::Reply);
                    }
                    if icons::button(ui, Icon::ReplyAll, 16.0, muted, "Rispondi a tutti").clicked()
                    {
                        action = Some(Action::ReplyAll);
                    }
                    if icons::button(ui, Icon::Forward, 16.0, muted, "Inoltra").clicked() {
                        action = Some(Action::Forward);
                    }
                    if icons::button(ui, Icon::Print, 16.0, muted, "Stampa il messaggio").clicked()
                    {
                        action = Some(Action::Print);
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
                let cats = self.labels.of(&summary);
                if !cats.is_empty() {
                    ui.add_space(6.0);
                    organize::chips(ui, &cats);
                }
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
                    ui.add_space(10.0);
                    if let Some(a) = attachments_row(ui, &b.attachments) {
                        action = Some(a);
                    }
                }
                let sender = summary.from_addr.to_lowercase();
                let allow_remote = self.remote_allowed.contains(&key)
                    || self.config.trusted_image_senders.contains(&sender);
                if let Some(b) = body
                    && b.doc.remote_images > 0
                    && !allow_remote
                {
                    ui.add_space(8.0);
                    if let Some(a) = remote_banner(ui, b.doc.remote_images) {
                        action = Some(a);
                    }
                }
                ui.add_space(10.0);
                match (body, self.body_errors.get(&key)) {
                    (Some(b), _) => {
                        let doc = Arc::clone(&b.doc);
                        let paper = b.is_html;
                        let palette = if paper {
                            html_view::Palette::paper()
                        } else {
                            html_view::Palette::themed(ui)
                        };
                        let fill = if paper {
                            doc.background
                                .map(html_view::color)
                                .unwrap_or(egui::Color32::WHITE)
                        } else {
                            theme::tokens(ui).surface_alt
                        };
                        let cache_key = format!("{}/{}/{}", key.0, key.1, key.2);
                        let mut clicked = None;
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .id_salt(("reader", &cache_key))
                            .show(ui, |ui| {
                                egui::Frame::NONE
                                    .fill(fill)
                                    .stroke(if paper {
                                        egui::Stroke::new(1.0, theme::tokens(ui).border)
                                    } else {
                                        egui::Stroke::NONE
                                    })
                                    .corner_radius(egui::CornerRadius::same(8))
                                    .inner_margin(egui::Margin::same(if paper { 14 } else { 16 }))
                                    .show(ui, |ui| {
                                        ui.set_min_width(ui.available_width());
                                        let mut view = html_view::View::new(
                                            &mut self.images,
                                            b,
                                            cache_key.clone(),
                                            allow_remote,
                                            palette,
                                        );
                                        html_view::show(ui, &doc, &mut view);
                                        clicked = view.clicked;
                                    });
                                ui.add_space(24.0);
                            });
                        if let Some(url) = clicked {
                            action = Some(Action::Link(url));
                        }
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
        let Some(action) = action else { return };
        let ctx = root.ctx().clone();
        let key = self.selected.clone();
        let attachment = |app: &MailApp, i: usize| -> Option<Attachment> {
            let k = key.as_ref()?;
            app.bodies.get(k)?.attachments.get(i).cloned()
        };
        match action {
            Action::Reply => self.reply(false),
            Action::ReplyAll => self.reply(true),
            Action::Forward => self.forward(),
            Action::Delete => self.delete_selected(),
            Action::ToggleSeen => self.toggle_seen_selected(),
            Action::Print => self.print_selected(),
            Action::ShowImages => {
                if let Some(k) = key {
                    self.remote_allowed.insert(k);
                }
            }
            Action::TrustSender => {
                if let Some(s) = self.selected_summary() {
                    let addr = s.from_addr.to_lowercase();
                    if !addr.is_empty() && !self.config.trusted_image_senders.contains(&addr) {
                        self.config.trusted_image_senders.push(addr);
                        if let Err(e) = self.config.save() {
                            self.set_status(format!("Configurazione non salvata: {e}"), true);
                        }
                    }
                }
            }
            Action::OpenAttachment(i) => {
                if let Some(a) = attachment(self, i) {
                    self.open_attachment(key.clone(), a);
                }
            }
            Action::SaveAttachment(i) => {
                if let Some(a) = attachment(self, i) {
                    match files::save_dialog(&a) {
                        Ok(Some(p)) => {
                            self.set_status(format!("Salvato in {}", p.display()), false)
                        }
                        Ok(None) => {}
                        Err(e) => self.set_status(e, true),
                    }
                }
            }
            Action::OpenWithApp(i) => {
                if let Some(a) = attachment(self, i)
                    && let Err(e) = files::open_with_default(&a)
                {
                    self.set_status(e, true);
                }
            }
            Action::PrintAttachment(i) => {
                if let Some(a) = attachment(self, i) {
                    self.set_status(format!("Preparo la stampa di {}…", a.name), false);
                    self.background_task = Some(files::print(a));
                }
            }
            Action::ForwardAttachment(i) => {
                if let Some(a) = attachment(self, i) {
                    self.forward_with(key.clone(), vec![a]);
                }
            }
            Action::SaveAll => {
                let list = key
                    .as_ref()
                    .and_then(|k| self.bodies.get(k))
                    .map(|b| b.attachments.clone())
                    .unwrap_or_default();
                match files::save_all_dialog(&list) {
                    Ok(Some((dir, n))) => {
                        self.set_status(format!("{n} allegati salvati in {}", dir.display()), false)
                    }
                    Ok(None) => {}
                    Err(e) => self.set_status(e, true),
                }
            }
            Action::Link(url) => self.open_link(&ctx, &url),
            Action::Organize(a) => {
                if let Some(k) = key {
                    self.organize(k, a);
                }
            }
            Action::Retry => {
                if let Some((acc, folder, uid)) = self.selected.clone() {
                    self.body_errors.remove(&(acc.clone(), folder.clone(), uid));
                    self.request(&acc, mailopen_core::Request::LoadBody { folder, uid });
                }
            }
        }
    }
}

/// Attachment cards: badge, name, size. Click opens the viewer; right click for more.
fn attachments_row(ui: &mut egui::Ui, list: &[Attachment]) -> Option<Action> {
    let t = theme::tokens(ui);
    let mut action = None;
    ui.horizontal(|ui| {
        icons::small(ui, Icon::Attachment, 14.0, t.muted);
        let total: usize = list.iter().map(|a| a.size()).sum();
        ui.label(
            egui::RichText::new(if list.len() == 1 {
                format!(
                    "1 allegato · {}",
                    format::size(u32::try_from(total).unwrap_or(u32::MAX))
                )
            } else {
                format!(
                    "{} allegati · {}",
                    list.len(),
                    format::size(u32::try_from(total).unwrap_or(u32::MAX))
                )
            })
            .color(t.muted)
            .size(12.5),
        );
        if list.len() > 1
            && ui
                .add(
                    egui::Label::new(
                        egui::RichText::new("Salva tutti")
                            .color(t.accent)
                            .size(12.5),
                    )
                    .sense(egui::Sense::click()),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
        {
            action = Some(Action::SaveAll);
        }
    });
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
        for (i, a) in list.iter().enumerate() {
            let card_w = 236.0_f32.min(ui.available_width().max(120.0));
            let (rect, resp) =
                ui.allocate_exact_size(egui::vec2(card_w, 50.0), egui::Sense::click());
            if ui.is_rect_visible(rect) {
                let p = ui.painter();
                let fill = if resp.hovered() { t.hover } else { t.surface };
                p.rect_filled(rect, egui::CornerRadius::same(8), fill);
                p.rect_stroke(
                    rect,
                    egui::CornerRadius::same(8),
                    egui::Stroke::new(1.0, if resp.hovered() { t.accent } else { t.border }),
                    egui::StrokeKind::Inside,
                );
                let mut badge_ui =
                    ui.new_child(egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
                        rect.min + egui::vec2(10.0, 8.0),
                        egui::vec2(30.0, 34.0),
                    )));
                file_badge(&mut badge_ui, a, 34.0);
                let text_left = rect.left() + 50.0;
                let width = rect.right() - text_left - 10.0;
                theme::clipped_text(
                    ui.painter(),
                    egui::pos2(text_left, rect.top() + 9.0),
                    width,
                    &a.name,
                    theme::bold_font(12.5),
                    t.text,
                );
                theme::clipped_text(
                    ui.painter(),
                    egui::pos2(text_left, rect.top() + 28.0),
                    width,
                    &format::size(u32::try_from(a.size()).unwrap_or(u32::MAX)),
                    egui::FontId::proportional(11.5),
                    t.muted,
                );
            }
            let resp = resp
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(format!(
                    "{}\nClic: anteprima · tasto destro: altre azioni",
                    a.name
                ));
            if resp.clicked() {
                action = Some(Action::OpenAttachment(i));
            }
            resp.context_menu(|ui| {
                if ui.button("Anteprima").clicked() {
                    action = Some(Action::OpenAttachment(i));
                    ui.close();
                }
                if ui.button("Salva…").clicked() {
                    action = Some(Action::SaveAttachment(i));
                    ui.close();
                }
                if ui.button("Apri con l'app predefinita").clicked() {
                    action = Some(Action::OpenWithApp(i));
                    ui.close();
                }
                if ui.button("Stampa").clicked() {
                    action = Some(Action::PrintAttachment(i));
                    ui.close();
                }
                if ui.button("Inoltra").clicked() {
                    action = Some(Action::ForwardAttachment(i));
                    ui.close();
                }
            });
        }
    });
    action
}

fn remote_banner(ui: &mut egui::Ui, count: usize) -> Option<Action> {
    let t = theme::tokens(ui);
    let mut action = None;
    egui::Frame::NONE
        .fill(t.surface_alt)
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                let text = if count == 1 {
                    "1 immagine remota bloccata: scaricarla direbbe al mittente che hai aperto il messaggio.".to_string()
                } else {
                    format!("{count} immagini remote bloccate: scaricarle direbbe al mittente che hai aperto il messaggio.")
                };
                ui.label(egui::RichText::new(text).color(t.muted).size(12.5));
                if ui
                    .add(egui::Label::new(theme::bold("Mostra immagini", 12.5).color(t.accent)).sense(egui::Sense::click()))
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    action = Some(Action::ShowImages);
                }
                ui.label(egui::RichText::new("·").color(t.muted));
                if ui
                    .add(
                        egui::Label::new(egui::RichText::new("Sempre da questo mittente").color(t.accent).size(12.5))
                            .sense(egui::Sense::click()),
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    action = Some(Action::TrustSender);
                }
            });
        });
    action
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
