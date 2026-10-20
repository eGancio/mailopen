//! Message list: title with icon actions, rows with avatar, sender, subject and preview.

use crate::app::{MailApp, MsgKey, View};
use crate::ui::icons::{self, Icon};
use crate::ui::{format, theme};

const ROW_H: f32 = 72.0;

enum Action {
    Select(MsgKey),
    Delete(MsgKey),
    Compose,
    Refresh,
}

impl MailApp {
    pub fn message_list(&mut self, root: &mut egui::Ui) {
        let mut action: Option<Action> = None;
        let messages = self.visible_messages();
        let unified = self.view == View::Unified;
        let mut search_open = self.search_open;
        let mut search = self.search.clone();
        egui::Panel::left("list")
            .resizable(true)
            .default_size(330.0)
            .size_range(260.0..=560.0)
            .frame(egui::Frame::NONE.fill(theme::tokens(root).surface))
            .show(root, |ui| {
                let title = match &self.view {
                    View::Unified => "Posta in arrivo".to_string(),
                    View::Folder { account, folder } => self
                        .folders
                        .get(account)
                        .and_then(|fs| fs.iter().find(|f| &f.name == folder))
                        .map(|f| f.display())
                        .unwrap_or_else(|| folder.clone()),
                };
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    ui.label(theme::bold(title, 18.0));
                    ui.add_space(2.0);
                    if icons::button(
                        ui,
                        Icon::Compose,
                        15.0,
                        theme::tokens(ui).muted,
                        "Nuovo messaggio (⌘N)",
                    )
                    .clicked()
                    {
                        action = Some(Action::Compose);
                    }
                    if icons::button(ui, Icon::Search, 15.0, theme::tokens(ui).muted, "Cerca")
                        .clicked()
                    {
                        search_open = !search_open;
                        if !search_open {
                            search.clear();
                        }
                    }
                    if icons::button(ui, Icon::Refresh, 15.0, theme::tokens(ui).muted, "Aggiorna")
                        .clicked()
                    {
                        action = Some(Action::Refresh);
                    }
                    if self.is_busy() {
                        ui.spinner();
                    }
                });
                if search_open {
                    ui.horizontal(|ui| {
                        ui.add_space(14.0);
                        let r = ui.add(
                            egui::TextEdit::singleline(&mut search)
                                .hint_text("Cerca per mittente o oggetto")
                                .desired_width(ui.available_width() - 20.0),
                        );
                        if self.search_focus {
                            r.request_focus();
                        }
                    });
                }
                ui.add_space(6.0);
                theme::hline(ui);
                if messages.is_empty() {
                    ui.add_space(40.0);
                    ui.vertical_centered(|ui| {
                        let text = if self.is_busy() {
                            "Caricamento…"
                        } else if self.config.accounts.is_empty() {
                            "Aggiungi un account per cominciare."
                        } else {
                            "Nessun messaggio."
                        };
                        ui.label(
                            egui::RichText::new(text)
                                .color(theme::tokens(ui).muted)
                                .size(13.0),
                        );
                    });
                    return;
                }
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show_rows(ui, ROW_H, messages.len(), |ui, range| {
                        for m in messages
                            .iter()
                            .skip(range.start)
                            .take(range.end.saturating_sub(range.start))
                        {
                            let key = m.key();
                            let selected = self.selected.as_ref() == Some(&key);
                            let stripe = if unified {
                                self.config
                                    .account(&m.account_id)
                                    .map(|a| theme::account_color(a.color))
                            } else {
                                None
                            };
                            let logo = self.avatars.get(&m.from_addr);
                            let resp = row(ui, m, selected, stripe, logo);
                            if resp.clicked() {
                                action = Some(Action::Select(key.clone()));
                            }
                            resp.context_menu(|ui| {
                                if ui.button("Elimina").clicked() {
                                    action = Some(Action::Delete(key.clone()));
                                    ui.close();
                                }
                            });
                        }
                    });
            });
        self.search_focus = search_open && !self.search_open;
        self.search_open = search_open;
        self.search = search;
        match action {
            Some(Action::Select(k)) => self.select_message(k),
            Some(Action::Delete(k)) => self.delete_message(k),
            Some(Action::Compose) => self.compose_new(),
            Some(Action::Refresh) => self.refresh_view(),
            None => {}
        }
    }
}

fn row(
    ui: &mut egui::Ui,
    m: &mailopen_core::MessageSummary,
    selected: bool,
    stripe: Option<egui::Color32>,
    logo: Option<egui::load::SizedTexture>,
) -> egui::Response {
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, ROW_H), egui::Sense::click());
    if !ui.is_rect_visible(rect) {
        return resp;
    }
    let p = ui.painter();
    let (text, muted) = if selected {
        p.rect_filled(rect, egui::CornerRadius::ZERO, theme::tokens(ui).accent);
        (egui::Color32::WHITE, egui::Color32::from_white_alpha(200))
    } else {
        if resp.hovered() {
            p.rect_filled(rect, egui::CornerRadius::ZERO, theme::tokens(ui).hover);
        }
        p.line_segment(
            [
                egui::pos2(rect.left() + 56.0, rect.bottom()),
                egui::pos2(rect.right(), rect.bottom()),
            ],
            egui::Stroke::new(1.0, theme::tokens(ui).border),
        );
        (theme::tokens(ui).text, theme::tokens(ui).muted)
    };
    if let Some(c) = stripe {
        p.rect_filled(
            egui::Rect::from_min_size(
                egui::pos2(rect.left(), rect.top() + 8.0),
                egui::vec2(3.0, ROW_H - 16.0),
            ),
            egui::CornerRadius::same(2),
            c,
        );
    }
    if !m.seen {
        p.circle_filled(
            egui::pos2(rect.left() + 12.0, rect.top() + 22.0),
            3.5,
            if selected {
                egui::Color32::WHITE
            } else {
                theme::tokens(ui).accent
            },
        );
    }
    let avatar_rect = egui::Rect::from_center_size(
        egui::pos2(rect.left() + 34.0, rect.top() + 26.0),
        egui::vec2(26.0, 26.0),
    );
    theme::draw_avatar(
        ui,
        avatar_rect,
        &theme::initial(m.sender()),
        theme::hash_color(&m.from_addr.to_lowercase()),
        logo,
    );
    let p = ui.painter();
    let left = rect.left() + 56.0;
    let date = format::short_date(m.timestamp);
    let date_galley = p.layout_no_wrap(date, egui::FontId::proportional(11.5), muted);
    let date_w = date_galley.size().x;
    p.galley(
        egui::pos2(rect.right() - 12.0 - date_w, rect.top() + 10.0),
        date_galley,
        muted,
    );
    let sender_font = if m.seen {
        egui::FontId::proportional(13.0)
    } else {
        theme::bold_font(13.0)
    };
    theme::clipped_text(
        p,
        egui::pos2(left, rect.top() + 9.0),
        rect.right() - 20.0 - date_w - left,
        m.sender(),
        sender_font,
        text,
    );
    let subject = if m.subject.is_empty() {
        "(senza oggetto)"
    } else {
        m.subject.as_str()
    };
    let subject_font = if m.seen {
        egui::FontId::proportional(13.0)
    } else {
        theme::bold_font(13.0)
    };
    let right_pad = if m.has_attachment { 34.0 } else { 12.0 };
    theme::clipped_text(
        p,
        egui::pos2(left, rect.top() + 28.0),
        rect.right() - right_pad - left,
        subject,
        subject_font,
        text,
    );
    if m.has_attachment {
        icons::at(
            p,
            egui::pos2(rect.right() - 20.0, rect.top() + 36.0),
            13.0,
            Icon::Attachment,
            muted,
        );
    }
    theme::clipped_text(
        p,
        egui::pos2(left, rect.top() + 47.0),
        rect.right() - 12.0 - left,
        &m.snippet,
        egui::FontId::proportional(12.0),
        muted,
    );
    resp
}
