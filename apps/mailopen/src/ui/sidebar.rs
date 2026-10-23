//! Folder column: standard folders with icons, then "Le tue cartelle".

use mailopen_core::FolderKind;

use crate::app::{MailApp, Rail, View};
use crate::ui::icons::{self, Icon};
use crate::ui::theme;

enum Action {
    View(View),
    Rail(Rail),
    ManageCategories,
}

const W: f32 = 196.0;
const ROW: f32 = 30.0;

fn kind_icon(kind: FolderKind) -> Icon {
    match kind {
        FolderKind::Inbox => Icon::Inbox,
        FolderKind::Drafts => Icon::Draft,
        FolderKind::Sent => Icon::Sent,
        FolderKind::Archive => Icon::Archive,
        FolderKind::Spam => Icon::Spam,
        FolderKind::Trash => Icon::Trash,
        FolderKind::Other => Icon::Folder,
    }
}

impl MailApp {
    pub fn sidebar(&mut self, root: &mut egui::Ui) {
        let mut action: Option<Action> = None;
        egui::Panel::left("sidebar")
            .resizable(false)
            .exact_size(W)
            .frame(egui::Frame::NONE.fill(theme::tokens(root).surface))
            .show(root, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add_space(14.0);
                        match self.rail.clone() {
                            Rail::All => self.sidebar_all(ui, &mut action),
                            Rail::Account(id) => self.sidebar_account(ui, &id, &mut action),
                        }
                        ui.add_space(12.0);
                    });
            });
        match action {
            Some(Action::View(v)) => self.set_view(v),
            Some(Action::Rail(r)) => self.select_rail(r),
            Some(Action::ManageCategories) => {
                self.settings_open = true;
                self.settings_categories = true;
            }
            None => {}
        }
    }

    /// "Documenti" and the categories, for the accounts in scope.
    fn sidebar_organize(&self, ui: &mut egui::Ui, action: &mut Option<Action>) {
        ui.add_space(14.0);
        header(ui, "ORGANIZZA");
        let docs: usize = self
            .documents
            .iter()
            .filter(|(acc, _)| self.in_scope(acc) && self.config.account(acc).is_some())
            .map(|(_, l)| l.len())
            .sum();
        let indexing = self.index_status.keys().any(|acc| self.in_scope(acc));
        let resp = row(
            ui,
            Icon::Document,
            theme::tokens(ui).text,
            "Documenti",
            0,
            self.view == View::Documents,
            false,
        );
        if ui.is_rect_visible(resp.rect) && docs > 0 {
            muted_count(ui, resp.rect, &docs.to_string());
        }
        let tip = if indexing {
            "Tutti i file allegati: cerca per tipo, nome o contenuto
(indicizzazione in corso)"
        } else {
            "Tutti i file allegati: cerca per tipo, nome o contenuto"
        };
        if resp.on_hover_text(tip).clicked() {
            *action = Some(Action::View(View::Documents));
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.add_space(14.0);
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(W - 56.0, 16.0), egui::Sense::hover());
            theme::clipped_text(
                ui.painter(),
                rect.left_top(),
                rect.width(),
                "CATEGORIE",
                theme::bold_font(10.0),
                theme::tokens(ui).muted,
            );
            if icons::button(
                ui,
                Icon::Gear,
                11.0,
                theme::tokens(ui).muted,
                "Gestisci categorie",
            )
            .clicked()
            {
                *action = Some(Action::ManageCategories);
            }
        });
        for c in &self.labels.categories {
            let selected = self.view == View::Category(c.id.clone());
            let n = self.category_messages(&c.id).len();
            let resp = dot_row(ui, theme::account_color(c.color), &c.name, selected);
            if n > 0 && ui.is_rect_visible(resp.rect) {
                muted_count(ui, resp.rect, &n.to_string());
            }
            if resp.clicked() {
                *action = Some(Action::View(View::Category(c.id.clone())));
            }
        }
    }

    fn sidebar_all(&self, ui: &mut egui::Ui, action: &mut Option<Action>) {
        header(ui, "TUTTI GLI ACCOUNT");
        let total: usize = self
            .config
            .accounts
            .iter()
            .map(|a| self.unread_count(&a.id, &self.inbox_name(&a.id)))
            .sum();
        if row(
            ui,
            Icon::Inbox,
            theme::tokens(ui).text,
            "Posta in arrivo",
            total,
            self.view == View::Unified,
            true,
        )
        .clicked()
        {
            *action = Some(Action::View(View::Unified));
        }
        self.sidebar_organize(ui, action);
        ui.add_space(14.0);
        header(ui, "ACCOUNT");
        for a in &self.config.accounts {
            let unread = self.unread_count(&a.id, &self.inbox_name(&a.id));
            let resp = account_row(
                ui,
                &a.initial(),
                theme::account_color(a.color),
                &a.name,
                unread,
            );
            if resp.clicked() {
                *action = Some(Action::Rail(Rail::Account(a.id.clone())));
            }
        }
        if self.config.accounts.is_empty() {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.add_space(14.0);
                ui.label(
                    egui::RichText::new("Nessun account")
                        .color(theme::tokens(ui).muted)
                        .size(12.5),
                );
            });
        }
    }

    fn sidebar_account(&self, ui: &mut egui::Ui, id: &str, action: &mut Option<Action>) {
        let Some(account) = self.config.account(id) else {
            return;
        };
        header(ui, &account.email.to_uppercase());
        if let Some(err) = self.account_errors.get(id) {
            ui.horizontal(|ui| {
                ui.add_space(14.0);
                ui.label(
                    egui::RichText::new("Password non disponibile")
                        .color(theme::ERROR)
                        .size(12.5),
                )
                .on_hover_text(err);
            });
            return;
        }
        let Some(folders) = self.folders.get(id) else {
            ui.horizontal(|ui| {
                ui.add_space(14.0);
                ui.spinner();
                ui.label(
                    egui::RichText::new("connessione…")
                        .color(theme::tokens(ui).muted)
                        .size(12.5),
                );
            });
            return;
        };
        let mut others = Vec::new();
        for f in folders.iter().filter(|f| f.selectable) {
            if f.kind == FolderKind::Other {
                others.push(f);
                continue;
            }
            let unread = if f.kind == FolderKind::Inbox {
                self.unread_count(id, &f.name)
            } else {
                0
            };
            let selected = matches!(&self.view, View::Folder { account, folder } if account == id && folder == &f.name);
            if row(
                ui,
                kind_icon(f.kind),
                theme::tokens(ui).text,
                &f.display(),
                unread,
                selected,
                f.kind == FolderKind::Inbox,
            )
            .clicked()
            {
                *action = Some(Action::View(View::Folder {
                    account: id.to_string(),
                    folder: f.name.clone(),
                }));
            }
        }
        if !others.is_empty() {
            ui.add_space(14.0);
            header(ui, "LE TUE CARTELLE");
            for f in others {
                let selected = matches!(&self.view, View::Folder { account, folder } if account == id && folder == &f.name);
                if dot_row(ui, theme::hash_color(&f.name), &f.display(), selected).clicked() {
                    *action = Some(Action::View(View::Folder {
                        account: id.to_string(),
                        folder: f.name.clone(),
                    }));
                }
            }
        }
        self.sidebar_organize(ui, action);
    }
}

fn header(ui: &mut egui::Ui, text: &str) {
    ui.horizontal(|ui| {
        ui.add_space(14.0);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(W - 28.0, 16.0), egui::Sense::hover());
        theme::clipped_text(
            ui.painter(),
            rect.left_top(),
            rect.width(),
            text,
            theme::bold_font(10.0),
            theme::tokens(ui).muted,
        );
    });
    ui.add_space(4.0);
}

fn row_frame(ui: &mut egui::Ui, selected: bool) -> (egui::Rect, egui::Response) {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(W, ROW), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let inner = rect.shrink2(egui::vec2(6.0, 1.0));
        if selected {
            ui.painter().rect_filled(
                inner,
                egui::CornerRadius::same(6),
                theme::tokens(ui).accent_soft,
            );
        } else if resp.hovered() {
            ui.painter()
                .rect_filled(inner, egui::CornerRadius::same(6), theme::tokens(ui).hover);
        }
    }
    (rect, resp)
}

fn row(
    ui: &mut egui::Ui,
    icon: Icon,
    icon_color: egui::Color32,
    label: &str,
    count: usize,
    selected: bool,
    strong: bool,
) -> egui::Response {
    let (rect, resp) = row_frame(ui, selected);
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        icons::at(
            p,
            egui::pos2(rect.left() + 24.0, rect.center().y),
            15.0,
            icon,
            icon_color,
        );
        let font = if strong || selected {
            theme::bold_font(13.0)
        } else {
            egui::FontId::proportional(13.0)
        };
        let badge_w = if count > 0 { 44.0 } else { 0.0 };
        theme::clipped_text(
            p,
            egui::pos2(rect.left() + 42.0, rect.center().y - 8.0),
            rect.width() - 56.0 - badge_w,
            label,
            font,
            theme::tokens(ui).text,
        );
        if count > 0 {
            theme::badge(
                p,
                egui::pos2(rect.right() - 12.0, rect.center().y),
                count,
                theme::tokens(ui).accent_soft,
                theme::tokens(ui).accent,
            );
        }
    }
    resp
}

/// A muted count at the right end of a sidebar row.
fn muted_count(ui: &egui::Ui, rect: egui::Rect, text: &str) {
    let p = ui.painter();
    let galley = p.layout_no_wrap(
        text.to_string(),
        egui::FontId::proportional(11.5),
        theme::tokens(ui).muted,
    );
    let pos = egui::pos2(
        rect.right() - 14.0 - galley.size().x,
        rect.center().y - galley.size().y / 2.0,
    );
    p.galley(pos, galley, theme::tokens(ui).muted);
}

fn dot_row(ui: &mut egui::Ui, color: egui::Color32, label: &str, selected: bool) -> egui::Response {
    let (rect, resp) = row_frame(ui, selected);
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        p.rect_filled(
            egui::Rect::from_center_size(
                egui::pos2(rect.left() + 24.0, rect.center().y),
                egui::vec2(8.0, 8.0),
            ),
            egui::CornerRadius::same(2),
            color,
        );
        let font = if selected {
            theme::bold_font(13.0)
        } else {
            egui::FontId::proportional(13.0)
        };
        theme::clipped_text(
            p,
            egui::pos2(rect.left() + 42.0, rect.center().y - 8.0),
            rect.width() - 56.0,
            label,
            font,
            theme::tokens(ui).text,
        );
    }
    resp
}

fn account_row(
    ui: &mut egui::Ui,
    letter: &str,
    color: egui::Color32,
    label: &str,
    count: usize,
) -> egui::Response {
    let (rect, resp) = row_frame(ui, false);
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        theme::paint_avatar(
            p,
            egui::pos2(rect.left() + 24.0, rect.center().y),
            letter,
            color,
            9.0,
        );
        let badge_w = if count > 0 { 44.0 } else { 0.0 };
        theme::clipped_text(
            p,
            egui::pos2(rect.left() + 42.0, rect.center().y - 8.0),
            rect.width() - 56.0 - badge_w,
            label,
            egui::FontId::proportional(13.0),
            theme::tokens(ui).text,
        );
        if count > 0 {
            theme::badge(
                p,
                egui::pos2(rect.right() - 12.0, rect.center().y),
                count,
                theme::tokens(ui).accent_soft,
                theme::tokens(ui).accent,
            );
        }
    }
    resp
}
