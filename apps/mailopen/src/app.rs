//! Application state, the event loop and every action the UI can trigger.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use mailopen_core::{
    Account, AccountWorker, Config, Contacts, Event, Folder, FolderKind, MessageBody,
    MessageSummary, OutgoingMessage, Request, SignatureKind, imap_client::ImapClient, secrets,
};

use crate::ui::account_dialog::AccountDialog;
use crate::ui::compose::ComposeState;
use crate::ui::format;

/// Pseudo account id used by the add-account dialog's test login.
pub const DIALOG_ID: &str = "__dialog__";

pub type MsgKey = (String, String, u32);

/// What the account rail has selected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rail {
    All,
    Account(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum View {
    /// Every account's inbox, merged.
    Unified,
    Folder {
        account: String,
        folder: String,
    },
}

pub struct Status {
    pub text: String,
    pub is_error: bool,
    pub at: Instant,
}

pub struct MailApp {
    pub config: Config,
    pub avatars: crate::ui::avatars::AvatarCache,
    pub contacts: Contacts,
    contacts_dirty: bool,
    pub workers: HashMap<String, AccountWorker>,
    events_rx: Receiver<(String, Event)>,
    pub events_tx: Sender<(String, Event)>,
    pub wake: mailopen_core::worker::Wake,
    pub folders: HashMap<String, Vec<Folder>>,
    pub messages: HashMap<(String, String), Vec<MessageSummary>>,
    pub bodies: HashMap<MsgKey, MessageBody>,
    pub body_errors: HashMap<MsgKey, String>,
    pub rail: Rail,
    pub view: View,
    pub selected: Option<MsgKey>,
    pub search: String,
    pub search_open: bool,
    pub settings_open: bool,
    pub search_focus: bool,
    pub compose: Option<ComposeState>,
    pub account_dialog: Option<AccountDialog>,
    pub status: Option<Status>,
    pub busy: HashMap<String, usize>,
    /// Accounts whose password could not be read from the keyring.
    pub account_errors: HashMap<String, String>,
    last_refresh: Instant,
    last_contacts_save: Instant,
}

impl MailApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        Self::new_with_ctx(&cc.egui_ctx)
    }

    /// Build the app for any context (the window, or a headless harness).
    pub fn new_with_ctx(egui_ctx: &egui::Context) -> Self {
        let ctx = egui_ctx.clone();
        let wake: mailopen_core::worker::Wake = Arc::new(move || ctx.request_repaint());
        let (events_tx, events_rx) = channel();
        let (config, status) = match Config::load() {
            Ok(c) => (c, None),
            Err(e) => (
                Config::default(),
                Some(Status {
                    text: format!("Configurazione non letta: {e}"),
                    is_error: true,
                    at: Instant::now(),
                }),
            ),
        };
        let contacts = Contacts::load().unwrap_or_else(|e| {
            log::warn!("rubrica non letta: {e}");
            Contacts::default()
        });
        let avatars = crate::ui::avatars::AvatarCache::new(Arc::clone(&wake), config.sender_logos);
        let mut app = MailApp {
            config,
            avatars,
            contacts,
            contacts_dirty: false,
            workers: HashMap::new(),
            events_rx,
            events_tx,
            wake,
            folders: HashMap::new(),
            messages: HashMap::new(),
            bodies: HashMap::new(),
            body_errors: HashMap::new(),
            rail: Rail::All,
            view: View::Unified,
            selected: None,
            search: String::new(),
            search_open: false,
            settings_open: false,
            search_focus: false,
            compose: None,
            account_dialog: None,
            status,
            busy: HashMap::new(),
            account_errors: HashMap::new(),
            last_refresh: Instant::now(),
            last_contacts_save: Instant::now(),
        };
        if std::env::var_os("MAILOPEN_DEMO").is_some() {
            app.load_demo();
            return app;
        }
        let accounts = app.config.accounts.clone();
        for account in accounts {
            app.start_account(&account);
        }
        if app.config.accounts.is_empty() {
            app.account_dialog = Some(AccountDialog::new(app.config.next_color()));
        }
        app
    }

    /// Fake accounts and messages for looking at the UI without a server (`MAILOPEN_DEMO=1`).
    /// Nothing is saved and no worker runs.
    fn load_demo(&mut self) {
        use mailopen_core::SmtpSecurity;
        let mk = |id: &str, name: &str, email: &str, color: u8| Account {
            id: id.into(),
            name: name.into(),
            email: email.into(),
            username: email.into(),
            imap_host: "imap.example.it".into(),
            imap_port: 993,
            smtp_host: "smtp.example.it".into(),
            smtp_port: 465,
            smtp_security: SmtpSecurity::Tls,
            color,
            signature: String::new(),
            signature_kind: SignatureKind::Text,
            signature_html: String::new(),
            style: None,
        };
        self.config.accounts = vec![
            mk("demo-a", "Lavoro", "e.mancinelli@example.it", 0),
            mk("demo-b", "Personale", "edoardo@example.it", 2),
        ];
        let folder = |name: &str, kind: FolderKind| Folder {
            name: name.into(),
            kind,
            selectable: true,
        };
        let std_folders = |extra: &[&str]| {
            let mut v = vec![
                folder("INBOX", FolderKind::Inbox),
                folder("Drafts", FolderKind::Drafts),
                folder("Sent", FolderKind::Sent),
                folder("Archive", FolderKind::Archive),
                folder("Junk", FolderKind::Spam),
                folder("Trash", FolderKind::Trash),
            ];
            v.extend(extra.iter().map(|n| folder(n, FolderKind::Other)));
            v
        };
        self.folders.insert(
            "demo-a".into(),
            std_folders(&[
                "AUDIT",
                "Contratti",
                "Ordini in corso",
                "Preventivi",
                "Problemi tecnici",
            ]),
        );
        self.folders
            .insert("demo-b".into(), std_folders(&["Famiglia", "Viaggi"]));
        let now = chrono::Utc::now().timestamp();
        type Row = (
            &'static str,
            &'static str,
            &'static str,
            &'static str,
            &'static str,
            i64,
            bool,
            bool,
        );
        let rows: &[Row] = &[
            (
                "demo-a",
                "sales@evcharge.example",
                "",
                "Re: RFQ for split-type DC charging stations",
                "Dear Edoardo, after internal discussions we can confirm the revised pricing for the 120 kW units and the delivery window in Q1.",
                600,
                false,
                false,
            ),
            (
                "demo-a",
                "amministrazione@bambutech.example",
                "",
                "Test invio – collegamento casella",
                "Mail di test inviata per verificare il collegamento della casella amministrazione.",
                1800,
                false,
                false,
            ),
            (
                "demo-a",
                "noreply@monitor.example",
                "",
                "ALERT rimbalzo mail: 2 non recapitate",
                "Il server di posta ha rifiutato queste mail: destinatario sconosciuto (550). Controllare gli indirizzi nel CRM.",
                3000,
                true,
                false,
            ),
            (
                "demo-b",
                "giulia@progetto-eu.example",
                "Giulia Conti",
                "Canceled: WP5 monthly call",
                "The monthly call is cancelled this week; the agenda moves to the next slot. Minutes attached.",
                7200,
                true,
                true,
            ),
            (
                "demo-a",
                "d.ferri@officina.example",
                "Davide Ferri",
                "I: Vedi altre conversioni che stai già generando",
                "Da: The Business Team. Ciao, abbiamo notato che la tua pagina sta ricevendo nuovi contatti dalle campagne attive.",
                86400,
                true,
                false,
            ),
            (
                "demo-b",
                "info@consulenza.example",
                "Alma Consulting s.r.l.",
                "I: Wallbox in elenco GSE - listino a confronto",
                "Ciao Edoardo, buon pomeriggio, spero tutto bene. Ti giro il confronto listino richiesto con le note sul bonus colonnine.",
                90000,
                true,
                true,
            ),
            (
                "demo-a",
                "p.kumar@charger.example",
                "Priya Kumar",
                "RE: DC charger received - installation",
                "Dear team, the charger was received at the site; installation is scheduled for Monday with the local electrician.",
                172800,
                true,
                false,
            ),
        ];
        for (i, (acc, addr, name, subject, text, ago, seen, att)) in rows.iter().enumerate() {
            let uid = (100 + i) as u32;
            let summary = MessageSummary {
                account_id: (*acc).into(),
                folder: "INBOX".into(),
                uid,
                from_name: (*name).into(),
                from_addr: (*addr).into(),
                subject: (*subject).into(),
                timestamp: now - ago,
                seen: *seen,
                answered: false,
                flagged: false,
                size: 12_000 + (i as u32) * 4000,
                message_id: format!("<demo-{i}@example.it>"),
                snippet: text.chars().take(120).collect(),
                has_attachment: *att,
            };
            self.messages
                .entry(((*acc).into(), "INBOX".into()))
                .or_default()
                .push(summary);
            self.bodies.insert(
                ((*acc).into(), "INBOX".into(), uid),
                MessageBody {
                    to: vec!["e.mancinelli@example.it".into()],
                    cc: Vec::new(),
                    reply_to: None,
                    text: format!(
                        "{text}\n\nCordiali saluti,\n{}",
                        if name.is_empty() { *addr } else { *name }
                    ),
                    was_html: false,
                    attachments: if *att {
                        vec!["documento.pdf".into()]
                    } else {
                        Vec::new()
                    },
                    message_id: format!("demo-{i}@example.it"),
                    references: Vec::new(),
                },
            );
        }
        for (n, m) in [
            ("Anna Rossi", "anna.rossi@example.it"),
            ("Marco Verdi", "marco@verdi.it"),
            ("Davide Ferri", "d.ferri@officina.example"),
        ] {
            self.contacts.remember(n, m, now);
        }
        self.select_message(("demo-a".into(), "INBOX".into(), 101));
        self.set_status("Modalità demo: dati finti, nessuna connessione", false);
    }

    /// Read the password and start the worker; on failure remember why (shown in the sidebar).
    pub fn start_account(&mut self, account: &Account) {
        match secrets::load(&account.id) {
            Ok(password) => {
                let worker = AccountWorker::spawn(
                    account.clone(),
                    password,
                    self.events_tx.clone(),
                    Arc::clone(&self.wake),
                );
                worker.send(Request::ListFolders);
                self.workers.insert(account.id.clone(), worker);
                self.account_errors.remove(&account.id);
            }
            Err(e) => {
                self.account_errors
                    .insert(account.id.clone(), e.to_string());
            }
        }
    }

    pub fn request(&self, account_id: &str, req: Request) {
        if let Some(w) = self.workers.get(account_id) {
            w.send(req);
        }
    }

    pub fn set_status(&mut self, text: impl Into<String>, is_error: bool) {
        self.status = Some(Status {
            text: text.into(),
            is_error,
            at: Instant::now(),
        });
    }

    pub fn is_busy(&self) -> bool {
        self.busy.values().any(|n| *n > 0)
    }

    pub fn folder_of_kind(&self, account_id: &str, kind: FolderKind) -> Option<String> {
        self.folders
            .get(account_id)?
            .iter()
            .find(|f| f.kind == kind && f.selectable)
            .map(|f| f.name.clone())
    }

    pub fn inbox_name(&self, account_id: &str) -> String {
        self.folder_of_kind(account_id, FolderKind::Inbox)
            .unwrap_or_else(|| "INBOX".to_string())
    }

    pub fn unread_count(&self, account_id: &str, folder: &str) -> usize {
        self.messages
            .get(&(account_id.to_string(), folder.to_string()))
            .map(|v| v.iter().filter(|m| !m.seen).count())
            .unwrap_or(0)
    }

    /// Messages of the current view, newest first, filtered by the search box.
    pub fn visible_messages(&self) -> Vec<MessageSummary> {
        let mut out: Vec<MessageSummary> = match &self.view {
            View::Unified => self
                .config
                .accounts
                .iter()
                .flat_map(|a| {
                    let inbox = self.inbox_name(&a.id);
                    self.messages
                        .get(&(a.id.clone(), inbox))
                        .cloned()
                        .unwrap_or_default()
                })
                .collect(),
            View::Folder { account, folder } => self
                .messages
                .get(&(account.clone(), folder.clone()))
                .cloned()
                .unwrap_or_default(),
        };
        let needle = self.search.trim().to_lowercase();
        if !needle.is_empty() {
            out.retain(|m| {
                m.from_name.to_lowercase().contains(&needle)
                    || m.from_addr.to_lowercase().contains(&needle)
                    || m.subject.to_lowercase().contains(&needle)
            });
        }
        out.sort_by(|a, b| {
            b.timestamp
                .cmp(&a.timestamp)
                .then_with(|| b.uid.cmp(&a.uid))
        });
        out
    }

    pub fn selected_summary(&self) -> Option<MessageSummary> {
        let (acc, folder, uid) = self.selected.as_ref()?;
        self.messages
            .get(&(acc.clone(), folder.clone()))?
            .iter()
            .find(|m| m.uid == *uid)
            .cloned()
    }

    /// (Re)load the folders shown by the current view.
    pub fn refresh_view(&mut self) {
        let limit = self.config.messages_per_folder;
        match self.view.clone() {
            View::Unified => {
                for a in self.config.accounts.clone() {
                    let folder = self.inbox_name(&a.id);
                    self.request(&a.id, Request::LoadMessages { folder, limit });
                }
            }
            View::Folder { account, folder } => {
                self.request(&account, Request::LoadMessages { folder, limit });
            }
        }
        self.last_refresh = Instant::now();
    }

    pub fn select_rail(&mut self, rail: Rail) {
        let view = match &rail {
            Rail::All => View::Unified,
            Rail::Account(id) => View::Folder {
                account: id.clone(),
                folder: self.inbox_name(id),
            },
        };
        self.rail = rail;
        self.set_view(view);
    }

    pub fn set_view(&mut self, view: View) {
        if self.view != view {
            self.view = view;
            self.selected = None;
            self.refresh_view();
        }
    }

    pub fn select_message(&mut self, key: MsgKey) {
        if self.selected.as_ref() == Some(&key) {
            return;
        }
        let (acc, folder, uid) = key.clone();
        self.selected = Some(key.clone());
        if !self.bodies.contains_key(&key) {
            self.body_errors.remove(&key);
            self.request(
                &acc,
                Request::LoadBody {
                    folder: folder.clone(),
                    uid,
                },
            );
        }
        let unseen = self.summary_mut(&key).map(|m| !m.seen).unwrap_or(false);
        if unseen {
            if let Some(m) = self.summary_mut(&key) {
                m.seen = true;
            }
            self.request(
                &acc,
                Request::SetSeen {
                    folder,
                    uid,
                    seen: true,
                },
            );
        }
    }

    fn summary_mut(&mut self, key: &MsgKey) -> Option<&mut MessageSummary> {
        let (acc, folder, uid) = key;
        self.messages
            .get_mut(&(acc.clone(), folder.clone()))?
            .iter_mut()
            .find(|m| m.uid == *uid)
    }

    pub fn toggle_seen_selected(&mut self) {
        let Some(key) = self.selected.clone() else {
            return;
        };
        let Some(seen) = self.summary_mut(&key).map(|m| {
            m.seen = !m.seen;
            m.seen
        }) else {
            return;
        };
        let (acc, folder, uid) = key;
        self.request(&acc, Request::SetSeen { folder, uid, seen });
    }

    pub fn delete_message(&mut self, key: MsgKey) {
        let (acc, folder, uid) = key.clone();
        let trash = self.folder_of_kind(&acc, FolderKind::Trash);
        if let Some(list) = self.messages.get_mut(&(acc.clone(), folder.clone())) {
            list.retain(|m| m.uid != uid);
        }
        self.bodies.remove(&key);
        if self.selected.as_ref() == Some(&key) {
            self.selected = None;
        }
        self.request(&acc, Request::Delete { folder, uid, trash });
    }

    pub fn delete_selected(&mut self) {
        if let Some(key) = self.selected.clone() {
            self.delete_message(key);
        }
    }

    fn account_index(&self, id: &str) -> usize {
        self.config
            .accounts
            .iter()
            .position(|a| a.id == id)
            .unwrap_or(0)
    }

    pub fn compose_new(&mut self) {
        let idx = match &self.view {
            View::Folder { account, .. } => self.account_index(account),
            View::Unified => 0,
        };
        let mut c = ComposeState::new(idx);
        c.body = self.signature_block(idx);
        c.note = self.html_signature_note(idx);
        self.compose = Some(c);
    }

    fn html_signature_note(&self, account_index: usize) -> Option<String> {
        let a = self.config.accounts.get(account_index)?;
        (a.signature_kind == mailopen_core::SignatureKind::Html
            && !a.signature_html.trim().is_empty())
        .then(|| "La firma HTML dell'account viene aggiunta all'invio.".to_string())
    }

    /// The style to draw with: the selected account's own, else the general one.
    pub fn effective_style(&self) -> String {
        if let Rail::Account(id) = &self.rail
            && let Some(s) = self.config.account(id).and_then(|a| a.style.clone())
        {
            return s;
        }
        self.config.style.clone()
    }

    /// "\n\n-- \nfirma" for the account, or empty (HTML signatures are added when sending).
    fn signature_block(&self, account_index: usize) -> String {
        match self
            .config
            .accounts
            .get(account_index)
            .filter(|a| a.signature_kind == mailopen_core::SignatureKind::Text)
            .map(|a| a.signature.trim())
        {
            Some(sig) if !sig.is_empty() => format!("\n\n-- \n{sig}"),
            _ => String::new(),
        }
    }

    pub fn reply(&mut self, all: bool) {
        let Some(summary) = self.selected_summary() else {
            return;
        };
        let Some(key) = self.selected.clone() else {
            return;
        };
        let body = self.bodies.get(&key).cloned().unwrap_or_default();
        let own = self
            .config
            .account(&summary.account_id)
            .map(|a| a.email.to_lowercase())
            .unwrap_or_default();
        let mut c = ComposeState::new(self.account_index(&summary.account_id));
        let from = if summary.from_name.is_empty() {
            summary.from_addr.clone()
        } else {
            format!("{} <{}>", summary.from_name, summary.from_addr)
        };
        c.to = body.reply_to.clone().unwrap_or(from.clone());
        if all {
            let others: Vec<String> = body
                .to
                .iter()
                .chain(body.cc.iter())
                .filter(|a| !a.to_lowercase().contains(&own) || own.is_empty())
                .cloned()
                .collect();
            c.cc = others.join(", ");
            c.show_cc = !c.cc.is_empty();
        }
        c.subject = if summary.subject.to_lowercase().starts_with("re:") {
            summary.subject.clone()
        } else {
            format!("Re: {}", summary.subject)
        };
        c.body = format!(
            "{}\n\nIl {}, {} ha scritto:\n{}",
            self.signature_block(c.account_index),
            format::long_date(summary.timestamp),
            from,
            format::quote(&body.text)
        );
        let id = if body.message_id.is_empty() {
            summary.message_id.clone()
        } else {
            body.message_id.clone()
        };
        c.in_reply_to = if id.is_empty() {
            None
        } else {
            Some(id.clone())
        };
        c.references = body.references.clone();
        if !id.is_empty() {
            c.references.push(id);
        }
        self.compose = Some(c);
    }

    pub fn forward(&mut self) {
        let Some(summary) = self.selected_summary() else {
            return;
        };
        let Some(key) = self.selected.clone() else {
            return;
        };
        let body = self.bodies.get(&key).cloned().unwrap_or_default();
        let mut c = ComposeState::new(self.account_index(&summary.account_id));
        c.subject = if summary.subject.to_lowercase().starts_with("fwd:") {
            summary.subject.clone()
        } else {
            format!("Fwd: {}", summary.subject)
        };
        c.body = format!(
            "{}\n\n---------- Messaggio inoltrato ----------\nDa: {} <{}>\nData: {}\nOggetto: {}\nA: {}\n\n{}",
            self.signature_block(c.account_index),
            summary.from_name,
            summary.from_addr,
            format::long_date(summary.timestamp),
            summary.subject,
            body.to.join(", "),
            body.text
        );
        if !body.attachments.is_empty() {
            c.note = Some(format!(
                "Gli allegati ({}) non vengono inoltrati in questa versione.",
                body.attachments.join(", ")
            ));
        }
        self.compose = Some(c);
    }

    /// Called by the compose window's Send button.
    pub fn send_compose(&mut self) {
        let Some(c) = self.compose.as_mut() else {
            return;
        };
        let Some(account) = self.config.accounts.get(c.account_index).cloned() else {
            c.error = Some("Nessun account".into());
            return;
        };
        let message = OutgoingMessage {
            to: vec![c.to.clone()],
            cc: vec![c.cc.clone()],
            bcc: vec![c.bcc.clone()],
            subject: c.subject.clone(),
            body: c.body.clone(),
            in_reply_to: c.in_reply_to.clone(),
            references: c.references.clone(),
            html_body: mailopen_core::markup::has_markup(&c.body)
                .then(|| mailopen_core::markup::to_html(&c.body)),
            html_signature: (account.signature_kind == mailopen_core::SignatureKind::Html
                && !account.signature_html.trim().is_empty())
            .then(|| account.signature_html.clone()),
        };
        c.sending = true;
        c.error = None;
        let now = chrono::Utc::now().timestamp();
        let entries = [message.to.clone(), message.cc.clone(), message.bcc.clone()].concat();
        if self.contacts.remember_list(&entries, now) {
            self.contacts_dirty = true;
        }
        self.save_contacts();
        let sent_folder = self.folder_of_kind(&account.id, FolderKind::Sent);
        self.request(
            &account.id,
            Request::Send {
                message,
                sent_folder,
            },
        );
    }

    /// Write the address book when it changed (not in demo mode).
    pub fn save_contacts(&mut self) {
        if !self.contacts_dirty || std::env::var_os("MAILOPEN_DEMO").is_some() {
            return;
        }
        match self.contacts.save() {
            Ok(()) => self.contacts_dirty = false,
            Err(e) => log::warn!("rubrica non salvata: {e}"),
        }
    }

    /// Called by the account dialog: test the credentials on a throwaway thread.
    pub fn test_account(&mut self, account: Account, password: String) {
        let password = if password.is_empty() {
            match secrets::load(&account.id) {
                Ok(p) => p,
                Err(e) => {
                    if let Some(d) = self.account_dialog.as_mut() {
                        d.error = Some(format!("Password non trovata nel portachiavi: {e}"));
                    }
                    return;
                }
            }
        } else {
            password
        };
        let tx = self.events_tx.clone();
        let wake = Arc::clone(&self.wake);
        if let Some(d) = self.account_dialog.as_mut() {
            d.testing = true;
            d.error = None;
            d.pending = Some((account.clone(), password.clone()));
        }
        let spawned = std::thread::Builder::new()
            .name("mailopen-test-login".into())
            .spawn(move || {
                let ev = match ImapClient::test_login(&account, &password) {
                    Ok(()) => Event::LoginOk,
                    Err(e) => Event::Failed {
                        what: "Accesso".into(),
                        error: e.to_string(),
                    },
                };
                let _ = tx.send((DIALOG_ID.to_string(), ev));
                wake();
            });
        if let Err(e) = spawned {
            self.set_status(format!("Thread di prova non avviato: {e}"), true);
        }
    }

    fn finish_account_dialog(&mut self) {
        let Some(mut dialog) = self.account_dialog.take() else {
            return;
        };
        let Some((account, password)) = dialog.pending.take() else {
            return;
        };
        if let Err(e) = secrets::store(&account.id, &password) {
            self.set_status(format!("Password non salvata nel portachiavi: {e}"), true);
        }
        if let Some(old) = self.workers.remove(&account.id) {
            drop(old);
        }
        self.config.remove(&account.id);
        self.config.accounts.push(account.clone());
        if let Err(e) = self.config.save() {
            self.set_status(format!("Configurazione non salvata: {e}"), true);
        } else {
            self.set_status(format!("Account {} aggiunto", account.email), false);
        }
        self.start_account(&account);
    }

    pub fn remove_account(&mut self, id: &str) {
        self.workers.remove(id);
        self.config.remove(id);
        self.folders.remove(id);
        self.messages.retain(|(acc, _), _| acc != id);
        self.bodies.retain(|(acc, _, _), _| acc != id);
        self.account_errors.remove(id);
        if matches!(&self.view, View::Folder { account, .. } if account == id) {
            self.rail = Rail::All;
            self.view = View::Unified;
            self.selected = None;
        }
        if let Err(e) = secrets::delete(id) {
            log::warn!("portachiavi: {e}");
        }
        match self.config.save() {
            Ok(()) => self.set_status("Account rimosso", false),
            Err(e) => self.set_status(format!("Configurazione non salvata: {e}"), true),
        }
    }

    pub fn edit_account(&mut self, id: &str) {
        if let Some(a) = self.config.account(id).cloned() {
            self.account_dialog = Some(AccountDialog::edit(a));
        }
    }

    fn handle_event(&mut self, id: String, ev: Event) {
        if id == DIALOG_ID {
            match ev {
                Event::LoginOk => self.finish_account_dialog(),
                Event::Failed { error, .. } => {
                    if let Some(d) = self.account_dialog.as_mut() {
                        d.testing = false;
                        d.pending = None;
                        d.error = Some(error);
                    }
                }
                _ => {}
            }
            return;
        }
        match ev {
            Event::Folders(folders) => {
                self.folders.insert(id.clone(), folders);
                let inbox = self.inbox_name(&id);
                let limit = self.config.messages_per_folder;
                self.request(
                    &id,
                    Request::LoadMessages {
                        folder: inbox,
                        limit,
                    },
                );
            }
            Event::Messages { folder, messages } => {
                for m in &messages {
                    self.contacts_dirty |=
                        self.contacts
                            .remember(&m.from_name, &m.from_addr, m.timestamp);
                }
                self.messages.insert((id, folder), messages);
            }
            Event::Body { folder, uid, body } => {
                let now = chrono::Utc::now().timestamp();
                self.contacts_dirty |= self.contacts.remember_list(&body.to, now);
                self.contacts_dirty |= self.contacts.remember_list(&body.cc, now);
                self.bodies.insert((id, folder, uid), body);
            }
            Event::SeenChanged { folder, uid, seen } => {
                if let Some(m) = self.summary_mut(&(id, folder, uid)) {
                    m.seen = seen;
                }
            }
            Event::Deleted { .. } => {}
            Event::Sent => {
                self.compose = None;
                self.set_status("Messaggio inviato", false);
            }
            Event::LoginOk => {}
            Event::Failed { what, error } => {
                if what == "Invio" {
                    if let Some(c) = self.compose.as_mut() {
                        c.sending = false;
                        c.error = Some(error.clone());
                    }
                } else if what == "Apertura messaggio"
                    && let Some(key) = self.selected.clone()
                {
                    self.body_errors.insert(key, error.clone());
                }
                let who = self
                    .config
                    .account(&id)
                    .map(|a| a.email.clone())
                    .unwrap_or_default();
                self.set_status(format!("{who}: {what}: {error}"), true);
            }
            Event::Busy(n) => {
                self.busy.insert(id, n);
            }
        }
    }

    fn handle_events(&mut self) {
        while let Ok((id, ev)) = self.events_rx.try_recv() {
            self.handle_event(id, ev);
        }
    }

    fn handle_keys(&mut self, ctx: &egui::Context) {
        let typing = ctx.memory(|m| m.focused().is_some());
        if typing {
            return;
        }
        let (del, cmd_n, cmd_r) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace),
                i.modifiers.command && i.key_pressed(egui::Key::N),
                i.modifiers.command && i.key_pressed(egui::Key::R),
            )
        });
        if del && self.compose.is_none() && self.account_dialog.is_none() {
            self.delete_selected();
        }
        if cmd_n && self.compose.is_none() {
            self.compose_new();
        }
        if cmd_r && self.compose.is_none() && self.selected.is_some() {
            self.reply(false);
        }
    }
}

impl eframe::App for MailApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.root(ui);
    }
}

impl MailApp {
    /// One frame of the whole window. Shared by the app and the headless harness.
    pub fn root(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if !crate::ui::theme::ensure(&ctx, self.config.theme) {
            ctx.request_repaint();
            return;
        }
        crate::ui::theme::apply_style(&ctx, &self.effective_style());
        self.avatars.enabled = self.config.sender_logos;
        self.avatars.poll(&ctx);
        let ctx = &ctx;
        self.handle_events();
        if self.last_refresh.elapsed() >= Duration::from_secs(self.config.refresh_seconds) {
            self.refresh_view();
        }
        if self.contacts_dirty && self.last_contacts_save.elapsed() >= Duration::from_secs(20) {
            self.save_contacts();
            self.last_contacts_save = Instant::now();
        }
        if let Some(s) = &self.status
            && !s.is_error
            && s.at.elapsed() > Duration::from_secs(6)
        {
            self.status = None;
        }
        self.handle_keys(ctx);

        self.status_bar(ui);
        self.rail(ui);
        self.sidebar(ui);
        self.message_list(ui);
        self.reader(ui);
        self.compose_window(ctx);
        self.account_dialog_window(ctx);
        self.settings_window(ctx);

        ctx.request_repaint_after(Duration::from_secs(5));
    }
}

impl MailApp {
    fn status_bar(&mut self, root: &mut egui::Ui) {
        let Some(status) = self.status.as_ref().map(|s| (s.text.clone(), s.is_error)) else {
            return;
        };
        let mut dismiss = false;
        egui::Panel::bottom("status")
            .frame(
                egui::Frame::NONE
                    .fill(crate::ui::theme::tokens(root).surface_alt)
                    .inner_margin(egui::Margin::symmetric(12, 6)),
            )
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    let color = if status.1 {
                        crate::ui::theme::ERROR
                    } else {
                        crate::ui::theme::OK
                    };
                    ui.label(egui::RichText::new(&status.0).color(color).size(12.5));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if crate::ui::icons::button(
                            ui,
                            crate::ui::icons::Icon::Close,
                            11.0,
                            crate::ui::theme::tokens(ui).muted,
                            "",
                        )
                        .clicked()
                        {
                            dismiss = true;
                        }
                    });
                });
            });
        if dismiss {
            self.status = None;
        }
    }
}
