//! The Documenti view: every file attached to the mail, from the document index (also the
//! messages that are not on screen). Filter by type; search by name, sender, subject and the text
//! inside PDF, Word, Excel and text files. A click shows the message, a double click the file.

use std::collections::HashMap;
use std::sync::Arc;

use mailopen_core::documents::query_words;
use mailopen_core::{Attachment, DocEntry, FileKind};

use crate::app::{DocOp, MailApp, kind_group};
use crate::ui::icons::{self, Icon};
use crate::ui::list::pill;
use crate::ui::viewer::file_badge;
use crate::ui::{format, theme};

const ROW_H: f32 = 70.0;

/// The search box's id (Ctrl+F focuses it).
pub fn search_id() -> egui::Id {
    egui::Id::unique("mailopen-doc-search")
}

const KINDS: [(Option<FileKind>, &str); 9] = [
    (None, "Tutti"),
    (Some(FileKind::Pdf), "PDF"),
    (Some(FileKind::Document), "Word"),
    (Some(FileKind::Spreadsheet), "Excel"),
    (Some(FileKind::Image), "Immagini"),
    (Some(FileKind::Presentation), "Presentazioni"),
    (Some(FileKind::Text), "Testo"),
    (Some(FileKind::Archive), "Archivi"),
    (Some(FileKind::Other), "Altro"),
];

enum Action {
    Open(Arc<DocEntry>, Option<DocOp>),
    GoTo(Arc<DocEntry>),
    Kind(Option<FileKind>),
    Reindex,
}

impl MailApp {
    pub fn documents_list(&mut self, root: &mut egui::Ui) {
        let mut action: Option<Action> = None;
        let docs = self.filtered_documents();
        let mut counts: HashMap<FileKind, usize> = HashMap::new();
        let mut total = 0usize;
        for (acc, list) in &self.documents {
            if !self.in_scope(acc) || self.config.account(acc).is_none() {
                continue;
            }
            for e in list.iter() {
                *counts.entry(kind_group(e.kind())).or_default() += 1;
                total += 1;
            }
        }
        let status: Vec<String> = self
            .index_status
            .iter()
            .filter(|(acc, _)| self.in_scope(acc))
            .map(|(_, s)| s.clone())
            .collect();
        let mut query = self.doc_query.clone();
        let words = query_words(&query);
        let multi_account = self.config.accounts.len() > 1;
        egui::Panel::left("list")
            .resizable(true)
            .default_size(340.0)
            .size_range(260.0..=620.0)
            .frame(egui::Frame::NONE.fill(theme::tokens(root).surface))
            .show(root, |ui| {
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    ui.label(theme::bold("Documenti", 18.0));
                    ui.add_space(2.0);
                    if icons::button(
                        ui,
                        Icon::Refresh,
                        15.0,
                        theme::tokens(ui).muted,
                        "Cerca nuovi documenti nella posta",
                    )
                    .clicked()
                    {
                        action = Some(Action::Reindex);
                    }
                    if !status.is_empty() {
                        ui.spinner();
                    }
                });
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    ui.add(
                        egui::TextEdit::singleline(&mut query)
                            .id(search_id())
                            .hint_text("Nome, mittente, oggetto o testo del file")
                            .desired_width(ui.available_width() - 20.0),
                    );
                });
                ui.add_space(6.0);
                egui::Frame::NONE
                    .inner_margin(egui::Margin {
                        left: 12,
                        right: 10,
                        top: 0,
                        bottom: 0,
                    })
                    .show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
                            for (kind, label) in KINDS {
                                let n = match kind {
                                    None => total,
                                    Some(k) => counts.get(&k).copied().unwrap_or(0),
                                };
                                if kind.is_some() && n == 0 && self.doc_kind != kind {
                                    continue;
                                }
                                let text = format!("{label} {n}");
                                if pill(ui, &text, self.doc_kind == kind).clicked() {
                                    action = Some(Action::Kind(kind));
                                }
                            }
                        })
                    });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    let mut line = if docs.len() == total {
                        format!("{total} documenti")
                    } else {
                        format!("{} di {total} documenti", docs.len())
                    };
                    if let Some(s) = status.first() {
                        line.push_str(" · ");
                        line.push_str(s);
                    }
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(line)
                                .color(theme::tokens(ui).muted)
                                .size(11.5),
                        )
                        .truncate(),
                    );
                });
                ui.add_space(6.0);
                theme::hline(ui);
                if docs.is_empty() {
                    ui.add_space(40.0);
                    ui.vertical_centered(|ui| {
                        let text = if total > 0 {
                            "Nessun documento trovato."
                        } else if !status.is_empty() {
                            "Indicizzazione in corso…"
                        } else if !self.config.documents_index {
                            "L'indice dei documenti è spento (Impostazioni)."
                        } else {
                            "Ancora nessun documento.\nL'indice si costruisce in background."
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
                    .id_salt("documents")
                    .show_rows(ui, ROW_H, docs.len(), |ui, range| {
                        for e in docs
                            .iter()
                            .skip(range.start)
                            .take(range.end.saturating_sub(range.start))
                        {
                            let selected = self.selected_doc.as_ref()
                                == Some(&(
                                    (e.account_id.clone(), e.folder.clone(), e.uid),
                                    e.index,
                                ));
                            let stripe = if multi_account {
                                self.config
                                    .account(&e.account_id)
                                    .map(|a| theme::account_color(a.color))
                            } else {
                                None
                            };
                            let folder = self.folder_label(&e.account_id, &e.folder);
                            let resp = row(ui, e, selected, stripe, &folder, &words);
                            if resp.double_clicked() {
                                action = Some(Action::Open(Arc::clone(e), Some(DocOp::Preview)));
                            } else if resp.clicked() {
                                action = Some(Action::Open(Arc::clone(e), None));
                            }
                            resp.context_menu(|ui| {
                                for (label, op) in [
                                    ("Anteprima", DocOp::Preview),
                                    ("Apri con l'app predefinita", DocOp::OpenWith),
                                    ("Salva…", DocOp::Save),
                                    ("Stampa", DocOp::Print),
                                    ("Inoltra", DocOp::Forward),
                                ] {
                                    if ui.button(label).clicked() {
                                        action = Some(Action::Open(Arc::clone(e), Some(op)));
                                        ui.close();
                                    }
                                }
                                ui.separator();
                                if ui.button("Vai al messaggio").clicked() {
                                    action = Some(Action::GoTo(Arc::clone(e)));
                                    ui.close();
                                }
                            });
                        }
                    });
            });
        if query != self.doc_query {
            self.doc_query = query;
        }
        match action {
            Some(Action::Open(e, op)) => self.open_document(&e, op),
            Some(Action::GoTo(e)) => self.go_to_message(&e),
            Some(Action::Kind(k)) => self.doc_kind = k,
            Some(Action::Reindex) => {
                self.run_all_indexes();
                if self.indexers.is_empty() {
                    self.set_status("L'indice dei documenti è spento (Impostazioni)", false);
                }
            }
            None => {}
        }
    }
}

fn row(
    ui: &mut egui::Ui,
    e: &DocEntry,
    selected: bool,
    stripe: Option<egui::Color32>,
    folder: &str,
    words: &[String],
) -> egui::Response {
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, ROW_H), egui::Sense::click());
    if !ui.is_rect_visible(rect) {
        return resp;
    }
    let t = theme::tokens(ui);
    let (text, muted) = if selected {
        ui.painter()
            .rect_filled(rect, egui::CornerRadius::ZERO, t.accent);
        (egui::Color32::WHITE, egui::Color32::from_white_alpha(205))
    } else {
        if resp.hovered() {
            ui.painter()
                .rect_filled(rect, egui::CornerRadius::ZERO, t.hover);
        }
        ui.painter().line_segment(
            [
                egui::pos2(rect.left() + 56.0, rect.bottom()),
                egui::pos2(rect.right(), rect.bottom()),
            ],
            egui::Stroke::new(1.0, t.border),
        );
        (t.text, t.muted)
    };
    if let Some(c) = stripe {
        ui.painter().rect_filled(
            egui::Rect::from_min_size(
                egui::pos2(rect.left(), rect.top() + 8.0),
                egui::vec2(3.0, ROW_H - 16.0),
            ),
            egui::CornerRadius::same(2),
            c,
        );
    }
    let badge = Attachment {
        name: e.name.clone(),
        mime: e.mime.clone(),
        content_id: None,
        data: Arc::from(&[][..]),
    };
    let mut badge_ui = ui.new_child(egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
        rect.min + egui::vec2(16.0, 12.0),
        egui::vec2(30.0, 36.0),
    )));
    file_badge(&mut badge_ui, &badge, 36.0);
    let p = ui.painter();
    let left = rect.left() + 56.0;
    let date_galley = p.layout_no_wrap(
        format::short_date(e.timestamp),
        egui::FontId::proportional(11.5),
        muted,
    );
    let date_w = date_galley.size().x;
    p.galley(
        egui::pos2(rect.right() - 12.0 - date_w, rect.top() + 10.0),
        date_galley,
        muted,
    );
    theme::clipped_text(
        p,
        egui::pos2(left, rect.top() + 9.0),
        rect.right() - 20.0 - date_w - left,
        &e.name,
        theme::bold_font(13.0),
        text,
    );
    let from = if e.from_name.trim().is_empty() {
        e.from_addr.as_str()
    } else {
        e.from_name.as_str()
    };
    let subject = if e.subject.is_empty() {
        "(senza oggetto)"
    } else {
        e.subject.as_str()
    };
    theme::clipped_text(
        p,
        egui::pos2(left, rect.top() + 28.0),
        rect.right() - 12.0 - left,
        &format!("{from} · {subject}"),
        egui::FontId::proportional(12.0),
        text,
    );
    let third = (!words.is_empty())
        .then(|| e.excerpt(words, 90))
        .flatten()
        .unwrap_or_else(|| {
            format!(
                "{} · {}",
                format::size(u32::try_from(e.size).unwrap_or(u32::MAX)),
                folder
            )
        });
    theme::clipped_text(
        p,
        egui::pos2(left, rect.top() + 47.0),
        rect.right() - 12.0 - left,
        &third,
        egui::FontId::proportional(11.5),
        muted,
    );
    resp.on_hover_text(format!(
        "{}\nDoppio clic: anteprima · tasto destro: altre azioni",
        e.name
    ))
}
