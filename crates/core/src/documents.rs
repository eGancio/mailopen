//! The document index: every file attached to an account's mail (not only the messages on
//! screen), with the text inside PDF, Word, Excel and text files, so the "Documenti" view can
//! filter by type and search by name, sender, subject and content.
//!
//! A thread of its own with its own IMAP connection builds it, so the reading pane never waits.
//! Each run only looks at messages it has not seen; messages that left a folder drop out.
//! Saved as JSON per account in `documents/` next to the config.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::thread;

use serde::{Deserialize, Serialize};

use crate::config::{Account, Config};
use crate::error::{Error, Result};
use crate::imap_client::{self, ImapClient};
use crate::model::{FileKind, file_kind};
use crate::worker::{Event, Wake, with_session};

/// Messages fetched together: at most this many, and at most [`BATCH_BYTES`].
const BATCH: usize = 25;
const BATCH_BYTES: u64 = 24 * 1024 * 1024;
/// Messages bigger than this are skipped (and not asked for again).
const MAX_MESSAGE: u32 = 30 * 1024 * 1024;
/// Save the index after this many new messages, so a crash or a quit loses little.
const SAVE_EVERY: usize = 150;

/// One attached file.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DocEntry {
    pub account_id: String,
    pub folder: String,
    pub uid: u32,
    #[serde(default)]
    pub message_id: String,
    #[serde(default)]
    pub from_name: String,
    #[serde(default)]
    pub from_addr: String,
    #[serde(default)]
    pub subject: String,
    /// Unix seconds (UTC) of the message.
    #[serde(default)]
    pub timestamp: i64,
    /// Position among the message's attachments, as [`imap_client::parse_body`] lists them.
    pub index: usize,
    pub name: String,
    #[serde(default)]
    pub mime: String,
    #[serde(default)]
    pub size: u64,
    /// Text inside the file (capped); empty for images, archives and unreadable files.
    #[serde(default)]
    pub text: String,
    /// Lowercased name, subject, sender and text: what the search looks in. Not saved.
    #[serde(skip)]
    pub folded: String,
}

impl DocEntry {
    pub fn kind(&self) -> FileKind {
        file_kind(&self.name, &self.mime)
    }

    fn fold(&mut self) {
        self.folded = format!(
            "{}\n{}\n{} {}\n{}",
            self.name, self.subject, self.from_name, self.from_addr, self.text
        )
        .to_lowercase();
    }

    /// Whether every word of the (lowercased) query is somewhere in the entry.
    pub fn matches(&self, words: &[String]) -> bool {
        words.iter().all(|w| self.folded.contains(w.as_str()))
    }

    /// A short piece of the file's text around the first word found in it, for the list.
    pub fn excerpt(&self, words: &[String], width: usize) -> Option<String> {
        let lower = self.text.to_lowercase();
        // Lowercasing can change byte lengths; work in characters.
        let chars: Vec<char> = self.text.chars().collect();
        let lower_chars: Vec<char> = lower.chars().collect();
        if chars.len() != lower_chars.len() {
            return None;
        }
        let at = words.iter().find_map(|w| {
            let i = lower.find(w.as_str())?;
            Some(lower.get(..i)?.chars().count())
        })?;
        let start = at.saturating_sub(width / 3);
        let end = start.saturating_add(width).min(chars.len());
        let mut out: String = chars.get(start..end)?.iter().collect();
        if start > 0 {
            out.insert(0, '…');
        }
        if end < chars.len() {
            out.push('…');
        }
        Some(out)
    }
}

/// The entries of one account, shared with the GUI without copying.
pub type DocList = Arc<Vec<Arc<DocEntry>>>;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct FolderState {
    #[serde(default)]
    uid_validity: u32,
    /// Messages already looked at.
    #[serde(default)]
    scanned: Vec<u32>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DocIndex {
    #[serde(default)]
    folders: BTreeMap<String, FolderState>,
    #[serde(default)]
    entries: Vec<Arc<DocEntry>>,
}

impl DocIndex {
    pub fn dir() -> Result<PathBuf> {
        Ok(Config::dir()?.join("documents"))
    }

    fn path(account_id: &str) -> Result<PathBuf> {
        let safe: String = account_id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        Ok(Self::dir()?.join(format!("{safe}.json")))
    }

    /// The saved index, or an empty one (a corrupt file is rebuilt, it is only a cache).
    pub fn load(account_id: &str) -> DocIndex {
        let Ok(path) = Self::path(account_id) else {
            return DocIndex::default();
        };
        let Ok(bytes) = std::fs::read(&path) else {
            return DocIndex::default();
        };
        match serde_json::from_slice::<DocIndex>(&bytes) {
            Ok(mut index) => {
                for e in &mut index.entries {
                    if let Some(e) = Arc::get_mut(e) {
                        e.fold();
                    }
                }
                index
            }
            Err(e) => {
                log::warn!(
                    "{}: indice documenti illeggibile, lo ricostruisco: {e}",
                    path.display()
                );
                DocIndex::default()
            }
        }
    }

    pub fn save(&self, account_id: &str) -> Result<()> {
        let path = Self::path(account_id)?;
        let dir = Self::dir()?;
        std::fs::create_dir_all(&dir)?;
        let tmp = path.with_extension("json.tmp");
        let bytes = serde_json::to_vec(self).map_err(|e| Error::other(e.to_string()))?;
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    pub fn entries(&self) -> DocList {
        Arc::new(self.entries.clone())
    }

    /// Delete the saved index of an account (when the account goes away).
    pub fn remove(account_id: &str) {
        if let Ok(p) = Self::path(account_id) {
            let _ = std::fs::remove_file(p);
        }
    }
}

enum Job {
    Run { folders: Vec<String>, depth: u32 },
    Reset,
}

/// The indexing thread of one account.
pub struct Indexer {
    tx: Sender<Job>,
    stop: Arc<AtomicBool>,
}

impl Indexer {
    /// Load the saved index (sent to the GUI right away) and wait for runs.
    pub fn spawn(
        account: Account,
        password: String,
        events: Sender<(String, Event)>,
        wake: Wake,
    ) -> Indexer {
        let (tx, rx) = channel::<Job>();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);
        let name = format!("mailopen-index-{}", account.email);
        let spawned = thread::Builder::new().name(name).spawn(move || {
            let mut worker = IndexWorker {
                index: DocIndex::load(&account.id),
                account,
                password,
                session: None,
                events,
                wake,
                stop: stop_thread,
            };
            worker.emit(None);
            worker.serve(&rx);
            if let Some(s) = worker.session.take() {
                s.logout();
            }
        });
        if let Err(e) = spawned {
            log::error!("indicizzatore: {e}");
        }
        Indexer { tx, stop }
    }

    /// Look at the newest `depth` messages of each folder (only the ones not seen yet).
    pub fn run(&self, folders: Vec<String>, depth: u32) {
        let _ = self.tx.send(Job::Run { folders, depth });
    }

    /// Forget everything; the next run starts from scratch.
    pub fn reset(&self) {
        let _ = self.tx.send(Job::Reset);
    }
}

impl Drop for Indexer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

struct IndexWorker {
    account: Account,
    password: String,
    session: Option<ImapClient>,
    index: DocIndex,
    events: Sender<(String, Event)>,
    wake: Wake,
    stop: Arc<AtomicBool>,
}

impl IndexWorker {
    fn emit(&self, status: Option<String>) {
        let _ = self.events.send((
            self.account.id.clone(),
            Event::Documents {
                entries: self.index.entries(),
                status,
            },
        ));
        (self.wake)();
    }

    fn stopped(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    fn save(&self) {
        if let Err(e) = self.index.save(&self.account.id) {
            log::warn!("{}: indice documenti non salvato: {e}", self.account.email);
        }
    }

    fn serve(&mut self, rx: &Receiver<Job>) {
        while let Ok(first) = rx.recv() {
            // Runs queued while busy collapse into the latest one.
            let mut job = first;
            loop {
                match rx.try_recv() {
                    Ok(next) => job = next,
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return,
                }
            }
            if self.stopped() {
                return;
            }
            match job {
                Job::Reset => {
                    self.index = DocIndex::default();
                    self.save();
                    self.emit(None);
                }
                Job::Run { folders, depth } => {
                    self.run(&folders, depth);
                    self.save();
                    self.emit(None);
                }
            }
        }
    }

    fn run(&mut self, folders: &[String], depth: u32) {
        let mut since_save = 0usize;
        for folder in folders {
            if self.stopped() {
                return;
            }
            let (account, password) = (self.account.clone(), self.password.clone());
            let listed = with_session(&account, &password, &mut self.session, |c| {
                c.all_uids(folder)
            });
            let (validity, uids) = match listed {
                Ok(v) => v,
                Err(e) => {
                    log::warn!("{}: indice di {folder}: {e}", self.account.email);
                    continue;
                }
            };
            let todo = self.prune(folder, validity, &uids, depth);
            let total = todo.len();
            let mut done = 0usize;
            let label = folder_label(folder);
            for chunk in todo.chunks(BATCH.saturating_mul(2)) {
                if self.stopped() {
                    return;
                }
                self.emit(Some(format!(
                    "Indicizzo i documenti di {label}: {done} di {total}"
                )));
                let headers = with_session(&account, &password, &mut self.session, |c| {
                    c.headers(folder, chunk)
                });
                let headers = match headers {
                    Ok(h) => h,
                    Err(e) => {
                        log::warn!("{}: intestazioni di {folder}: {e}", self.account.email);
                        break;
                    }
                };
                let mut scanned: Vec<u32> = Vec::new();
                let mut wanted: Vec<(u32, u32, Vec<u8>)> = Vec::new();
                for (uid, size, header) in headers {
                    if size <= MAX_MESSAGE && imap_client::may_have_files(&header) {
                        wanted.push((uid, size, header));
                    } else {
                        scanned.push(uid);
                    }
                }
                let mut fresh: Vec<Arc<DocEntry>> = Vec::new();
                for batch in batches(&wanted) {
                    if self.stopped() {
                        return;
                    }
                    let uids: Vec<u32> = batch.iter().map(|(u, _, _)| *u).collect();
                    let raws = with_session(&account, &password, &mut self.session, |c| {
                        c.raws(folder, &uids)
                    });
                    let raws = match raws {
                        Ok(r) => r,
                        Err(e) => {
                            log::warn!("{}: messaggi di {folder}: {e}", self.account.email);
                            continue;
                        }
                    };
                    for (uid, raw) in raws {
                        let header = batch
                            .iter()
                            .find(|(u, _, _)| *u == uid)
                            .map(|(_, _, h)| h.as_slice())
                            .unwrap_or(b"");
                        fresh.extend(entries_of(&account.id, folder, uid, header, &raw));
                        scanned.push(uid);
                    }
                }
                done = done.saturating_add(chunk.len());
                since_save = since_save.saturating_add(chunk.len());
                self.index.entries.extend(fresh);
                if let Some(state) = self.index.folders.get_mut(folder) {
                    state.scanned.extend(scanned);
                }
                if since_save >= SAVE_EVERY {
                    self.save();
                    since_save = 0;
                }
            }
        }
    }

    /// Drop what left the folder (or everything, when the server renumbered it) and return the
    /// newest `depth` UIDs not looked at yet, newest first.
    fn prune(&mut self, folder: &str, validity: u32, uids: &[u32], depth: u32) -> Vec<u32> {
        let state = self.index.folders.entry(folder.to_string()).or_default();
        if state.uid_validity != validity {
            state.uid_validity = validity;
            state.scanned.clear();
            self.index.entries.retain(|e| e.folder != folder);
        }
        let present: HashSet<u32> = uids.iter().copied().collect();
        let state = self.index.folders.entry(folder.to_string()).or_default();
        state.scanned.retain(|u| present.contains(u));
        let seen: HashSet<u32> = state.scanned.iter().copied().collect();
        self.index
            .entries
            .retain(|e| e.folder != folder || present.contains(&e.uid));
        uids.iter()
            .rev()
            .take(usize::try_from(depth).unwrap_or(usize::MAX))
            .filter(|u| !seen.contains(u))
            .copied()
            .collect()
    }
}

/// Groups of at most [`BATCH`] messages and [`BATCH_BYTES`].
fn batches(list: &[(u32, u32, Vec<u8>)]) -> Vec<&[(u32, u32, Vec<u8>)]> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut bytes = 0u64;
    for (i, (_, size, _)) in list.iter().enumerate() {
        let n = i.saturating_sub(start);
        if n > 0 && (n >= BATCH || bytes.saturating_add(u64::from(*size)) > BATCH_BYTES) {
            if let Some(b) = list.get(start..i) {
                out.push(b);
            }
            start = i;
            bytes = 0;
        }
        bytes = bytes.saturating_add(u64::from(*size));
    }
    if let Some(b) = list.get(start..).filter(|b| !b.is_empty()) {
        out.push(b);
    }
    out
}

/// One entry per attached file of a raw message.
pub fn entries_of(
    account_id: &str,
    folder: &str,
    uid: u32,
    header: &[u8],
    raw: &[u8],
) -> Vec<Arc<DocEntry>> {
    let info = imap_client::header_info(if header.is_empty() { raw } else { header });
    let body = imap_client::parse_body(raw);
    body.attachments
        .iter()
        .enumerate()
        .map(|(index, a)| {
            let mut e = DocEntry {
                account_id: account_id.to_string(),
                folder: folder.to_string(),
                uid,
                message_id: info.message_id.clone(),
                from_name: info.from_name.clone(),
                from_addr: info.from_addr.clone(),
                subject: info.subject.clone(),
                timestamp: info.timestamp,
                index,
                name: a.name.clone(),
                mime: a.mime.clone(),
                size: u64::try_from(a.size()).unwrap_or(u64::MAX),
                text: crate::extract::text_of(a),
                folded: String::new(),
            };
            e.fold();
            Arc::new(e)
        })
        .collect()
}

/// "INBOX.Progetti.Alfa" → "Alfa", "INBOX" → "Posta in arrivo".
fn folder_label(name: &str) -> String {
    if name.eq_ignore_ascii_case("inbox") {
        return "Posta in arrivo".into();
    }
    name.rsplit(['/', '.'])
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(name)
        .to_string()
}

/// Lowercased words of a query.
pub fn query_words(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .map(str::to_lowercase)
        .filter(|w| !w.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const RAW: &str = "From: Anna Rossi <anna@fornitore.it>\r\nSubject: Offerta wallbox\r\nMessage-ID: <x1@fornitore.it>\r\nDate: Tue, 20 Oct 2026 10:00:00 +0200\r\nContent-Type: multipart/mixed; boundary=\"M\"\r\n\r\n--M\r\nContent-Type: text/plain\r\n\r\nIn allegato.\r\n--M\r\nContent-Type: text/csv\r\nContent-Disposition: attachment; filename=\"listino.csv\"\r\n\r\ncodice;prezzo\r\nEWALL-GO;890\r\n--M\r\nContent-Type: image/png\r\nContent-Disposition: attachment; filename=\"foto.png\"\r\nContent-Transfer-Encoding: base64\r\n\r\naGk=\r\n--M--\r\n";

    #[test]
    fn entries_carry_message_and_text() {
        let list = entries_of("acc", "INBOX", 42, b"", RAW.as_bytes());
        let names: Vec<&str> = list.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["listino.csv", "foto.png"]);
        let csv = list.first().unwrap();
        assert_eq!(csv.subject, "Offerta wallbox");
        assert_eq!(csv.from_addr, "anna@fornitore.it");
        assert_eq!(csv.message_id, "x1@fornitore.it");
        assert!(csv.timestamp > 0);
        assert_eq!(csv.kind(), FileKind::Text);
        assert!(csv.text.contains("EWALL-GO;890"));
        assert!(csv.matches(&query_words("ewall-go  LISTINO")));
        assert!(csv.matches(&query_words("anna offerta")));
        assert!(!csv.matches(&query_words("fattura")));
        assert_eq!(
            csv.excerpt(&query_words("890"), 12).as_deref(),
            Some("…-GO;890")
        );
        assert_eq!(list.get(1).map(|e| e.text.as_str()), Some(""));
        assert_eq!(list.get(1).map(|e| e.kind()), Some(FileKind::Image));
    }

    #[test]
    fn json_roundtrip_refolds() {
        let index = DocIndex {
            folders: BTreeMap::new(),
            entries: entries_of("acc", "INBOX", 1, b"", RAW.as_bytes()),
        };
        let bytes = serde_json::to_vec(&index).unwrap();
        let mut back: DocIndex = serde_json::from_slice(&bytes).unwrap();
        for e in &mut back.entries {
            Arc::get_mut(e).unwrap().fold();
        }
        assert!(back.entries.first().unwrap().matches(&["listino".into()]));
    }

    #[test]
    fn batches_respect_count_and_bytes() {
        let list: Vec<(u32, u32, Vec<u8>)> = (0..60).map(|i| (i, 1000, Vec::new())).collect();
        let b = batches(&list);
        assert_eq!(
            b.iter().map(|x| x.len()).collect::<Vec<_>>(),
            vec![25, 25, 10]
        );
        let big: Vec<(u32, u32, Vec<u8>)> =
            (0..3).map(|i| (i, 20 * 1024 * 1024, Vec::new())).collect();
        assert_eq!(batches(&big).len(), 3);
        assert!(batches(&[]).is_empty());
    }

    #[test]
    fn garbage_never_panics() {
        for raw in [
            &b""[..],
            b"\xff\xfe",
            b"Content-Type: multipart/mixed\r\n\r\n--",
        ] {
            let _ = entries_of("a", "f", 1, b"", raw);
        }
    }
}
