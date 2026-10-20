//! Add / edit an account: pick a provider, type address and password, test, save.

use mailopen_core::presets::{PRESETS, for_email};
use mailopen_core::{Account, SignatureKind, SmtpSecurity};

use crate::app::MailApp;
use crate::ui::theme;

#[derive(Clone, Debug)]
pub struct AccountDialog {
    pub edit_id: Option<String>,
    pub preset: usize,
    pub name: String,
    pub email: String,
    pub password: String,
    pub username: String,
    pub imap_host: String,
    pub imap_port: u16,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_security: SmtpSecurity,
    pub color: u8,
    pub signature: String,
    pub signature_kind: SignatureKind,
    pub signature_html: String,
    pub style: Option<String>,
    pub advanced: bool,
    pub testing: bool,
    pub error: Option<String>,
    pub pending: Option<(Account, String)>,
    pub open: bool,
    last_email: String,
}

impl AccountDialog {
    pub fn new(color: u8) -> Self {
        let manual = PRESETS.len().saturating_sub(1);
        let mut d = AccountDialog {
            edit_id: None,
            preset: manual,
            name: String::new(),
            email: String::new(),
            password: String::new(),
            username: String::new(),
            imap_host: String::new(),
            imap_port: 993,
            smtp_host: String::new(),
            smtp_port: 465,
            smtp_security: SmtpSecurity::Tls,
            color,
            signature: String::new(),
            signature_kind: SignatureKind::Text,
            signature_html: String::new(),
            style: None,
            advanced: false,
            testing: false,
            error: None,
            pending: None,
            open: true,
            last_email: String::new(),
        };
        d.apply_preset();
        d
    }

    pub fn edit(a: Account) -> Self {
        AccountDialog {
            edit_id: Some(a.id.clone()),
            preset: PRESETS.len().saturating_sub(1),
            name: a.name.clone(),
            email: a.email.clone(),
            password: String::new(),
            username: a.username.clone(),
            imap_host: a.imap_host.clone(),
            imap_port: a.imap_port,
            smtp_host: a.smtp_host.clone(),
            smtp_port: a.smtp_port,
            smtp_security: a.smtp_security,
            color: a.color,
            signature: a.signature.clone(),
            signature_kind: a.signature_kind,
            signature_html: a.signature_html.clone(),
            style: a.style.clone(),
            advanced: true,
            testing: false,
            error: None,
            pending: None,
            open: true,
            last_email: a.email,
        }
    }

    fn apply_preset(&mut self) {
        if let Some(p) = PRESETS.get(self.preset) {
            self.imap_host = p.imap_host.to_string();
            self.imap_port = p.imap_port;
            self.smtp_host = p.smtp_host.to_string();
            self.smtp_port = p.smtp_port;
            self.smtp_security = p.smtp_security;
        }
    }

    fn to_account(&self) -> Account {
        let email = self.email.trim().to_string();
        let username = if self.username.trim().is_empty() {
            email.clone()
        } else {
            self.username.trim().to_string()
        };
        let name = if self.name.trim().is_empty() {
            email.split('@').next().unwrap_or("").to_string()
        } else {
            self.name.trim().to_string()
        };
        Account {
            id: self
                .edit_id
                .clone()
                .unwrap_or_else(|| Account::new_id(&email)),
            name,
            email,
            username,
            imap_host: self.imap_host.trim().to_string(),
            imap_port: self.imap_port,
            smtp_host: self.smtp_host.trim().to_string(),
            smtp_port: self.smtp_port,
            smtp_security: self.smtp_security,
            color: self.color,
            signature: self.signature.trim_end().to_string(),
            signature_kind: self.signature_kind,
            signature_html: self.signature_html.clone(),
            style: self.style.clone(),
        }
    }

    fn validate(&self) -> Option<String> {
        if !self.email.contains('@') {
            return Some("Indirizzo e-mail non valido".into());
        }
        if self.password.is_empty() && self.edit_id.is_none() {
            return Some("Inserisci la password".into());
        }
        if self.imap_host.trim().is_empty() || self.smtp_host.trim().is_empty() {
            return Some("Server IMAP e SMTP obbligatori".into());
        }
        None
    }
}

impl MailApp {
    pub fn account_dialog_window(&mut self, ctx: &egui::Context) {
        let Some(d) = self.account_dialog.as_mut() else {
            return;
        };
        let mut test: Option<(Account, String)> = None;
        let mut close = false;
        let title = if d.edit_id.is_some() {
            "Modifica account"
        } else {
            "Aggiungi account"
        };
        let mut open = d.open;
        egui::Window::new(title)
            .id(egui::Id::unique("account-dialog"))
            .collapsible(false)
            .resizable(false)
            .default_width(460.0)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .open(&mut open)
            .show(ctx, |ui| {
                ui.add_enabled_ui(!d.testing, |ui| {
                    egui::Grid::new("account-fields")
                        .num_columns(2)
                        .spacing([10.0, 8.0])
                        .show(ui, |ui| {
                            ui.weak("Provider");
                            let name = PRESETS.get(d.preset).map(|p| p.name).unwrap_or("");
                            let mut changed = false;
                            egui::ComboBox::from_id_salt("preset")
                                .selected_text(name)
                                .width(300.0)
                                .show_ui(ui, |ui| {
                                    for (i, p) in PRESETS.iter().enumerate() {
                                        if ui.selectable_value(&mut d.preset, i, p.name).changed() {
                                            changed = true;
                                        }
                                    }
                                });
                            if changed {
                                d.apply_preset();
                            }
                            ui.end_row();

                            ui.weak("Nome");
                            ui.add(
                                egui::TextEdit::singleline(&mut d.name)
                                    .hint_text("Lavoro, Personale…")
                                    .desired_width(300.0),
                            );
                            ui.end_row();

                            ui.weak("E-mail");
                            ui.add(
                                egui::TextEdit::singleline(&mut d.email)
                                    .hint_text("nome@dominio.it")
                                    .desired_width(300.0),
                            );
                            ui.end_row();

                            ui.weak("Password");
                            ui.add(
                                egui::TextEdit::singleline(&mut d.password)
                                    .password(true)
                                    .desired_width(300.0),
                            );
                            ui.end_row();

                            ui.weak("Firma");
                            ui.label(
                            egui::RichText::new(
                                "Si imposta in Impostazioni → Account (editor classico o HTML).",
                            )
                            .color(theme::tokens(ui).muted)
                            .size(11.5),
                        );
                            ui.end_row();
                        });
                    if d.email != d.last_email {
                        d.last_email = d.email.clone();
                        if d.edit_id.is_none() && d.email.contains('@') {
                            let guess = for_email(&d.email);
                            if guess != d.preset && guess != PRESETS.len().saturating_sub(1) {
                                d.preset = guess;
                                d.apply_preset();
                            }
                        }
                    }
                    if let Some(hint) = PRESETS
                        .get(d.preset)
                        .map(|p| p.hint)
                        .filter(|h| !h.is_empty())
                    {
                        ui.add_space(4.0);
                        ui.weak(hint);
                    }
                    ui.add_space(6.0);
                    ui.checkbox(&mut d.advanced, "Impostazioni server");
                    if d.advanced {
                        egui::Grid::new("server-fields")
                            .num_columns(2)
                            .spacing([10.0, 6.0])
                            .show(ui, |ui| {
                                ui.weak("Utente");
                                ui.add(
                                    egui::TextEdit::singleline(&mut d.username)
                                        .hint_text("= e-mail")
                                        .desired_width(300.0),
                                );
                                ui.end_row();
                                ui.weak("IMAP");
                                ui.horizontal(|ui| {
                                    ui.add(
                                        egui::TextEdit::singleline(&mut d.imap_host)
                                            .desired_width(220.0),
                                    );
                                    ui.add(egui::DragValue::new(&mut d.imap_port));
                                });
                                ui.end_row();
                                ui.weak("SMTP");
                                ui.horizontal(|ui| {
                                    ui.add(
                                        egui::TextEdit::singleline(&mut d.smtp_host)
                                            .desired_width(220.0),
                                    );
                                    ui.add(egui::DragValue::new(&mut d.smtp_port));
                                });
                                ui.end_row();
                                ui.weak("Sicurezza SMTP");
                                ui.horizontal(|ui| {
                                    ui.radio_value(
                                        &mut d.smtp_security,
                                        SmtpSecurity::Tls,
                                        "TLS (465)",
                                    );
                                    ui.radio_value(
                                        &mut d.smtp_security,
                                        SmtpSecurity::StartTls,
                                        "STARTTLS (587)",
                                    );
                                });
                                ui.end_row();
                                ui.weak("Colore");
                                ui.horizontal(|ui| {
                                    for (i, c) in theme::PALETTE.iter().enumerate() {
                                        let i = i as u8;
                                        let r = theme::avatar(
                                            ui,
                                            if d.color == i { "x" } else { "" },
                                            *c,
                                            9.0,
                                            None,
                                        );
                                        if r.interact(egui::Sense::click()).clicked() {
                                            d.color = i;
                                        }
                                    }
                                });
                                ui.end_row();
                            });
                    }
                });
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let label = if d.edit_id.is_some() {
                        "Prova e salva"
                    } else {
                        "Prova e aggiungi"
                    };
                    if ui
                        .add_enabled(!d.testing, egui::Button::new(label))
                        .clicked()
                    {
                        match d.validate() {
                            Some(e) => d.error = Some(e),
                            None => test = Some((d.to_account(), d.password.clone())),
                        }
                    }
                    if d.testing {
                        ui.spinner();
                        ui.weak("verifica dell'accesso IMAP…");
                    }
                    if ui.button("Annulla").clicked() {
                        close = true;
                    }
                });
                if let Some(e) = &d.error {
                    ui.add_space(4.0);
                    ui.colored_label(theme::ERROR, e);
                }
            });
        d.open = open;
        if close || !d.open {
            self.account_dialog = None;
        } else if let Some((account, password)) = test {
            self.test_account(account, password);
        }
    }
}
