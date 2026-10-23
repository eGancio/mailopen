//! Application state, the event loop and every action the UI can trigger.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use mailopen_core::documents::{DocEntry, DocIndex};
use mailopen_core::labels::message_key;
use mailopen_core::{
    Account, AccountWorker, Config, Contacts, DocList, Event, Folder, FolderKind, Indexer, Labels,
    MessageBody, MessageSummary, OutgoingMessage, Request, SignatureKind, imap_client::ImapClient,
    secrets,
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
    /// Messages in a category (by id), from every loaded folder.
    Category(String),
    /// Files attached to the mail, from the document index.
    Documents,
    /// What the servers found for a query.
    Search(String),
}

/// Quick filter above the message list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ListFilter {
    #[default]
    All,
    Unread,
    Flagged,
    Attachments,
}

impl ListFilter {
    pub const ALL: [ListFilter; 4] = [
        ListFilter::All,
        ListFilter::Unread,
        ListFilter::Flagged,
        ListFilter::Attachments,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ListFilter::All => "Tutti",
            ListFilter::Unread => "Non letti",
            ListFilter::Flagged => "Contrassegnati",
            ListFilter::Attachments => "Con allegati",
        }
    }

    fn keeps(self, m: &MessageSummary) -> bool {
        match self {
            ListFilter::All => true,
            ListFilter::Unread => !m.seen,
            ListFilter::Flagged => m.flagged,
            ListFilter::Attachments => m.has_attachment,
        }
    }
}

/// What to do with a document once its message is downloaded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocOp {
    Preview,
    Save,
    OpenWith,
    Print,
    Forward,
}

/// Filtered documents and what they were filtered by: (query, type, rail, index generation).
pub type DocCache = (
    String,
    Option<mailopen_core::FileKind>,
    Rail,
    u64,
    Vec<Arc<DocEntry>>,
);

/// A server search in progress or done.
pub struct SearchState {
    pub query: String,
    pub results: Vec<MsgKey>,
    /// Accounts that have not answered yet.
    pub pending: usize,
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
    /// Inline and attachment images as textures.
    pub images: crate::ui::images::ImageCache,
    /// The attachment viewer window, when open.
    pub viewer: Option<crate::ui::viewer::Viewer>,
    /// Messages whose remote images the user allowed this session.
    pub remote_allowed: HashSet<MsgKey>,
    /// A print started from the reading pane; its outcome goes to the status bar.
    pub background_task: Option<Receiver<Result<String, String>>>,
    /// Categories and the messages classified by hand.
    pub labels: Labels,
    labels_dirty: bool,
    /// The labels file could not be read: never overwrite it.
    labels_locked: bool,
    pub filter: ListFilter,
    /// Rows known outside the loaded folders: search results, documents, classified messages.
    pub extra: HashMap<MsgKey, MessageSummary>,
    pub server_search: Option<SearchState>,
    /// A message to select once the server tells where it is (by `Message-ID`).
    pending_locate: Option<String>,
    pub indexers: HashMap<String, Indexer>,
    pub documents: HashMap<String, DocList>,
    /// What each account's indexer is doing, for the Documenti view.
    pub index_status: HashMap<String, String>,
    last_index: Instant,
    /// Bumped whenever an index changes (invalidates the filtered list).
    pub documents_gen: u64,
    pub doc_query: String,
    pub doc_kind: Option<mailopen_core::FileKind>,
    /// (message, attachment index) of the selected document.
    pub selected_doc: Option<(MsgKey, usize)>,
    /// A document action waiting for its message's body.
    pub pending_doc: Option<(MsgKey, usize, DocOp)>,
    /// Filtered documents for (query, kind, rail, generation).
    pub doc_cache: Option<DocCache>,
    /// Open Impostazioni at the categories.
    pub settings_categories: bool,
    /// Last keystroke in the category editor (the file is written when typing pauses).
    labels_edited: Option<Instant>,
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
        let images = crate::ui::images::ImageCache::new(Arc::clone(&wake));
        let demo = std::env::var_os("MAILOPEN_DEMO").is_some();
        let (labels, labels_locked, status) = match (demo, Labels::load()) {
            (true, _) => (Labels::starter(), true, status),
            (false, Ok(l)) => (l, false, status),
            (false, Err(e)) => (
                Labels::starter(),
                true,
                Some(Status {
                    text: format!("Categorie non lette (il file non viene toccato): {e}"),
                    is_error: true,
                    at: Instant::now(),
                }),
            ),
        };
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
            images,
            viewer: None,
            remote_allowed: HashSet::new(),
            background_task: None,
            labels,
            labels_dirty: false,
            labels_locked,
            filter: ListFilter::All,
            extra: HashMap::new(),
            server_search: None,
            pending_locate: None,
            indexers: HashMap::new(),
            documents: HashMap::new(),
            index_status: HashMap::new(),
            last_index: Instant::now(),
            documents_gen: 0,
            doc_query: String::new(),
            doc_kind: None,
            selected_doc: None,
            pending_doc: None,
            doc_cache: None,
            settings_categories: false,
            labels_edited: None,
        };
        if demo {
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
                flagged: i == 3 || i == 5,
                size: 12_000 + (i as u32) * 4000,
                message_id: format!("<demo-{i}@example.it>"),
                snippet: text.chars().take(120).collect(),
                has_attachment: *att,
            };
            self.messages
                .entry(((*acc).into(), "INBOX".into()))
                .or_default()
                .push(summary);
            let body_text = format!(
                "{text}\n\nCordiali saluti,\n{}",
                if name.is_empty() { *addr } else { *name }
            );
            self.bodies.insert(
                ((*acc).into(), "INBOX".into(), uid),
                MessageBody {
                    to: vec!["e.mancinelli@example.it".into()],
                    doc: Arc::new(mailopen_core::html::from_plain(&body_text)),
                    text: body_text,
                    attachments: if *att {
                        vec![mailopen_core::Attachment {
                            name: "documento.pdf".into(),
                            mime: "application/pdf".into(),
                            content_id: None,
                            data: Arc::from(&b"%PDF-1.4"[..]),
                        }]
                    } else {
                        Vec::new()
                    },
                    message_id: format!("demo-{i}@example.it"),
                    ..MessageBody::default()
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
        if let Some(m) = self
            .summary(&("demo-a".into(), "INBOX".into(), 104))
            .cloned()
        {
            self.labels.toggle(&m, "progetto");
        }
        self.load_demo_documents(now);
        self.select_message(("demo-a".into(), "INBOX".into(), 101));
        self.set_status("Modalità demo: dati finti, nessuna connessione", false);
    }

    /// Fake document index for the demo (the Documenti view without a server).
    fn load_demo_documents(&mut self, now: i64) {
        type Doc = (
            &'static str,
            u32,
            &'static str,
            &'static str,
            &'static str,
            i64,
            &'static str,
        );
        let docs: &[Doc] = &[
            (
                "demo-a",
                100,
                "Offerta_DC_120kW_rev2.pdf",
                "application/pdf",
                "Re: RFQ for split-type DC charging stations",
                600,
                "Quotation 2026-118 split-type DC charging station 120 kW unit price EUR 18.900 delivery Q1 2027 warranty 36 months",
            ),
            (
                "demo-a",
                100,
                "Scheda_tecnica_DC120.pdf",
                "application/pdf",
                "Re: RFQ for split-type DC charging stations",
                600,
                "Technical data sheet DC120 input 400 V AC output 150-1000 V DC CCS2 OCPP 1.6J",
            ),
            (
                "demo-b",
                105,
                "Listino_wallbox_GSE.xlsx",
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
                "I: Wallbox in elenco GSE - listino a confronto",
                90000,
                "Modello Prezzo E-WALL GO 7,4 kW 890 E-WALL SMART 1.190 E-WALL EVO 22 kW 1.490 bonus colonnine domestiche",
            ),
            (
                "demo-b",
                103,
                "Verbale_WP5_ottobre.docx",
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                "Canceled: WP5 monthly call",
                7200,
                "Verbale riunione WP5 partecipanti azioni aperte consegna D5.2 entro dicembre",
            ),
            (
                "demo-a",
                106,
                "foto_installazione.jpg",
                "image/jpeg",
                "RE: DC charger received - installation",
                172800,
                "",
            ),
            (
                "demo-a",
                106,
                "DDT_4471.pdf",
                "application/pdf",
                "RE: DC charger received - installation",
                172800,
                "Documento di trasporto 4471 colli 2 peso 310 kg destinatario Rossi Srl Perugia",
            ),
            (
                "demo-a",
                102,
                "report_rimbalzi.csv",
                "text/csv",
                "ALERT rimbalzo mail: 2 non recapitate",
                3000,
                "indirizzo;errore mario.rossi@example.it;550 utente sconosciuto",
            ),
        ];
        let mut by_account: HashMap<String, Vec<Arc<DocEntry>>> = HashMap::new();
        let mut per_message: HashMap<u32, usize> = HashMap::new();
        for (i, (acc, uid, name, mime, subject, ago, text)) in docs.iter().enumerate() {
            let index = per_message.entry(*uid).or_default();
            let this_index = *index;
            *index += 1;
            let from = self
                .summary(&((*acc).into(), "INBOX".into(), *uid))
                .map(|m| {
                    (
                        m.from_name.clone(),
                        m.from_addr.clone(),
                        m.message_id.clone(),
                    )
                })
                .unwrap_or_default();
            let mut e = DocEntry {
                account_id: (*acc).into(),
                folder: "INBOX".into(),
                uid: *uid,
                message_id: from.2,
                from_name: from.0,
                from_addr: from.1,
                subject: (*subject).into(),
                timestamp: now - ago,
                index: this_index,
                name: (*name).into(),
                mime: (*mime).into(),
                size: 48_000 + (i as u64) * 91_000,
                text: (*text).into(),
                folded: String::new(),
            };
            e.folded = format!(
                "{}
{}
{} {}
{}",
                e.name, e.subject, e.from_name, e.from_addr, e.text
            )
            .to_lowercase();
            by_account
                .entry((*acc).into())
                .or_default()
                .push(Arc::new(e));
        }
        for (acc, list) in by_account {
            self.documents.insert(acc, Arc::new(list));
        }
        self.index_status.insert(
            "demo-a".into(),
            "Indicizzo i documenti di Posta in arrivo: 120 di 840".into(),
        );
        self.documents_gen = self.documents_gen.wrapping_add(1);
    }

    /// Read the password and start the worker; on failure remember why (shown in the sidebar).
    pub fn start_account(&mut self, account: &Account) {
        match secrets::load(&account.id) {
            Ok(password) => {
                let worker = AccountWorker::spawn(
                    account.clone(),
                    password.clone(),
                    self.events_tx.clone(),
                    Arc::clone(&self.wake),
                );
                worker.send(Request::ListFolders);
                self.workers.insert(account.id.clone(), worker);
                self.account_errors.remove(&account.id);
                if self.config.documents_index {
                    let indexer = Indexer::spawn(
                        account.clone(),
                        password,
                        self.events_tx.clone(),
                        Arc::clone(&self.wake),
                    );
                    self.indexers.insert(account.id.clone(), indexer);
                }
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

    /// Whether an account is shown by the rail selection.
    pub fn in_scope(&self, account: &str) -> bool {
        match &self.rail {
            Rail::All => true,
            Rail::Account(id) => id == account,
        }
    }

    /// The row of a message: from its folder's list, else from the rows known elsewhere.
    pub fn summary(&self, key: &MsgKey) -> Option<&MessageSummary> {
        let (acc, folder, uid) = key;
        self.messages
            .get(&(acc.clone(), folder.clone()))
            .and_then(|v| v.iter().find(|m| m.uid == *uid))
            .or_else(|| self.extra.get(key))
    }

    /// Loaded messages of the accounts in scope that are in a category, plus the ones classified
    /// by hand whose folder is not loaded.
    pub fn category_messages(&self, id: &str) -> Vec<MessageSummary> {
        let mut keys: HashSet<String> = HashSet::new();
        let mut out = Vec::new();
        let loaded = self
            .messages
            .iter()
            .filter(|((acc, _), _)| self.in_scope(acc))
            .flat_map(|(_, v)| v.iter())
            .chain(self.extra.values().filter(|m| self.in_scope(&m.account_id)));
        for m in loaded {
            if self.labels.has(m, id) && keys.insert(message_key(m)) {
                out.push(m.clone());
            }
        }
        for t in self.labels.added_to(id) {
            if self.in_scope(&t.account_id)
                && self.config.account(&t.account_id).is_some()
                && keys.insert(t.key.clone())
            {
                out.push(t.summary());
            }
        }
        out
    }

    /// Messages of the current view, newest first, filtered by the search box and the quick
    /// filter.
    pub fn visible_messages(&self) -> Vec<MessageSummary> {
        let mut out: Vec<MessageSummary> = match &self.view {
            View::Category(id) => self.category_messages(id),
            View::Search(_) => self
                .server_search
                .as_ref()
                .map(|s| {
                    s.results
                        .iter()
                        .filter_map(|k| self.summary(k).cloned())
                        .collect()
                })
                .unwrap_or_default(),
            View::Documents => Vec::new(),
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
        // Server results already match the query (often in the text, not in these fields).
        if !needle.is_empty() && !matches!(self.view, View::Search(_)) {
            out.retain(|m| {
                m.from_name.to_lowercase().contains(&needle)
                    || m.from_addr.to_lowercase().contains(&needle)
                    || m.subject.to_lowercase().contains(&needle)
                    || m.snippet.to_lowercase().contains(&needle)
                    || self
                        .labels
                        .of(m)
                        .iter()
                        .any(|c| c.name.to_lowercase().contains(&needle))
            });
        }
        let filter = self.filter;
        out.retain(|m| filter.keeps(m));
        out.sort_by(|a, b| {
            b.timestamp
                .cmp(&a.timestamp)
                .then_with(|| b.uid.cmp(&a.uid))
        });
        out
    }

    pub fn selected_summary(&self) -> Option<MessageSummary> {
        self.summary(self.selected.as_ref()?).cloned()
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
            // New mail lands in the inboxes; reload them so it gets classified.
            View::Category(_) => {
                for a in self.config.accounts.clone() {
                    if self.in_scope(&a.id) {
                        let folder = self.inbox_name(&a.id);
                        self.request(&a.id, Request::LoadMessages { folder, limit });
                    }
                }
            }
            View::Documents | View::Search(_) => {}
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
        let in_list = self
            .messages
            .get(&(acc.clone(), folder.clone()))
            .is_some_and(|v| v.iter().any(|m| m.uid == *uid));
        if in_list {
            self.messages
                .get_mut(&(acc.clone(), folder.clone()))?
                .iter_mut()
                .find(|m| m.uid == *uid)
        } else {
            self.extra.get_mut(key)
        }
    }

    /// Select a row that may live outside the loaded folders (search result, document, message
    /// classified by hand). A row whose uid is unknown (moved) is looked up by `Message-ID`.
    pub fn open_summary(&mut self, m: MessageSummary) {
        if m.uid == 0 {
            if m.message_id.trim().is_empty() {
                self.set_status("Messaggio non più raggiungibile", true);
                return;
            }
            self.pending_locate = Some(message_key(&m));
            self.request(
                &m.account_id,
                Request::Locate {
                    folder: m.folder.clone(),
                    message_id: m.message_id.clone(),
                },
            );
            self.set_status("Cerco il messaggio nella sua nuova cartella…", false);
            return;
        }
        let key = m.key();
        if self.summary(&key).is_none() {
            self.extra.insert(key.clone(), m);
        }
        self.select_message(key);
    }

    pub fn set_seen(&mut self, key: &MsgKey, seen: bool) {
        let Some(m) = self.summary_mut(key) else {
            return;
        };
        if m.seen == seen {
            return;
        }
        m.seen = seen;
        let (acc, folder, uid) = key.clone();
        self.request(&acc, Request::SetSeen { folder, uid, seen });
    }

    pub fn toggle_seen(&mut self, key: &MsgKey) {
        if let Some(seen) = self.summary(key).map(|m| m.seen) {
            self.set_seen(key, !seen);
        }
    }

    pub fn toggle_seen_selected(&mut self) {
        if let Some(key) = self.selected.clone() {
            self.toggle_seen(&key);
        }
    }

    /// Flag ("contrassegna") a message, or take the flag away.
    pub fn toggle_flag(&mut self, key: &MsgKey) {
        let Some(flagged) = self.summary_mut(key).map(|m| {
            m.flagged = !m.flagged;
            m.flagged
        }) else {
            return;
        };
        let (acc, folder, uid) = key.clone();
        self.request(
            &acc,
            Request::SetFlagged {
                folder,
                uid,
                flagged,
            },
        );
    }

    /// Put a message in a category, or take it out.
    pub fn toggle_category(&mut self, key: &MsgKey, category: &str) {
        let Some(m) = self.summary(key).cloned() else {
            return;
        };
        self.labels.toggle(&m, category);
        self.labels_dirty = true;
        self.save_labels();
    }

    /// The category list was edited (Impostazioni): saved a moment later, in one go.
    pub fn labels_changed(&mut self) {
        self.labels_dirty = true;
        self.labels_edited = Some(Instant::now());
        if let View::Category(id) = &self.view
            && self.labels.category(id).is_none()
        {
            self.view = View::Unified;
            self.rail = Rail::All;
        }
    }

    /// Write the categories file when it changed (never when it could not be read).
    pub fn save_labels(&mut self) {
        if !self.labels_dirty || self.labels_locked {
            return;
        }
        // Not retried every frame: the next change tries again.
        self.labels_dirty = false;
        if let Err(e) = self.labels.save() {
            self.set_status(format!("Categorie non salvate: {e}"), true);
        }
    }

    /// Take a message off the screen (it was deleted or moved).
    fn drop_message(&mut self, key: &MsgKey) {
        let (acc, folder, uid) = key;
        if let Some(list) = self.messages.get_mut(&(acc.clone(), folder.clone())) {
            list.retain(|m| m.uid != *uid);
        }
        self.extra.remove(key);
        self.bodies.remove(key);
        if let Some(s) = self.server_search.as_mut() {
            s.results.retain(|k| k != key);
        }
        if self.selected.as_ref() == Some(key) {
            self.selected = None;
        }
    }

    pub fn delete_message(&mut self, key: MsgKey) {
        let (acc, folder, uid) = key.clone();
        let trash = self.folder_of_kind(&acc, FolderKind::Trash);
        self.drop_message(&key);
        self.request(&acc, Request::Delete { folder, uid, trash });
    }

    /// Move a message to another folder of its account.
    pub fn move_message(&mut self, key: MsgKey, to: String) {
        let (acc, folder, uid) = key.clone();
        if folder == to {
            return;
        }
        if let Some(m) = self.summary(&key).cloned()
            && self.labels.moved(&m, &to)
        {
            self.labels_dirty = true;
            self.save_labels();
        }
        self.drop_message(&key);
        self.request(
            &acc,
            Request::Move {
                folder,
                uid,
                to: to.clone(),
            },
        );
        let label = self.folder_label(&acc, &to);
        self.set_status(format!("Spostato in {label}"), false);
    }

    /// The name a folder is shown with.
    pub fn folder_label(&self, account: &str, folder: &str) -> String {
        self.folders
            .get(account)
            .and_then(|fs| fs.iter().find(|f| f.name == folder))
            .map(|f| f.display())
            .unwrap_or_else(|| folder.to_string())
    }

    /// Folders a message of `account` can be moved to, as (name, label), standard ones first.
    pub fn move_targets(&self, account: &str, except: &str) -> Vec<(String, String)> {
        self.folders
            .get(account)
            .map(|fs| {
                fs.iter()
                    .filter(|f| f.selectable && f.name != except && f.kind != FolderKind::Drafts)
                    .map(|f| (f.name.clone(), f.display()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Folders searched on the server and indexed for documents: not Trash, Spam or Drafts.
    pub fn content_folders(&self, account: &str) -> Vec<String> {
        self.folders
            .get(account)
            .map(|fs| {
                fs.iter()
                    .filter(|f| {
                        f.selectable
                            && !matches!(
                                f.kind,
                                FolderKind::Trash | FolderKind::Spam | FolderKind::Drafts
                            )
                    })
                    .map(|f| f.name.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Search every folder of the accounts in scope on the servers (headers and text).
    pub fn start_server_search(&mut self) {
        let query = self.search.trim().to_string();
        if query.is_empty() {
            return;
        }
        let accounts: Vec<String> = self
            .config
            .accounts
            .iter()
            .filter(|a| self.in_scope(&a.id) && self.workers.contains_key(&a.id))
            .map(|a| a.id.clone())
            .collect();
        for id in &accounts {
            let folders = self.content_folders(id);
            self.request(
                id,
                Request::Search {
                    folders,
                    query: query.clone(),
                    limit: 100,
                },
            );
        }
        self.server_search = Some(SearchState {
            query: query.clone(),
            results: Vec::new(),
            pending: accounts.len(),
        });
        self.set_view(View::Search(query));
    }

    /// Start (or continue) the document index of an account.
    pub fn run_index(&mut self, account: &str) {
        let folders = self.content_folders(account);
        if folders.is_empty() {
            return;
        }
        if let Some(ix) = self.indexers.get(account) {
            ix.run(folders, self.config.documents_depth);
        }
    }

    pub fn run_all_indexes(&mut self) {
        let ids: Vec<String> = self.indexers.keys().cloned().collect();
        for id in ids {
            self.run_index(&id);
        }
        self.last_index = Instant::now();
    }

    /// Forget every index and build them again.
    pub fn rebuild_indexes(&mut self) {
        for ix in self.indexers.values() {
            ix.reset();
        }
        self.run_all_indexes();
        self.set_status("Indice dei documenti in ricostruzione", false);
    }

    /// Start or stop the indexers after the setting changed.
    pub fn apply_index_setting(&mut self) {
        if !self.config.documents_index {
            self.indexers.clear();
            self.index_status.clear();
            return;
        }
        if std::env::var_os("MAILOPEN_DEMO").is_some() {
            return;
        }
        for a in self.config.accounts.clone() {
            if self.indexers.contains_key(&a.id) || !self.workers.contains_key(&a.id) {
                continue;
            }
            if let Ok(password) = secrets::load(&a.id) {
                let ix = Indexer::spawn(
                    a.clone(),
                    password,
                    self.events_tx.clone(),
                    Arc::clone(&self.wake),
                );
                self.indexers.insert(a.id.clone(), ix);
                self.run_index(&a.id);
            }
        }
    }

    /// Documents of the accounts in scope matching the query and the type, newest first.
    /// Cached until one of them or an index changes.
    pub fn filtered_documents(&mut self) -> Vec<Arc<DocEntry>> {
        if let Some((q, k, r, g, list)) = &self.doc_cache
            && *q == self.doc_query
            && *k == self.doc_kind
            && *r == self.rail
            && *g == self.documents_gen
        {
            return list.clone();
        }
        let words = mailopen_core::documents::query_words(&self.doc_query);
        let kind = self.doc_kind;
        let mut list: Vec<Arc<DocEntry>> = self
            .documents
            .iter()
            .filter(|(acc, _)| self.in_scope(acc) && self.config.account(acc).is_some())
            .flat_map(|(_, l)| l.iter())
            .filter(|e| kind.is_none_or(|k| kind_group(e.kind()) == k) && e.matches(&words))
            .cloned()
            .collect();
        list.sort_by(|a, b| {
            b.timestamp
                .cmp(&a.timestamp)
                .then_with(|| a.name.cmp(&b.name))
        });
        self.doc_cache = Some((
            self.doc_query.clone(),
            kind,
            self.rail.clone(),
            self.documents_gen,
            list.clone(),
        ));
        list
    }

    /// Select the message a document came with; run `op` on the file once it is downloaded.
    pub fn open_document(&mut self, e: &DocEntry, op: Option<DocOp>) {
        let key: MsgKey = (e.account_id.clone(), e.folder.clone(), e.uid);
        self.selected_doc = Some((key.clone(), e.index));
        let summary = self
            .summary(&key)
            .cloned()
            .unwrap_or_else(|| doc_summary(e));
        self.open_summary(summary);
        if let Some(op) = op {
            if self.bodies.contains_key(&key) {
                self.run_doc_op(&key, e.index, op);
            } else {
                self.pending_doc = Some((key, e.index, op));
                self.set_status(format!("Scarico {}…", e.name), false);
            }
        }
    }

    /// Show a document's message in its folder.
    pub fn go_to_message(&mut self, e: &DocEntry) {
        let key: MsgKey = (e.account_id.clone(), e.folder.clone(), e.uid);
        let summary = self
            .summary(&key)
            .cloned()
            .unwrap_or_else(|| doc_summary(e));
        if let Rail::Account(id) = &self.rail
            && *id != e.account_id
        {
            self.rail = Rail::Account(e.account_id.clone());
        }
        self.set_view(View::Folder {
            account: e.account_id.clone(),
            folder: e.folder.clone(),
        });
        self.open_summary(summary);
    }

    fn run_doc_op(&mut self, key: &MsgKey, index: usize, op: DocOp) {
        let Some(a) = self
            .bodies
            .get(key)
            .and_then(|b| b.attachments.get(index))
            .cloned()
        else {
            self.set_status(
                "Allegato non trovato: il messaggio è cambiato, aggiorna l'indice dei documenti",
                true,
            );
            return;
        };
        match op {
            DocOp::Preview => self.open_attachment(Some(key.clone()), a),
            DocOp::Save => match crate::ui::files::save_dialog(&a) {
                Ok(Some(p)) => self.set_status(format!("Salvato in {}", p.display()), false),
                Ok(None) => {}
                Err(e) => self.set_status(e, true),
            },
            DocOp::OpenWith => {
                if let Err(e) = crate::ui::files::open_with_default(&a) {
                    self.set_status(e, true);
                }
            }
            DocOp::Print => {
                self.set_status(format!("Preparo la stampa di {}…", a.name), false);
                self.background_task = Some(crate::ui::files::print(a));
            }
            DocOp::Forward => self.forward_with(Some(key.clone()), vec![a]),
        }
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
        let idx = match (&self.view, &self.rail) {
            (View::Folder { account, .. }, _) | (_, Rail::Account(account)) => {
                self.account_index(account)
            }
            _ => 0,
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

    /// Forward the selected message with all its attachments.
    pub fn forward(&mut self) {
        let Some(key) = self.selected.clone() else {
            return;
        };
        let attachments = self
            .bodies
            .get(&key)
            .map(|b| b.attachments.clone())
            .unwrap_or_default();
        self.forward_with(Some(key), attachments);
    }

    /// Forward a message (when known) with these attachments: the quoted original, ready to send.
    pub fn forward_with(
        &mut self,
        key: Option<MsgKey>,
        attachments: Vec<mailopen_core::Attachment>,
    ) {
        let summary = key.as_ref().and_then(|(acc, folder, uid)| {
            self.messages
                .get(&(acc.clone(), folder.clone()))?
                .iter()
                .find(|m| m.uid == *uid)
                .cloned()
        });
        let body = key
            .as_ref()
            .and_then(|k| self.bodies.get(k))
            .cloned()
            .unwrap_or_default();
        let index = summary
            .as_ref()
            .map(|s| self.account_index(&s.account_id))
            .unwrap_or(0);
        let mut c = ComposeState::new(index);
        match &summary {
            Some(summary) => {
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
            }
            None => {
                c.subject = attachments
                    .first()
                    .map(|a| format!("Fwd: {}", a.name))
                    .unwrap_or_default();
                c.body = self.signature_block(c.account_index);
            }
        }
        c.attachments = attachments;
        c.note = self.html_signature_note(c.account_index);
        self.compose = Some(c);
    }

    /// Open a link from a message: `mailto:` starts a new message, the rest goes to the browser.
    pub fn open_link(&mut self, ctx: &egui::Context, url: &str) {
        if let Some(rest) = url
            .strip_prefix("mailto:")
            .or_else(|| url.strip_prefix("MAILTO:"))
        {
            let (addr, query) = rest.split_once('?').unwrap_or((rest, ""));
            self.compose_new();
            if let Some(c) = self.compose.as_mut() {
                c.to = percent_decode(addr);
                for pair in query.split('&') {
                    if let Some((k, v)) = pair.split_once('=')
                        && k.eq_ignore_ascii_case("subject")
                    {
                        c.subject = percent_decode(v);
                    }
                }
            }
            return;
        }
        ctx.open_url(egui::OpenUrl::new_tab(url));
    }

    /// Print the selected message (HTML as sent, with its inline images) through the browser.
    pub fn print_selected(&mut self) {
        let Some(summary) = self.selected_summary() else {
            return;
        };
        let Some(key) = self.selected.clone() else {
            return;
        };
        let Some(body) = self.bodies.get(&key) else {
            return;
        };
        let sender = summary.from_addr.to_lowercase();
        let allow_remote = self.remote_allowed.contains(&key)
            || self.config.trusted_image_senders.contains(&sender);
        let from = if summary.from_name.is_empty() {
            summary.from_addr.clone()
        } else {
            format!("{} <{}>", summary.from_name, summary.from_addr)
        };
        let mut header = vec![
            ("Da".to_string(), from),
            ("Data".to_string(), format::long_date(summary.timestamp)),
        ];
        if !body.to.is_empty() {
            header.push(("A".into(), body.to.join(", ")));
        }
        if !body.cc.is_empty() {
            header.push(("Cc".into(), body.cc.join(", ")));
        }
        if !body.attachments.is_empty() {
            let names: Vec<&str> = body.attachments.iter().map(|a| a.name.as_str()).collect();
            header.push(("Allegati".into(), names.join(", ")));
        }
        self.background_task = Some(crate::ui::files::print_message(
            &summary.subject,
            &header,
            body,
            allow_remote,
        ));
        self.set_status("Preparo la stampa…", false);
    }

    /// Show a raw message as if it were selected (offscreen screenshots of real mail).
    pub fn load_eml(&mut self, raw: &[u8]) {
        let body = mailopen_core::imap_client::parse_body(raw);
        let parsed = mail_header(raw);
        let acc = self
            .config
            .accounts
            .first()
            .map(|a| a.id.clone())
            .unwrap_or_else(|| "demo-a".into());
        let summary = MessageSummary {
            account_id: acc.clone(),
            folder: "INBOX".into(),
            uid: 9999,
            from_name: parsed.0,
            from_addr: parsed.1,
            subject: parsed.2,
            timestamp: chrono::Utc::now().timestamp(),
            seen: true,
            answered: false,
            flagged: false,
            size: u32::try_from(raw.len()).unwrap_or(u32::MAX),
            message_id: String::new(),
            snippet: mailopen_core::imap_client::preview_line(&body.text),
            has_attachment: !body.attachments.is_empty(),
        };
        self.messages
            .entry((acc.clone(), "INBOX".into()))
            .or_default()
            .insert(0, summary);
        self.bodies
            .insert((acc.clone(), "INBOX".into(), 9999), body);
        self.selected = None;
        self.select_message((acc, "INBOX".into(), 9999));
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
            attachments: c.attachments.clone(),
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
                message: Box::new(message),
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
        self.indexers.remove(&account.id);
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
        self.indexers.remove(id);
        self.documents.remove(id);
        self.index_status.remove(id);
        self.documents_gen = self.documents_gen.wrapping_add(1);
        DocIndex::remove(id);
        self.config.remove(id);
        self.folders.remove(id);
        self.messages.retain(|(acc, _), _| acc != id);
        self.extra.retain(|(acc, _, _), _| acc != id);
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
                self.run_index(&id);
            }
            Event::Messages {
                folder,
                mut messages,
            } => {
                // The open message stays on screen even when it is older than the list.
                if let Some(sel) = self.selected.clone()
                    && sel.0 == id
                    && sel.1 == folder
                    && !messages.iter().any(|m| m.uid == sel.2)
                    && let Some(old) = self
                        .messages
                        .get(&(id.clone(), folder.clone()))
                        .and_then(|v| v.iter().find(|m| m.uid == sel.2))
                        .cloned()
                {
                    self.extra.insert(sel, old);
                }
                for m in &mut messages {
                    self.contacts_dirty |=
                        self.contacts
                            .remember(&m.from_name, &m.from_addr, m.timestamp);
                    // Keep a preview filled in from the opened message across refreshes.
                    if m.snippet.trim().is_empty()
                        && let Some(b) = self.bodies.get(&(id.clone(), folder.clone(), m.uid))
                    {
                        m.snippet = mailopen_core::imap_client::preview_line(&b.text);
                    }
                }
                self.messages.insert((id, folder), messages);
            }
            Event::Body { folder, uid, body } => {
                let now = chrono::Utc::now().timestamp();
                self.contacts_dirty |= self.contacts.remember_list(&body.to, now);
                self.contacts_dirty |= self.contacts.remember_list(&body.cc, now);
                let key = (id, folder, uid);
                // A preview the first bytes could not give comes from the whole text.
                let preview = mailopen_core::imap_client::preview_line(&body.text);
                if let Some(m) = self.summary_mut(&key)
                    && m.snippet.trim().is_empty()
                {
                    m.snippet = preview;
                }
                self.bodies.insert(key.clone(), *body);
                if let Some((k, index, op)) = self.pending_doc.take() {
                    if k == key {
                        self.run_doc_op(&key, index, op);
                    } else {
                        self.pending_doc = Some((k, index, op));
                    }
                }
            }
            Event::SeenChanged { folder, uid, seen } => {
                if let Some(m) = self.summary_mut(&(id, folder, uid)) {
                    m.seen = seen;
                }
            }
            Event::FlaggedChanged {
                folder,
                uid,
                flagged,
            } => {
                if let Some(m) = self.summary_mut(&(id, folder, uid)) {
                    m.flagged = flagged;
                }
            }
            Event::Moved { to, .. } => {
                // The target folder shows the message on its next load.
                let limit = self.config.messages_per_folder;
                if matches!(&self.view, View::Folder { account, folder } if *account == id && *folder == to)
                {
                    self.request(&id, Request::LoadMessages { folder: to, limit });
                }
            }
            Event::SearchResults { query, messages } => {
                if let Some(mut s) = self.server_search.take() {
                    if s.query == query {
                        s.pending = s.pending.saturating_sub(1);
                        for m in messages {
                            let key = m.key();
                            if self.summary(&key).is_none() {
                                self.extra.insert(key.clone(), m);
                            }
                            if !s.results.contains(&key) {
                                s.results.push(key);
                            }
                        }
                    }
                    self.server_search = Some(s);
                }
            }
            Event::Located {
                message_id,
                summary,
            } => {
                let key = mailopen_core::labels::id_key(&message_id);
                if self.pending_locate.as_deref() != Some(key.as_str()) {
                    return;
                }
                self.pending_locate = None;
                match summary {
                    Some(s) => {
                        if let Some(t) = self.labels.tagged.iter_mut().find(|t| t.key == key) {
                            t.folder = s.folder.clone();
                            t.uid = s.uid;
                            self.labels_dirty = true;
                        }
                        self.save_labels();
                        self.status = None;
                        self.open_summary(*s);
                    }
                    None => self.set_status(
                        "Messaggio non trovato: forse è stato spostato o eliminato da un altro programma",
                        true,
                    ),
                }
            }
            Event::Documents { entries, status } => {
                self.documents.insert(id.clone(), entries);
                self.documents_gen = self.documents_gen.wrapping_add(1);
                match status {
                    Some(s) => {
                        self.index_status.insert(id, s);
                    }
                    None => {
                        self.index_status.remove(&id);
                    }
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
                    self.pending_doc = None;
                    self.pending_locate = None;
                } else if what == "Ricerca sul server"
                    && let Some(s) = self.server_search.as_mut()
                {
                    s.pending = s.pending.saturating_sub(1);
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
        let (del, cmd_n, cmd_r, cmd_u, cmd_f, step) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace),
                i.modifiers.command && i.key_pressed(egui::Key::N),
                i.modifiers.command && i.key_pressed(egui::Key::R),
                i.modifiers.command && i.key_pressed(egui::Key::U),
                i.modifiers.command && i.key_pressed(egui::Key::F),
                if i.key_pressed(egui::Key::ArrowDown) {
                    1i32
                } else if i.key_pressed(egui::Key::ArrowUp) {
                    -1
                } else {
                    0
                },
            )
        });
        if self.compose.is_some() || self.account_dialog.is_some() {
            return;
        }
        // In Documenti a row is a file: Canc must not throw away the whole message.
        if del && self.view != View::Documents {
            self.delete_selected();
        }
        if cmd_n {
            self.compose_new();
        }
        if cmd_r && self.selected.is_some() {
            self.reply(false);
        }
        if cmd_u {
            self.toggle_seen_selected();
        }
        if cmd_f {
            if self.view == View::Documents {
                // The Documenti view has its own search box, always visible.
                ctx.memory_mut(|m| m.request_focus(crate::ui::documents::search_id()));
            } else {
                self.search_focus = true;
                self.search_open = true;
            }
        }
        if step != 0 {
            self.step_selection(step);
        }
    }

    /// Select the next (1) or previous (-1) row of the list on screen.
    fn step_selection(&mut self, step: i32) {
        if self.view == View::Documents {
            let docs = self.filtered_documents();
            let at = docs.iter().position(|e| {
                self.selected_doc.as_ref()
                    == Some(&((e.account_id.clone(), e.folder.clone(), e.uid), e.index))
            });
            let next = match (at, step > 0) {
                (None, _) => 0,
                (Some(i), true) => i.saturating_add(1),
                (Some(i), false) => i.saturating_sub(1),
            };
            if let Some(e) = docs.get(next) {
                let e = Arc::clone(e);
                self.open_document(&e, None);
            }
            return;
        }
        let list = self.visible_messages();
        let at = self
            .selected
            .as_ref()
            .and_then(|k| list.iter().position(|m| m.key() == *k));
        let next = match (at, step > 0) {
            (None, _) => 0,
            (Some(i), true) => i.saturating_add(1),
            (Some(i), false) => i.saturating_sub(1),
        };
        if let Some(m) = list.get(next) {
            self.open_summary(m.clone());
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
        self.images.poll(&ctx);
        if let Some(rx) = &self.background_task
            && let Ok(result) = rx.try_recv()
        {
            self.background_task = None;
            match result {
                Ok(s) => self.set_status(s, false),
                Err(e) => self.set_status(e, true),
            }
        }
        let ctx = &ctx;
        self.handle_events();
        if self.last_refresh.elapsed() >= Duration::from_secs(self.config.refresh_seconds) {
            self.refresh_view();
        }
        // New mail gets into the document index every quarter of an hour.
        if self.last_index.elapsed() >= Duration::from_secs(15 * 60) {
            self.run_all_indexes();
        }
        if self.labels_dirty
            && self
                .labels_edited
                .is_none_or(|t| t.elapsed() >= Duration::from_secs(1))
        {
            self.labels_edited = None;
            self.save_labels();
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
        self.viewer_window(ctx);
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

/// The type filters of the Documenti view: messages, audio and video count as "Altro".
pub fn kind_group(k: mailopen_core::FileKind) -> mailopen_core::FileKind {
    use mailopen_core::FileKind as K;
    match k {
        K::Message | K::Audio | K::Video | K::Other => K::Other,
        k => k,
    }
}

/// A list row for a document's message, when its folder is not loaded.
fn doc_summary(e: &DocEntry) -> MessageSummary {
    MessageSummary {
        account_id: e.account_id.clone(),
        folder: e.folder.clone(),
        uid: e.uid,
        from_name: e.from_name.clone(),
        from_addr: e.from_addr.clone(),
        subject: e.subject.clone(),
        timestamp: e.timestamp,
        seen: true,
        answered: false,
        flagged: false,
        size: 0,
        message_id: e.message_id.clone(),
        snippet: String::new(),
        has_attachment: true,
    }
}

fn percent_decode(s: &str) -> String {
    String::from_utf8_lossy(&mailopen_core::html::percent_decode(s)).into_owned()
}

/// Sender name, address and subject of a raw message.
fn mail_header(raw: &[u8]) -> (String, String, String) {
    mailopen_core::imap_client::sender_and_subject(raw)
}
