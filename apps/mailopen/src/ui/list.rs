//! Message list: title with icon actions, quick filters, rows with avatar, sender, subject,
//! flag, categories and preview. In the Documenti view the same column lists files instead.

use mailopen_core::MessageSummary;

use crate::app::{ListFilter, MailApp, MsgKey, View};
use crate::ui::icons::{self, Icon};
use crate::ui::organize::{self, Organize};
use crate::ui::{format, theme};

const ROW_H: f32 = 72.0;

enum Action {
    Select(Box<MessageSummary>),
    Organize(MsgKey, Organize),
    Compose,
    Refresh,
    ServerSearch,
    Filter(ListFilter),
}

impl MailApp {
    pub fn message_list(&mut self, root: &mut egui::Ui) {
        if self.view == View::Documents {
            self.documents_list(root);
            return;
        }
        let mut action: Option<Action> = None;
        let messages = self.visible_messages();
        let multi_account = !matches!(self.view, View::Folder { .. });
        let mut search_open = self.search_open;
        let mut search = self.search.clone();
        egui::Panel::left("list")
            .resizable(true)
            .default_size(340.0)
            .size_range(260.0..=560.0)
            .frame(egui::Frame::NONE.fill(theme::tokens(root).surface))
            .show(root, |ui| {
                let title = match &self.view {
                    View::Unified => "Posta in arrivo".to_string(),
                    View::Folder { account, folder } => self.folder_label(account, folder),
                    View::Category(id) => self
                        .labels
                        .category(id)
                        .map(|c| c.name.clone())
                        .unwrap_or_else(|| "Categoria".into()),
                    View::Search(q) => format!("Risultati: «{q}»"),
                    View::Documents => "Documenti".into(),
                };
                let title = if title.chars().count() > 26 {
                    format!("{}…", title.chars().take(25).collect::<String>())
                } else {
                    title
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
                        "Nuovo messaggio (Ctrl+N)",
                    )
                    .clicked()
                    {
                        action = Some(Action::Compose);
                    }
                    if icons::button(
                        ui,
                        Icon::Search,
                        15.0,
                        theme::tokens(ui).muted,
                        "Cerca (Ctrl+F)",
                    )
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
                                .hint_text("Mittente, oggetto, anteprima, categoria")
                                .desired_width(ui.available_width() - 20.0),
                        );
                        if self.search_focus {
                            r.request_focus();
                        }
                        if r.lost_focus()
                            && ui.input(|i| i.key_pressed(egui::Key::Enter))
                            && !search.trim().is_empty()
                        {
                            action = Some(Action::ServerSearch);
                        }
                    });
                    if !search.trim().is_empty() {
                        ui.horizontal(|ui| {
                            ui.add_space(14.0);
                            let text = format!("Cerca «{}» nel testo, sul server ↵", search.trim());
                            if link(ui, &text)
                                .on_hover_text(
                                    "Cerca in tutte le cartelle, anche nei messaggi vecchi e \
                                     nel testo dei messaggi (Invio)",
                                )
                                .clicked()
                            {
                                action = Some(Action::ServerSearch);
                            }
                        });
                    }
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.spacing_mut().item_spacing.x = 4.0;
                    for f in ListFilter::ALL {
                        if pill(ui, f.label(), self.filter == f).clicked() {
                            action = Some(Action::Filter(f));
                        }
                    }
                });
                ui.add_space(6.0);
                theme::hline(ui);
                if messages.is_empty() {
                    ui.add_space(40.0);
                    ui.vertical_centered(|ui| {
                        let searching = self
                            .server_search
                            .as_ref()
                            .is_some_and(|s| s.pending > 0)
                            && matches!(self.view, View::Search(_));
                        let text = if searching || (self.is_busy() && self.filter == ListFilter::All)
                        {
                            "Caricamento…"
                        } else if self.config.accounts.is_empty() {
                            "Aggiungi un account per cominciare."
                        } else if matches!(self.view, View::Category(_)) {
                            "Nessun messaggio in questa categoria.\nTasto destro su un messaggio › Categoria."
                        } else if self.filter != ListFilter::All {
                            "Nessun messaggio con questo filtro."
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
                            let stripe = if multi_account {
                                self.config
                                    .account(&m.account_id)
                                    .map(|a| theme::account_color(a.color))
                            } else {
                                None
                            };
                            let cats: Vec<(String, egui::Color32)> = self
                                .labels
                                .of(m)
                                .iter()
                                .map(|c| (c.name.clone(), organize::category_color(c)))
                                .collect();
                            let logo = self.avatars.get(&m.from_addr);
                            let resp = row(ui, m, selected, stripe, logo, &cats);
                            if resp.clicked() {
                                action = Some(Action::Select(Box::new(m.clone())));
                            }
                            resp.context_menu(|ui| {
                                if let Some(a) = organize::message_menu(ui, self, m) {
                                    action = Some(Action::Organize(key.clone(), a));
                                }
                            });
                        }
                    });
            });
        self.search_focus = search_open && !self.search_open;
        self.search_open = search_open;
        if search != self.search {
            self.search = search;
        }
        match action {
            Some(Action::Select(m)) => self.open_summary(*m),
            Some(Action::Organize(k, a)) => self.organize(k, a),
            Some(Action::Compose) => self.compose_new(),
            Some(Action::Refresh) => self.refresh_view(),
            Some(Action::ServerSearch) => self.start_server_search(),
            Some(Action::Filter(f)) => self.filter = f,
            None => {}
        }
    }
}

/// A clickable line of accent text.
pub fn link(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(
        egui::Label::new(
            egui::RichText::new(text)
                .color(theme::tokens(ui).accent)
                .size(12.5),
        )
        .sense(egui::Sense::click()),
    )
    .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A small rounded toggle (quick filters, document types).
pub fn pill(ui: &mut egui::Ui, text: &str, on: bool) -> egui::Response {
    let t = theme::tokens(ui);
    let font = if on {
        theme::bold_font(11.5)
    } else {
        egui::FontId::proportional(11.5)
    };
    let color = if on { t.accent } else { t.muted };
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, color);
    let size = egui::vec2(galley.size().x + 16.0, 22.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let fill = if on {
            t.accent_soft
        } else if resp.hovered() {
            t.hover
        } else {
            t.surface
        };
        p.rect_filled(rect, egui::CornerRadius::same(11), fill);
        if !on {
            p.rect_stroke(
                rect,
                egui::CornerRadius::same(11),
                egui::Stroke::new(1.0, t.border),
                egui::StrokeKind::Inside,
            );
        }
        p.galley(
            egui::pos2(rect.left() + 8.0, rect.center().y - galley.size().y / 2.0),
            galley,
            color,
        );
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn row(
    ui: &mut egui::Ui,
    m: &MessageSummary,
    selected: bool,
    stripe: Option<egui::Color32>,
    logo: Option<egui::load::SizedTexture>,
    cats: &[(String, egui::Color32)],
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
    let mut right_pad = 12.0;
    let mut mark_x = rect.right() - 20.0;
    if m.has_attachment {
        icons::at(
            p,
            egui::pos2(mark_x, rect.top() + 36.0),
            13.0,
            Icon::Attachment,
            muted,
        );
        mark_x -= 18.0;
        right_pad += 18.0;
    }
    if m.flagged {
        icons::at(
            p,
            egui::pos2(mark_x, rect.top() + 36.0),
            12.0,
            Icon::Flag,
            if selected {
                egui::Color32::WHITE
            } else {
                organize::FLAG
            },
        );
        right_pad += 18.0;
    }
    theme::clipped_text(
        p,
        egui::pos2(left, rect.top() + 28.0),
        rect.right() - right_pad - left,
        subject,
        subject_font,
        text,
    );
    let mut x = left;
    let line_y = rect.top() + 55.0;
    for (name, color) in cats {
        let w = organize::paint_chip(
            p,
            x,
            line_y,
            name,
            *color,
            selected,
            (rect.right() - 60.0 - x).max(0.0),
        );
        if w <= 0.0 {
            break;
        }
        x += w + 4.0;
    }
    theme::clipped_text(
        p,
        egui::pos2(
            x + if cats.is_empty() { 0.0 } else { 2.0 },
            rect.top() + 47.0,
        ),
        rect.right() - 12.0 - x,
        &m.snippet,
        egui::FontId::proportional(12.0),
        muted,
    );
    resp
}
