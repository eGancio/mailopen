//! The dark account rail on the far left: "all mail", one avatar per account, add.

use crate::app::{MailApp, Rail};
use crate::ui::icons::{self, Icon};
use crate::ui::theme;

enum Action {
    Select(Rail),
    Add,
    Settings,
    ToggleTheme(bool),
    Edit(String),
    Remove(String),
}

const W: f32 = 56.0;
const TILE: f32 = 38.0;

impl MailApp {
    pub fn rail(&mut self, root: &mut egui::Ui) {
        let mut action: Option<Action> = None;
        egui::Panel::left("rail")
            .resizable(false)
            .exact_size(W)
            .frame(egui::Frame::NONE.fill(theme::tokens(root).rail_bg))
            .show(root, |ui| {
                ui.add_space(10.0);
                let total: usize = self
                    .config
                    .accounts
                    .iter()
                    .map(|a| self.unread_count(&a.id, &self.inbox_name(&a.id)))
                    .sum();
                if tile(
                    ui,
                    None,
                    "",
                    total,
                    self.rail == Rail::All,
                    "Tutta la posta",
                    None,
                )
                .clicked()
                {
                    action = Some(Action::Select(Rail::All));
                }
                ui.add_space(6.0);
                let accounts = self.config.accounts.clone();
                for account in &accounts {
                    let logo = self.avatars.get(&account.email);
                    let unread = self.unread_count(&account.id, &self.inbox_name(&account.id));
                    let selected = self.rail == Rail::Account(account.id.clone());
                    let resp = tile(
                        ui,
                        Some(theme::account_color(account.color)),
                        &account.initial(),
                        unread,
                        selected,
                        &format!("{}\n{}", account.name, account.email),
                        logo,
                    );
                    if resp.clicked() {
                        action = Some(Action::Select(Rail::Account(account.id.clone())));
                    }
                    resp.context_menu(|ui| {
                        if ui.button("Modifica…").clicked() {
                            action = Some(Action::Edit(account.id.clone()));
                            ui.close();
                        }
                        if ui.button("Rimuovi account").clicked() {
                            action = Some(Action::Remove(account.id.clone()));
                            ui.close();
                        }
                    });
                }
                ui.add_space(4.0);
                let (rect, resp) =
                    ui.allocate_exact_size(egui::vec2(W, TILE), egui::Sense::click());
                if ui.is_rect_visible(rect) {
                    let c = if resp.hovered() {
                        egui::Color32::WHITE
                    } else {
                        egui::Color32::from_white_alpha(170)
                    };
                    icons::at(ui.painter(), rect.center(), 16.0, Icon::Plus, c);
                }
                if resp.on_hover_text("Aggiungi account").clicked() {
                    action = Some(Action::Add);
                }
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                    ui.add_space(10.0);
                    let (rect, resp) =
                        ui.allocate_exact_size(egui::vec2(W, TILE), egui::Sense::click());
                    if ui.is_rect_visible(rect) {
                        let c = if resp.hovered() {
                            egui::Color32::WHITE
                        } else {
                            egui::Color32::from_white_alpha(170)
                        };
                        icons::at(ui.painter(), rect.center(), 17.0, Icon::Gear, c);
                    }
                    if resp.on_hover_text("Impostazioni").clicked() {
                        action = Some(Action::Settings);
                    }
                    let dark = ui.visuals().dark_mode;
                    let (rect, resp) =
                        ui.allocate_exact_size(egui::vec2(W, TILE), egui::Sense::click());
                    if ui.is_rect_visible(rect) {
                        let c = if resp.hovered() {
                            egui::Color32::WHITE
                        } else {
                            egui::Color32::from_white_alpha(170)
                        };
                        let icon = if dark { Icon::Sun } else { Icon::Moon };
                        icons::at(ui.painter(), rect.center(), 17.0, icon, c);
                    }
                    let tip = if dark { "Tema chiaro" } else { "Tema scuro" };
                    if resp.on_hover_text(tip).clicked() {
                        action = Some(Action::ToggleTheme(dark));
                    }
                });
            });
        match action {
            Some(Action::Select(r)) => self.select_rail(r),
            Some(Action::Add) => {
                self.account_dialog = Some(crate::ui::account_dialog::AccountDialog::new(
                    self.config.next_color(),
                ));
            }
            Some(Action::Settings) => self.settings_open = true,
            Some(Action::ToggleTheme(dark)) => self.toggle_theme(root.ctx(), dark),
            Some(Action::Edit(id)) => self.edit_account(&id),
            Some(Action::Remove(id)) => self.remove_account(&id),
            None => {}
        }
    }
}

fn tile(
    ui: &mut egui::Ui,
    color: Option<egui::Color32>,
    letter: &str,
    unread: usize,
    selected: bool,
    tooltip: &str,
    logo: Option<egui::load::SizedTexture>,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(W, TILE + 10.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        if selected {
            p.rect_filled(
                rect.shrink2(egui::vec2(6.0, 3.0)),
                egui::CornerRadius::same(8),
                theme::tokens(ui).rail_selected,
            );
            p.rect_filled(
                egui::Rect::from_min_size(
                    egui::pos2(rect.left(), rect.top() + 9.0),
                    egui::vec2(3.0, TILE - 8.0),
                ),
                egui::CornerRadius::same(2),
                egui::Color32::WHITE,
            );
        } else if resp.hovered() {
            p.rect_filled(
                rect.shrink2(egui::vec2(6.0, 3.0)),
                egui::CornerRadius::same(8),
                theme::tokens(ui).rail_selected.gamma_multiply(0.6),
            );
        }
        let tile = egui::Rect::from_center_size(rect.center(), egui::vec2(TILE - 8.0, TILE - 8.0));
        match color {
            Some(_) if logo.is_some() => {}
            Some(c) => {
                p.rect_filled(tile, egui::CornerRadius::same(7), c);
                p.text(
                    tile.center(),
                    egui::Align2::CENTER_CENTER,
                    letter,
                    theme::bold_font(15.0),
                    egui::Color32::WHITE,
                );
            }
            None => {
                p.rect_filled(
                    tile,
                    egui::CornerRadius::same(7),
                    egui::Color32::from_white_alpha(28),
                );
                icons::at(p, tile.center(), 18.0, Icon::Mails, egui::Color32::WHITE);
            }
        }
        if let Some(tex) = logo {
            theme::paint_texture(ui.painter(), tile, tex.id, 7.0);
        }
        let p = ui.painter();
        if unread > 0 {
            theme::badge(
                p,
                egui::pos2(tile.right() + 8.0, tile.top() + 2.0),
                unread,
                theme::tokens(ui).accent,
                egui::Color32::WHITE,
            );
        }
    }
    resp.on_hover_text(tooltip)
}
