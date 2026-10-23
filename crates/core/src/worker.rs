//! One background thread per account. The GUI sends [`Request`]s and receives
//! [`Event`]s on a shared channel; every event carries the account id.
//! The thread owns the IMAP session and reconnects once when a call fails.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

use crate::config::Account;
use crate::documents::DocList;
use crate::error::{Error, Result};
use crate::imap_client::ImapClient;
use crate::model::{Folder, MessageBody, MessageSummary, OutgoingMessage};
use crate::smtp_client;

#[derive(Clone, Debug)]
pub enum Request {
    ListFolders,
    LoadMessages {
        folder: String,
        limit: u32,
    },
    LoadBody {
        folder: String,
        uid: u32,
    },
    SetSeen {
        folder: String,
        uid: u32,
        seen: bool,
    },
    SetFlagged {
        folder: String,
        uid: u32,
        flagged: bool,
    },
    Move {
        folder: String,
        uid: u32,
        to: String,
    },
    /// Search these folders on the server; at most `limit` results per folder.
    Search {
        folders: Vec<String>,
        query: String,
        limit: u32,
    },
    /// Find a message by `Message-ID` (its uid is not known, e.g. after a move).
    Locate {
        folder: String,
        message_id: String,
    },
    Delete {
        folder: String,
        uid: u32,
        trash: Option<String>,
    },
    Send {
        message: Box<OutgoingMessage>,
        sent_folder: Option<String>,
    },
    Ping,
    Shutdown,
}

#[derive(Clone, Debug)]
pub enum Event {
    Folders(Vec<Folder>),
    Messages {
        folder: String,
        messages: Vec<MessageSummary>,
    },
    Body {
        folder: String,
        uid: u32,
        body: Box<MessageBody>,
    },
    SeenChanged {
        folder: String,
        uid: u32,
        seen: bool,
    },
    FlaggedChanged {
        folder: String,
        uid: u32,
        flagged: bool,
    },
    Moved {
        folder: String,
        uid: u32,
        to: String,
    },
    SearchResults {
        query: String,
        messages: Vec<MessageSummary>,
    },
    Located {
        message_id: String,
        summary: Option<Box<MessageSummary>>,
    },
    /// The document index of the account (from the indexer thread), with what it is doing.
    Documents {
        entries: DocList,
        status: Option<String>,
    },
    Deleted {
        folder: String,
        uid: u32,
    },
    Sent,
    /// The account's credentials worked (used by the add-account dialog).
    LoginOk,
    /// The request failed; `what` is a short label for the status bar.
    Failed {
        what: String,
        error: String,
    },
    /// Number of requests still queued or running.
    Busy(usize),
}

/// Callback the worker calls after every event so the GUI repaints.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

pub struct AccountWorker {
    tx: Sender<Request>,
    pending: Arc<std::sync::atomic::AtomicUsize>,
}

impl AccountWorker {
    pub fn spawn(
        account: Account,
        password: String,
        events: Sender<(String, Event)>,
        wake: Wake,
    ) -> AccountWorker {
        let (tx, rx) = channel::<Request>();
        let pending = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let pending_thread = Arc::clone(&pending);
        let name = format!("mailopen-{}", account.email);
        let spawned = thread::Builder::new().name(name).spawn(move || {
            run(account, password, rx, events, wake, pending_thread);
        });
        if let Err(e) = spawned {
            log::error!("worker thread: {e}");
        }
        AccountWorker { tx, pending }
    }

    pub fn send(&self, req: Request) {
        self.pending
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if self.tx.send(req).is_err() {
            self.pending
                .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            log::warn!("worker gone");
        }
    }

    pub fn pending(&self) -> usize {
        self.pending.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl Drop for AccountWorker {
    fn drop(&mut self) {
        let _ = self.tx.send(Request::Shutdown);
    }
}

/// Run `op` on a live session, reconnecting once if the session is dead. A session that failed
/// or went out of step is never used again: the IMAP library panics when replies and commands
/// no longer match, so such a panic is caught here and costs only the connection.
pub(crate) fn with_session<T>(
    account: &Account,
    password: &str,
    session: &mut Option<ImapClient>,
    op: impl Fn(&mut ImapClient) -> Result<T>,
) -> Result<T> {
    for attempt in 0..2 {
        if session.is_none() {
            *session = Some(ImapClient::connect(account, password)?);
        }
        let Some(client) = session.as_mut() else {
            return Err(Error::other("sessione assente"));
        };
        let outcome = catch_unwind(AssertUnwindSafe(|| op(client)))
            .unwrap_or_else(|_| Err(Error::Imap("risposta del server non valida".into())));
        let broken = session.as_ref().is_none_or(|c| c.broken);
        match outcome {
            Ok(v) => {
                if broken {
                    *session = None;
                }
                return Ok(v);
            }
            Err(e) if attempt == 0 => {
                log::warn!("{}: {e}; riconnetto", account.email);
                *session = None;
            }
            Err(e) => {
                *session = None;
                return Err(e);
            }
        }
    }
    Err(Error::other("connessione IMAP non disponibile"))
}

fn run(
    account: Account,
    password: String,
    rx: Receiver<Request>,
    events: Sender<(String, Event)>,
    wake: Wake,
    pending: Arc<std::sync::atomic::AtomicUsize>,
) {
    let id = account.id.clone();
    let mut session: Option<ImapClient> = None;
    let emit = |ev: Event| {
        let _ = events.send((id.clone(), ev));
        wake();
    };
    while let Ok(req) = rx.recv() {
        emit(Event::Busy(
            pending.load(std::sync::atomic::Ordering::SeqCst),
        ));
        let what = label(&req);
        let result: Result<Event> = match req {
            Request::Shutdown => break,
            Request::Ping => with_session(&account, &password, &mut session, |c| c.noop())
                .map(|_| Event::LoginOk),
            Request::ListFolders => {
                with_session(&account, &password, &mut session, |c| c.folders()).map(Event::Folders)
            }
            Request::LoadMessages { folder, limit } => {
                with_session(&account, &password, &mut session, |c| {
                    c.summaries(&folder, limit)
                })
                .map(|messages| Event::Messages {
                    folder: folder.clone(),
                    messages,
                })
            }
            Request::LoadBody { folder, uid } => {
                with_session(&account, &password, &mut session, |c| c.body(&folder, uid)).map(
                    |body| Event::Body {
                        folder: folder.clone(),
                        uid,
                        body: Box::new(body),
                    },
                )
            }
            Request::SetSeen { folder, uid, seen } => {
                with_session(&account, &password, &mut session, |c| {
                    c.set_seen(&folder, uid, seen)
                })
                .map(|_| Event::SeenChanged {
                    folder: folder.clone(),
                    uid,
                    seen,
                })
            }
            Request::SetFlagged {
                folder,
                uid,
                flagged,
            } => with_session(&account, &password, &mut session, |c| {
                c.set_flagged(&folder, uid, flagged)
            })
            .map(|_| Event::FlaggedChanged {
                folder: folder.clone(),
                uid,
                flagged,
            }),
            Request::Move { folder, uid, to } => {
                with_session(&account, &password, &mut session, |c| {
                    c.move_to(&folder, uid, &to)
                })
                .map(|_| Event::Moved {
                    folder: folder.clone(),
                    uid,
                    to: to.clone(),
                })
            }
            Request::Search {
                folders,
                query,
                limit,
            } => {
                let mut messages = Vec::new();
                let mut last_error = None;
                for folder in &folders {
                    match with_session(&account, &password, &mut session, |c| {
                        c.search(folder, &query, limit)
                    }) {
                        Ok(found) => messages.extend(found),
                        Err(e) => {
                            log::warn!("{}: ricerca in {folder}: {e}", account.email);
                            last_error = Some(e);
                        }
                    }
                }
                match last_error {
                    Some(e) if messages.is_empty() => Err(e),
                    _ => Ok(Event::SearchResults {
                        query: query.clone(),
                        messages,
                    }),
                }
            }
            Request::Locate { folder, message_id } => {
                with_session(&account, &password, &mut session, |c| {
                    c.find_by_message_id(&folder, &message_id)
                })
                .map(|found| Event::Located {
                    message_id: message_id.clone(),
                    summary: found.map(Box::new),
                })
            }
            Request::Delete { folder, uid, trash } => {
                with_session(&account, &password, &mut session, |c| {
                    c.delete(&folder, uid, trash.as_deref())
                })
                .map(|_| Event::Deleted {
                    folder: folder.clone(),
                    uid,
                })
            }
            Request::Send {
                message,
                sent_folder,
            } => {
                // Sending never waits behind a slow IMAP call: own thread, own connection.
                let acc = account.clone();
                let pw = password.clone();
                let events = events.clone();
                let wake = Arc::clone(&wake);
                let pending_send = Arc::clone(&pending);
                let spawned =
                    thread::Builder::new()
                        .name("mailopen-send".into())
                        .spawn(move || {
                            let result =
                                send(&acc, &pw, &mut None, &message, sent_folder.as_deref());
                            pending_send.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                            let ev = match result {
                                Ok(ev) => ev,
                                Err(e) => {
                                    log::error!("{}: Invio: {e}", acc.email);
                                    Event::Failed {
                                        what: "Invio".into(),
                                        error: e.to_string(),
                                    }
                                }
                            };
                            let _ = events.send((acc.id.clone(), ev));
                            wake();
                        });
                if let Err(e) = spawned {
                    pending.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                    emit(Event::Failed {
                        what,
                        error: format!("thread di invio: {e}"),
                    });
                }
                continue;
            }
        };
        pending.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        match result {
            Ok(ev) => emit(ev),
            Err(e) => {
                log::error!("{}: {what}: {e}", account.email);
                emit(Event::Failed {
                    what,
                    error: e.to_string(),
                });
            }
        }
        emit(Event::Busy(
            pending.load(std::sync::atomic::Ordering::SeqCst),
        ));
    }
    if let Some(s) = session.take() {
        s.logout();
    }
}

fn send(
    account: &Account,
    password: &str,
    session: &mut Option<ImapClient>,
    message: &OutgoingMessage,
    sent_folder: Option<&str>,
) -> Result<Event> {
    let built = smtp_client::build(account, message)?;
    smtp_client::send(account, password, &built)?;
    if let Some(folder) = sent_folder {
        let raw = built.formatted();
        if let Err(e) = with_session(account, password, session, |c| c.append_sent(folder, &raw)) {
            // The mail went out; a missing copy in Sent is a warning, not a failure.
            log::warn!("{}: copia in {folder} non salvata: {e}", account.email);
        }
    }
    Ok(Event::Sent)
}

fn label(req: &Request) -> String {
    match req {
        Request::ListFolders => "Lettura cartelle".into(),
        Request::LoadMessages { folder, .. } => format!("Lettura di {folder}"),
        Request::LoadBody { .. } => "Apertura messaggio".into(),
        Request::SetSeen { .. } => "Segna letto/non letto".into(),
        Request::SetFlagged { .. } => "Contrassegno".into(),
        Request::Move { .. } => "Spostamento".into(),
        Request::Search { .. } => "Ricerca sul server".into(),
        Request::Locate { .. } => "Apertura messaggio".into(),
        Request::Delete { .. } => "Eliminazione".into(),
        Request::Send { .. } => "Invio".into(),
        Request::Ping => "Accesso".into(),
        Request::Shutdown => "Chiusura".into(),
    }
}
