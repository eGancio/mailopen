//! A thin, synchronous IMAP session over rustls. One per account, owned by the worker.

use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

use imap::types::{Fetch, Flag, NameAttribute};
use mail_parser::{Address, MessageParser, MimeHeaders, PartType};
use rustls_connector::{RustlsConnector, TlsStream};

use crate::config::Account;
use crate::error::{Error, Result};
use crate::model::{Attachment, Folder, FolderKind, MessageBody, MessageSummary, normalize_cid};

type Session = imap::Session<TlsStream<TcpStream>>;

const TIMEOUT: Duration = Duration::from_secs(120);
/// Bodies larger than this are not downloaded (the UI says so).
const MAX_BODY: usize = 40 * 1024 * 1024;
/// Bytes of body fetched together with the headers to build the list preview.
const SNIPPET_BYTES: usize = 6144;
const SNIPPET_CHARS: usize = 140;

pub struct ImapClient {
    session: Session,
    pub account: Account,
    /// The conversation went out of step (a reply the library could not read is still on the
    /// wire): the worker drops this session and opens a new one.
    pub broken: bool,
}

impl ImapClient {
    /// Connect, TLS-wrap, read the greeting and log in.
    pub fn connect(account: &Account, password: &str) -> Result<ImapClient> {
        let host = account.imap_host.trim();
        if host.is_empty() {
            return Err(Error::Config("server IMAP mancante".into()));
        }
        let addr = (host, account.imap_port)
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| Error::Imap(format!("{host}: indirizzo non risolto")))?;
        let tcp = TcpStream::connect_timeout(&addr, TIMEOUT)?;
        tcp.set_read_timeout(Some(TIMEOUT))?;
        tcp.set_write_timeout(Some(TIMEOUT))?;
        let connector = RustlsConnector::new_with_platform_verifier()?;
        let tls = connector
            .connect(host, tcp)
            .map_err(|e| Error::Tls(e.to_string()))?;
        let mut client = imap::Client::new(tls);
        client.read_greeting()?;
        let session = client
            .login(account.username.trim(), password)
            .map_err(|(e, _)| Error::Imap(format!("accesso rifiutato: {e}")))?;
        Ok(ImapClient {
            session,
            account: account.clone(),
            broken: false,
        })
    }

    /// Verify the credentials and hang up.
    pub fn test_login(account: &Account, password: &str) -> Result<()> {
        let mut c = ImapClient::connect(account, password)?;
        let _ = c.session.logout();
        Ok(())
    }

    pub fn logout(mut self) {
        let _ = self.session.logout();
    }

    /// All mailboxes, classified. INBOX first, then the special folders, then the rest by name.
    pub fn folders(&mut self) -> Result<Vec<Folder>> {
        let names = self.session.list(Some(""), Some("*"))?;
        let mut out: Vec<Folder> = names
            .iter()
            .map(|n| {
                let attrs: Vec<String> = n
                    .attributes()
                    .iter()
                    .filter_map(|a| match a {
                        NameAttribute::Custom(s) => Some(s.to_string()),
                        _ => None,
                    })
                    .collect();
                let selectable = !n
                    .attributes()
                    .iter()
                    .any(|a| matches!(a, NameAttribute::NoSelect));
                Folder {
                    name: n.name().to_string(),
                    kind: Folder::classify(n.name(), &attrs),
                    selectable,
                }
            })
            .collect();
        if !out.iter().any(|f| f.kind == FolderKind::Inbox) {
            out.insert(
                0,
                Folder {
                    name: "INBOX".into(),
                    kind: FolderKind::Inbox,
                    selectable: true,
                },
            );
        }
        out.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.name.cmp(&b.name)));
        Ok(out)
    }

    /// The newest `limit` messages of a folder, headers only, newest first.
    pub fn summaries(&mut self, folder: &str, limit: u32) -> Result<Vec<MessageSummary>> {
        let mailbox = self.session.select(folder)?;
        let exists = mailbox.exists;
        if exists == 0 {
            return Ok(Vec::new());
        }
        let first = exists.saturating_sub(limit.max(1).saturating_sub(1)).max(1);
        let range = format!("{first}:{exists}");
        let query = format!(
            "(UID FLAGS INTERNALDATE RFC822.SIZE BODY.PEEK[HEADER] BODY.PEEK[TEXT]<0.{SNIPPET_BYTES}>)"
        );
        let fetches = self.session.fetch(range, query)?;
        let mut out: Vec<MessageSummary> = fetches
            .iter()
            .filter_map(|f| summary_from_fetch(&self.account.id, folder, f))
            .collect();
        out.sort_by(|a, b| {
            b.timestamp
                .cmp(&a.timestamp)
                .then_with(|| b.uid.cmp(&a.uid))
        });
        Ok(out)
    }

    /// Full body of one message. Does not mark it as read (PEEK).
    pub fn body(&mut self, folder: &str, uid: u32) -> Result<MessageBody> {
        Ok(parse_body(&self.raw(folder, uid)?))
    }

    /// The whole RFC 5322 message, as stored on the server. Does not mark it as read (PEEK).
    pub fn raw(&mut self, folder: &str, uid: u32) -> Result<Vec<u8>> {
        self.session.select(folder)?;
        let fetches = self
            .session
            .uid_fetch(uid.to_string(), "(UID RFC822.SIZE BODY.PEEK[])")?;
        let fetch = fetches
            .iter()
            .find(|f| f.uid == Some(uid))
            .ok_or_else(|| Error::Imap(format!("messaggio {uid} non trovato in {folder}")))?;
        let raw = fetch
            .body()
            .ok_or_else(|| Error::Imap("corpo del messaggio assente".into()))?;
        if raw.len() > MAX_BODY {
            return Err(Error::Imap(format!(
                "messaggio troppo grande ({} MB)",
                raw.len() / (1024 * 1024)
            )));
        }
        Ok(raw.to_vec())
    }

    pub fn set_seen(&mut self, folder: &str, uid: u32, seen: bool) -> Result<()> {
        self.set_flag(folder, uid, "\\Seen", seen)
    }

    /// The star / "contrassegnato" flag.
    pub fn set_flagged(&mut self, folder: &str, uid: u32, flagged: bool) -> Result<()> {
        self.set_flag(folder, uid, "\\Flagged", flagged)
    }

    fn set_flag(&mut self, folder: &str, uid: u32, flag: &str, on: bool) -> Result<()> {
        self.session.select(folder)?;
        let sign = if on { '+' } else { '-' };
        self.session
            .uid_store(uid.to_string(), format!("{sign}FLAGS.SILENT ({flag})"))?;
        Ok(())
    }

    /// Move a message to another folder (MOVE, else COPY + delete).
    pub fn move_to(&mut self, folder: &str, uid: u32, to: &str) -> Result<()> {
        if folder == to {
            return Ok(());
        }
        self.session.select(folder)?;
        let uid_s = uid.to_string();
        if self.session.uid_mv(&uid_s, to).is_ok() {
            return Ok(());
        }
        self.session.uid_copy(&uid_s, to)?;
        self.session
            .uid_store(&uid_s, "+FLAGS.SILENT (\\Deleted)")?;
        if self.session.uid_expunge(&uid_s).is_err() {
            self.session.expunge()?;
        }
        Ok(())
    }

    /// Messages of `folder` whose headers or text contain every word of `query` (the server
    /// searches), newest first, at most `limit`.
    pub fn search(&mut self, folder: &str, query: &str, limit: u32) -> Result<Vec<MessageSummary>> {
        let Some(criteria) = search_criteria(query) else {
            return Ok(Vec::new());
        };
        self.session.select(folder)?;
        let mut uids = self.uid_search(&criteria)?;
        uids.sort_unstable_by(|a, b| b.cmp(a));
        uids.truncate(usize::try_from(limit.max(1)).unwrap_or(100));
        self.summaries_by_uid(folder, &uids)
    }

    /// The message with this `Message-ID` in `folder`, if it is there.
    pub fn find_by_message_id(
        &mut self,
        folder: &str,
        message_id: &str,
    ) -> Result<Option<MessageSummary>> {
        let id = message_id.trim().trim_matches(['<', '>']);
        if id.is_empty() || !id.is_ascii() || id.contains(['"', '\\', '\r', '\n']) {
            return Ok(None);
        }
        self.session.select(folder)?;
        let mut uids = self.uid_search(&format!("HEADER Message-ID \"{id}\""))?;
        uids.sort_unstable();
        let Some(uid) = uids.last().copied() else {
            return Ok(None);
        };
        Ok(self.summaries_by_uid(folder, &[uid])?.into_iter().next())
    }

    /// List rows for these UIDs of the selected `folder`, newest first.
    fn summaries_by_uid(&mut self, folder: &str, uids: &[u32]) -> Result<Vec<MessageSummary>> {
        if uids.is_empty() {
            return Ok(Vec::new());
        }
        let set: Vec<String> = uids.iter().map(u32::to_string).collect();
        let query = format!(
            "(UID FLAGS INTERNALDATE RFC822.SIZE BODY.PEEK[HEADER] BODY.PEEK[TEXT]<0.{SNIPPET_BYTES}>)"
        );
        let fetches = self.session.uid_fetch(set.join(","), query)?;
        let mut out: Vec<MessageSummary> = fetches
            .iter()
            .filter_map(|f| summary_from_fetch(&self.account.id, folder, f))
            .collect();
        out.sort_by(|a, b| {
            b.timestamp
                .cmp(&a.timestamp)
                .then_with(|| b.uid.cmp(&a.uid))
        });
        Ok(out)
    }

    /// UIDVALIDITY and every UID of a folder (for the document index).
    pub fn all_uids(&mut self, folder: &str) -> Result<(u32, Vec<u32>)> {
        let mailbox = self.session.select(folder)?;
        let validity = mailbox.uid_validity.unwrap_or(0);
        if mailbox.exists == 0 {
            return Ok((validity, Vec::new()));
        }
        let mut uids = self.uid_search("ALL")?;
        uids.sort_unstable();
        Ok((validity, uids))
    }

    /// Print the IMAP conversation on stderr from now on (examples only: it shows mail).
    pub fn set_debug(&mut self, on: bool) {
        self.session.debug = on;
    }

    /// The server's raw answer to a UID SEARCH (for trying criteria by hand).
    pub fn raw_search(&mut self, folder: &str, criteria: &str) -> Result<String> {
        self.session.select(folder)?;
        let raw = self
            .session
            .run_command_and_read_response(format!("UID SEARCH {criteria}"))?;
        Ok(String::from_utf8_lossy(&raw).into_owned())
    }

    /// UID SEARCH on the selected folder. The answer is read by hand: some servers (Dovecot)
    /// send progress lines like `* OK Searched 40% of the mailbox` during long searches, which
    /// the IMAP library takes for an error.
    fn uid_search(&mut self, criteria: &str) -> Result<Vec<u32>> {
        match self
            .session
            .run_command_and_read_response(format!("UID SEARCH {criteria}"))
        {
            Ok(raw) => Ok(parse_search(&raw)),
            // Zoho says "* SEARCH " (with a space) when nothing matches; the library cannot
            // parse that line and leaves the final OK unread.
            Err(imap::Error::Parse(imap::error::ParseError::Invalid(data)))
                if is_empty_search(&data) =>
            {
                self.broken = true;
                Ok(Vec::new())
            }
            Err(e) => {
                self.broken = true;
                Err(e.into())
            }
        }
    }

    /// (uid, size, header) of these UIDs of `folder`.
    pub fn headers(&mut self, folder: &str, uids: &[u32]) -> Result<Vec<(u32, u32, Vec<u8>)>> {
        if uids.is_empty() {
            return Ok(Vec::new());
        }
        self.session.select(folder)?;
        let set: Vec<String> = uids.iter().map(u32::to_string).collect();
        let fetches = self
            .session
            .uid_fetch(set.join(","), "(UID RFC822.SIZE BODY.PEEK[HEADER])")?;
        Ok(fetches
            .iter()
            .filter_map(|f| {
                Some((
                    f.uid?,
                    f.size.unwrap_or(0),
                    f.header().unwrap_or(b"").to_vec(),
                ))
            })
            .collect())
    }

    /// Whole messages for these UIDs of `folder`, in one round trip. The caller keeps the batch
    /// small; messages above the body cap are left out.
    pub fn raws(&mut self, folder: &str, uids: &[u32]) -> Result<Vec<(u32, Vec<u8>)>> {
        if uids.is_empty() {
            return Ok(Vec::new());
        }
        self.session.select(folder)?;
        let set: Vec<String> = uids.iter().map(u32::to_string).collect();
        let fetches = self.session.uid_fetch(set.join(","), "(UID BODY.PEEK[])")?;
        Ok(fetches
            .iter()
            .filter_map(|f| {
                let raw = f.body()?;
                (raw.len() <= MAX_BODY).then(|| (f.uid, raw.to_vec()))
            })
            .filter_map(|(uid, raw)| Some((uid?, raw)))
            .collect())
    }

    /// Move to `trash` when given (and not already there), otherwise flag deleted and expunge.
    pub fn delete(&mut self, folder: &str, uid: u32, trash: Option<&str>) -> Result<()> {
        self.session.select(folder)?;
        let uid_s = uid.to_string();
        if let Some(trash) = trash.filter(|t| *t != folder) {
            if self.session.uid_mv(&uid_s, trash).is_ok() {
                return Ok(());
            }
            self.session.uid_copy(&uid_s, trash)?;
        }
        self.session
            .uid_store(&uid_s, "+FLAGS.SILENT (\\Deleted)")?;
        if self.session.uid_expunge(&uid_s).is_err() {
            self.session.expunge()?;
        }
        Ok(())
    }

    /// Store a copy of a sent message (servers like Gmail do this by themselves).
    pub fn append_sent(&mut self, folder: &str, raw: &[u8]) -> Result<()> {
        self.session.append_with_flags(folder, raw, &[Flag::Seen])?;
        Ok(())
    }

    pub fn noop(&mut self) -> Result<()> {
        self.session.noop()?;
        Ok(())
    }
}

/// A response whose only search line is an empty `* SEARCH`.
fn is_empty_search(raw: &[u8]) -> bool {
    let text = String::from_utf8_lossy(raw);
    let found = text
        .lines()
        .map(str::trim)
        .any(|l| l.eq_ignore_ascii_case("* SEARCH"));
    found && parse_search(raw).is_empty()
}

/// The numbers of every `* SEARCH` line of a response; other lines are ignored.
pub fn parse_search(raw: &[u8]) -> Vec<u32> {
    let text = String::from_utf8_lossy(raw);
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line
            .get(..9)
            .filter(|p| p.eq_ignore_ascii_case("* SEARCH "))
            .and_then(|_| line.get(9..))
        else {
            continue;
        };
        out.extend(rest.split_whitespace().map_while(|w| w.parse::<u32>().ok()));
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// IMAP SEARCH criteria for a typed query: every word must be in the headers or the text
/// (`TEXT`). `None` when there is nothing to search. Quotes and control characters are dropped,
/// so the query can never break out of its string.
pub fn search_criteria(query: &str) -> Option<String> {
    let words: Vec<String> = query
        .split_whitespace()
        .map(|w| {
            w.chars()
                .filter(|c| !c.is_control() && *c != '"' && *c != '\\')
                .collect::<String>()
        })
        .filter(|w| !w.is_empty())
        .take(6)
        .collect();
    if words.is_empty() {
        return None;
    }
    let terms: Vec<String> = words.iter().map(|w| format!("TEXT \"{w}\"")).collect();
    let charset = if words.iter().all(|w| w.is_ascii()) {
        ""
    } else {
        "CHARSET UTF-8 "
    };
    Some(format!("{charset}{}", terms.join(" ")))
}

/// Whether a message with this header can carry files: a mixed multipart, or a single part
/// that is not text (a PDF sent on its own).
pub fn may_have_files(header: &[u8]) -> bool {
    let Some(msg) = MessageParser::default().parse_headers(header) else {
        return false;
    };
    let Some(ct) = msg.content_type() else {
        return false;
    };
    let ty = ct.ctype().to_ascii_lowercase();
    let sub = ct.subtype().unwrap_or("").to_ascii_lowercase();
    match ty.as_str() {
        "multipart" => sub == "mixed",
        "text" => false,
        _ => true,
    }
}

/// What the document index keeps from a message header.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HeaderInfo {
    pub from_name: String,
    pub from_addr: String,
    pub subject: String,
    pub message_id: String,
    /// Unix seconds (UTC), 0 when unknown.
    pub timestamp: i64,
}

pub fn header_info(raw: &[u8]) -> HeaderInfo {
    let Some(msg) = MessageParser::default().parse_headers(raw) else {
        return HeaderInfo::default();
    };
    let (from_name, from_addr) = first_addr(msg.from());
    HeaderInfo {
        from_name,
        from_addr,
        subject: msg.subject().unwrap_or("").trim().to_string(),
        message_id: msg.message_id().unwrap_or("").to_string(),
        timestamp: msg
            .date()
            .filter(|d| d.is_valid())
            .map(|d| d.to_timestamp())
            .unwrap_or(0),
    }
}

/// Sender name, sender address and subject of a raw message.
pub fn sender_and_subject(raw: &[u8]) -> (String, String, String) {
    let Some(msg) = MessageParser::default().parse_headers(raw) else {
        return Default::default();
    };
    let (name, addr) = first_addr(msg.from());
    (name, addr, msg.subject().unwrap_or("").trim().to_string())
}

fn first_addr(addr: Option<&Address<'_>>) -> (String, String) {
    addr.and_then(|a| a.first())
        .map(|a| {
            (
                a.name().unwrap_or("").trim().to_string(),
                a.address().unwrap_or("").trim().to_string(),
            )
        })
        .unwrap_or_default()
}

fn addr_list(addr: Option<&Address<'_>>) -> Vec<String> {
    addr.map(|a| {
        a.iter()
            .map(
                |x| match (x.name().map(str::trim), x.address().map(str::trim)) {
                    (Some(n), Some(ad)) if !n.is_empty() => format!("{n} <{ad}>"),
                    (_, Some(ad)) => ad.to_string(),
                    (Some(n), None) => n.to_string(),
                    (None, None) => String::new(),
                },
            )
            .filter(|s| !s.is_empty())
            .collect()
    })
    .unwrap_or_default()
}

fn summary_from_fetch(account_id: &str, folder: &str, f: &Fetch) -> Option<MessageSummary> {
    let uid = f.uid?;
    let header = f.header().unwrap_or(b"");
    let parsed = MessageParser::default().parse(header);
    let has_attachment = parsed
        .as_ref()
        .and_then(|m| m.content_type())
        .map(|ct| {
            ct.ctype().eq_ignore_ascii_case("multipart")
                && ct
                    .subtype()
                    .is_some_and(|s| s.eq_ignore_ascii_case("mixed"))
        })
        .unwrap_or(false);
    let snippet = f
        .text()
        .map(|t| snippet_from(header, t))
        .unwrap_or_default();
    let (from_name, from_addr) = first_addr(parsed.as_ref().and_then(|m| m.from()));
    let subject = parsed
        .as_ref()
        .and_then(|m| m.subject())
        .unwrap_or("")
        .trim()
        .to_string();
    let message_id = parsed
        .as_ref()
        .and_then(|m| m.message_id())
        .unwrap_or("")
        .to_string();
    let timestamp = parsed
        .as_ref()
        .and_then(|m| m.date())
        .filter(|d| d.is_valid())
        .map(|d| d.to_timestamp())
        .or_else(|| f.internal_date().map(|d| d.timestamp()))
        .unwrap_or(0);
    let flags = f.flags();
    Some(MessageSummary {
        account_id: account_id.to_string(),
        folder: folder.to_string(),
        uid,
        from_name,
        from_addr,
        subject,
        timestamp,
        seen: flags.iter().any(|x| matches!(x, Flag::Seen)),
        answered: flags.iter().any(|x| matches!(x, Flag::Answered)),
        flagged: flags.iter().any(|x| matches!(x, Flag::Flagged)),
        size: f.size.unwrap_or(0),
        message_id,
        snippet,
        has_attachment,
    })
}

/// Preview line from the headers plus a truncated body: parse what we have, take the text
/// part (or the HTML rendered to text), collapse whitespace and cut to `SNIPPET_CHARS`.
pub fn snippet_from(header: &[u8], partial_body: &[u8]) -> String {
    let raw = close_truncated(header, partial_body, partial_body.len() >= SNIPPET_BYTES);
    let Some(msg) = MessageParser::default().parse(&raw) else {
        return String::new();
    };
    let (text, html) = main_text(&msg);
    let preheader = html
        .map(|h| crate::html::parse(&h).preheader)
        .unwrap_or_default();
    preview_line(&format!("{preheader} {text}"))
}

/// One line for the list: no invisible fillers, bare URLs or `[image.png]` placeholders.
pub fn preview_line(text: &str) -> String {
    // Some senders put HTML line breaks in their text part.
    let text = text
        .replace("<br>", " ")
        .replace("<br/>", " ")
        .replace("<br />", " ")
        .replace("<BR>", " ");
    let cleaned = crate::html::clean_line(&text);
    let collapsed: String = cleaned
        .split(' ')
        .filter(|w| {
            let l = w
                .trim_matches(['[', ']', '<', '>', '(', ')'])
                .to_ascii_lowercase();
            !(l.starts_with("http://")
                || l.starts_with("https://")
                || (w.starts_with('[') && w.ends_with(']') && l.contains('.'))
                || matches!(*w, "[" | "]" | "<" | ">" | "|"))
        })
        .collect::<Vec<_>>()
        .join(" ");
    let mut out: String = collapsed.chars().take(SNIPPET_CHARS).collect();
    if collapsed.chars().count() > SNIPPET_CHARS {
        out.push('…');
    }
    out
}

/// Headers plus a body that may stop mid-part. When `truncated`, cut the body at its last full
/// line and close every multipart still open: otherwise the decoder never finds the end of a
/// base64 part and hands back the undecoded text.
fn close_truncated(header: &[u8], body: &[u8], truncated: bool) -> Vec<u8> {
    let mut raw = Vec::with_capacity(header.len().saturating_add(body.len()).saturating_add(256));
    raw.extend_from_slice(header);
    if !header.ends_with(b"\r\n\r\n") {
        raw.extend_from_slice(b"\r\n");
    }
    if !truncated {
        raw.extend_from_slice(body);
        return raw;
    }
    let cut = body
        .iter()
        .rposition(|b| *b == b'\n')
        .map(|i| i.saturating_add(1))
        .unwrap_or(0);
    let kept = body.get(..cut).unwrap_or(b"");
    raw.extend_from_slice(kept);
    let mut boundaries = boundaries_in(header);
    boundaries.extend(boundaries_in(kept));
    for b in boundaries.iter().rev() {
        let close = format!("--{b}--");
        if !contains(kept, close.as_bytes()) {
            raw.extend_from_slice(b"\r\n");
            raw.extend_from_slice(close.as_bytes());
            raw.extend_from_slice(b"\r\n");
        }
    }
    raw
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && hay.windows(needle.len()).any(|w| w == needle)
}

/// Every `boundary=` parameter value, in order.
fn boundaries_in(bytes: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(bytes);
    let lower = text.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut from = 0usize;
    while let Some(i) = lower.get(from..).and_then(|s| s.find("boundary=")) {
        let start = from.saturating_add(i).saturating_add("boundary=".len());
        let Some(rest) = text.get(start..) else { break };
        let value = if let Some(q) = rest.strip_prefix('"') {
            q.split('"').next().unwrap_or("")
        } else {
            rest.split(|c: char| c == ';' || c.is_whitespace())
                .next()
                .unwrap_or("")
        };
        if !value.is_empty() && !out.iter().any(|b: &String| b == value) {
            out.push(value.to_string());
        }
        from = start;
    }
    out
}

/// The text for replies and previews, and the HTML when the message has an HTML part.
fn main_text(msg: &mail_parser::Message<'_>) -> (String, Option<String>) {
    // mail-parser converts between text and HTML when a part is missing; only take what the
    // sender wrote.
    let has_plain = msg
        .text_bodies()
        .any(|p| matches!(p.body, PartType::Text(_)));
    let has_html = msg
        .html_bodies()
        .any(|p| matches!(p.body, PartType::Html(_)));
    let html = has_html
        .then(|| msg.body_html(0).map(|h| h.into_owned()))
        .flatten();
    let plain = msg.body_text(0).map(|t| t.into_owned()).unwrap_or_default();
    let text = match &html {
        // Some senders ship an empty text part next to the real HTML one.
        Some(h) if !has_plain || plain.trim().is_empty() => crate::html::parse(h).to_text(),
        _ => plain,
    };
    (text, html)
}

/// Whether the message's text part has real content.
fn plain_is_usable(msg: &mail_parser::Message<'_>) -> bool {
    msg.text_bodies()
        .any(|p| matches!(&p.body, PartType::Text(t) if !t.trim().is_empty()))
}

/// Parse a raw RFC 5322 message into what the reading pane shows.
pub fn parse_body(raw: &[u8]) -> MessageBody {
    let Some(msg) = MessageParser::default().parse(raw) else {
        let text = String::from_utf8_lossy(raw).into_owned();
        return MessageBody {
            doc: Arc::new(crate::html::from_plain(&text)),
            text,
            ..MessageBody::default()
        };
    };
    let has_plain = plain_is_usable(&msg);
    let html = msg
        .html_bodies()
        .any(|p| matches!(p.body, PartType::Html(_)))
        .then(|| msg.body_html(0).map(|h| h.into_owned()))
        .flatten();
    let (text, doc) = match &html {
        Some(h) => {
            let d = crate::html::parse(h);
            let text = if has_plain {
                msg.body_text(0).map(|t| t.into_owned()).unwrap_or_default()
            } else {
                d.to_text()
            };
            (text, d)
        }
        None => {
            let text = msg.body_text(0).map(|t| t.into_owned()).unwrap_or_default();
            let d = crate::html::from_plain(&text);
            (text, d)
        }
    };
    let (attachments, inline) = collect_parts(&msg, &doc.cids);
    let references = msg
        .references()
        .as_text_list()
        .map(|l| l.iter().map(|s| s.to_string()).collect())
        .or_else(|| msg.references().as_text().map(|s| vec![s.to_string()]))
        .unwrap_or_default();
    MessageBody {
        to: addr_list(msg.to()),
        cc: addr_list(msg.cc()),
        reply_to: addr_list(msg.reply_to()).into_iter().next(),
        text,
        is_html: html.is_some(),
        html,
        doc: Arc::new(doc),
        attachments,
        inline,
        message_id: msg.message_id().unwrap_or("").to_string(),
        references,
    }
}

/// Every file in the message: attachments to list, and the parts the HTML shows by `cid:`.
fn collect_parts(
    msg: &mail_parser::Message<'_>,
    cids: &[String],
) -> (Vec<Attachment>, Vec<Attachment>) {
    let mut attachments = Vec::new();
    let mut inline = Vec::new();
    let bodies: Vec<u32> = msg
        .text_body
        .iter()
        .chain(msg.html_body.iter())
        .copied()
        .collect();
    for (i, part) in msg.parts.iter().enumerate() {
        let id = u32::try_from(i).unwrap_or(u32::MAX);
        let listed = msg.attachments.contains(&id);
        let file = match &part.body {
            PartType::Multipart(_) => false,
            PartType::Text(_) | PartType::Html(_) => listed && !bodies.contains(&id),
            PartType::Binary(_) | PartType::InlineBinary(_) | PartType::Message(_) => true,
        };
        if !file {
            continue;
        }
        let mime = part
            .content_type()
            .map(|ct| {
                format!("{}/{}", ct.ctype(), ct.subtype().unwrap_or("octet-stream"))
                    .to_ascii_lowercase()
            })
            .unwrap_or_else(|| match &part.body {
                PartType::Text(_) => "text/plain".into(),
                PartType::Message(_) => "message/rfc822".into(),
                _ => "application/octet-stream".into(),
            });
        let content_id = part
            .content_id()
            .map(normalize_cid)
            .filter(|c| !c.is_empty());
        let is_inline = content_id.as_ref().is_some_and(|c| cids.contains(c));
        let name = match part
            .attachment_name()
            .map(str::trim)
            .filter(|n| !n.is_empty())
        {
            Some(n) => sanitize_name(n),
            None => match &part.body {
                PartType::Message(m) => {
                    let subject = m.subject().unwrap_or("messaggio").trim();
                    format!("{}.eml", sanitize_name(subject))
                }
                _ => {
                    let (word, n) = if is_inline {
                        ("immagine", inline.len())
                    } else {
                        ("allegato", attachments.len())
                    };
                    format!("{word}-{}.{}", n.saturating_add(1), extension_for(&mime))
                }
            },
        };
        let a = Attachment {
            name,
            mime,
            content_id,
            data: Arc::from(part.contents()),
        };
        if is_inline {
            inline.push(a);
        } else {
            attachments.push(a);
        }
    }
    (attachments, inline)
}

/// A file name safe to show and to save: no path separators or control characters.
pub fn sanitize_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim_matches(|c: char| c == '.' || c == '_' || c.is_whitespace());
    let mut out: String = trimmed.chars().take(150).collect();
    if out.is_empty() {
        out = "allegato".into();
    }
    out
}

fn extension_for(mime: &str) -> &'static str {
    match mime {
        "application/pdf" => "pdf",
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "text/plain" => "txt",
        "text/html" => "html",
        "text/calendar" => "ics",
        "text/csv" => "csv",
        "message/rfc822" => "eml",
        "application/zip" => "zip",
        _ => "bin",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RAW: &[u8] = b"From: Anna Rossi <anna@example.it>\r\nTo: Bo <bo@example.it>, carla@example.it\r\nCc: dan@example.it\r\nSubject: Prova\r\nMessage-ID: <1@example.it>\r\nReferences: <0@example.it>\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nCiao,\r\nquesto \xc3\xa8 un test.\r\n";

    #[test]
    fn parses_plain_text() {
        let b = parse_body(RAW);
        assert_eq!(b.to, vec!["Bo <bo@example.it>", "carla@example.it"]);
        assert_eq!(b.cc, vec!["dan@example.it"]);
        assert!(b.text.contains("questo è un test"));
        assert!(!b.is_html);
        assert_eq!(b.message_id, "1@example.it");
        assert_eq!(b.references, vec!["0@example.it"]);
        assert!(b.attachments.is_empty());
    }

    #[test]
    fn html_only_is_rendered_to_text() {
        let raw = b"From: a@b.it\r\nSubject: h\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<html><body><p>Ciao <b>mondo</b></p></body></html>";
        let b = parse_body(raw);
        assert!(b.is_html);
        assert!(b.text.contains("Ciao"));
        assert!(b.text.contains("mondo"));
        assert!(!b.text.contains("<b>"));
    }

    /// Needs the network: `cargo test -p mailopen-core -- --ignored gmail_rejects`.
    #[test]
    #[ignore]
    fn gmail_rejects_bad_password_over_tls() {
        let a = Account {
            id: "t".into(),
            name: "t".into(),
            email: "nobody@gmail.com".into(),
            username: "nobody@gmail.com".into(),
            imap_host: "imap.gmail.com".into(),
            imap_port: 993,
            smtp_host: "smtp.gmail.com".into(),
            smtp_port: 465,
            smtp_security: crate::config::SmtpSecurity::Tls,
            color: 0,
            signature: String::new(),
            signature_kind: crate::config::SignatureKind::Text,
            signature_html: String::new(),
            style: None,
        };
        let err = ImapClient::test_login(&a, "password-sbagliata")
            .unwrap_err()
            .to_string();
        assert!(err.contains("accesso rifiutato"), "{err}");
    }

    #[test]
    fn snippet_from_truncated_multipart() {
        let header = b"From: a@b.it\r\nSubject: s\r\nContent-Type: multipart/alternative; boundary=\"XX\"\r\n\r\n";
        let body = b"--XX\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nCiao   Edoardo,\r\nquesto e' il testo\r\n--XX\r\nContent-Type: text/html\r\n\r\n<p>Ciao</p";
        let s = snippet_from(header, body);
        assert!(s.starts_with("Ciao Edoardo, questo e' il testo"), "{s}");
        assert_eq!(snippet_from(b"", b""), "");
        let _ = snippet_from(header, b"\xff\xfe");
    }

    #[test]
    fn snippet_from_base64_cut_mid_part() {
        // The list fetches only the first bytes of the body: a base64 part cut there must still
        // be decoded, not shown as "RGVhciBTaXIv…".
        let header = [
            "From: a@b.it",
            "Subject: s",
            "Content-Type: multipart/alternative; boundary=\"BB\"",
            "",
            "",
        ]
        .join("\r\n");
        let text = "Dear Sir/Madam, we would like to introduce our terminals. ".repeat(80);
        let table = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut b64 = String::new();
        for chunk in text.as_bytes().chunks(3) {
            let n = chunk
                .iter()
                .enumerate()
                .fold(0u32, |n, (i, b)| n | (u32::from(*b) << (16 - 8 * i)));
            for i in 0..4 {
                if i <= chunk.len() {
                    b64.push(table[((n >> (18 - 6 * i)) & 63) as usize] as char);
                } else {
                    b64.push('=');
                }
            }
        }
        let lines: Vec<String> = b64
            .as_bytes()
            .chunks(76)
            .map(|l| String::from_utf8_lossy(l).into_owned())
            .collect();
        let body = [
            "--BB".to_string(),
            "Content-Type: text/plain; charset=utf-8".into(),
            "Content-Transfer-Encoding: base64".into(),
            String::new(),
            lines.join("\r\n"),
            "--BB".into(),
            "Content-Type: text/html".into(),
            String::new(),
            "<p>x</p>".into(),
            "--BB--".into(),
            String::new(),
        ]
        .join("\r\n");
        let partial = body.as_bytes().get(..SNIPPET_BYTES).unwrap();
        let s = snippet_from(header.as_bytes(), partial);
        assert!(s.starts_with("Dear Sir/Madam, we would like"), "{s}");
    }

    #[test]
    fn attachments_and_inline_images() {
        let raw = [
            "From: a@b.it",
            "Subject: s",
            "Content-Type: multipart/mixed; boundary=\"M\"",
            "",
            "--M",
            "Content-Type: multipart/related; boundary=\"R\"",
            "",
            "--R",
            "Content-Type: text/html; charset=utf-8",
            "",
            "<p>Ciao <img src=\"cid:Logo@x\"></p>",
            "--R",
            "Content-Type: image/png",
            "Content-ID: <logo@x>",
            "Content-Transfer-Encoding: base64",
            "",
            "aGk=",
            "--R--",
            "--M",
            "Content-Type: application/octet-stream",
            "Content-Disposition: attachment; filename=\"NDA Fornitore.docx\"",
            "Content-Transfer-Encoding: base64",
            "",
            "aGVsbG8=",
            "--M",
            "Content-Type: application/pdf",
            "Content-Transfer-Encoding: base64",
            "",
            "aGVsbG8=",
            "--M--",
            "",
        ]
        .join("\r\n");
        let b = parse_body(raw.as_bytes());
        assert!(b.is_html);
        assert_eq!(b.inline.len(), 1, "{:?}", b.inline);
        assert_eq!(
            b.inline_part("cid:LOGO@x").map(|a| a.data.as_ref()),
            Some(&b"hi"[..])
        );
        let names: Vec<&str> = b.attachments.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, vec!["NDA Fornitore.docx", "allegato-2.pdf"]);
        assert_eq!(
            b.attachments.first().map(|a| a.kind()),
            Some(crate::model::FileKind::Document)
        );
        assert_eq!(
            b.attachments.first().map(|a| a.data.as_ref()),
            Some(&b"hello"[..])
        );
        assert_eq!(sanitize_name("..\\a/b:c.pdf"), "a_b_c.pdf");
    }

    #[test]
    fn search_criteria_are_quoted_and_safe() {
        assert_eq!(search_criteria("  "), None);
        assert_eq!(
            search_criteria("fattura 118").as_deref(),
            Some("TEXT \"fattura\" TEXT \"118\"")
        );
        assert_eq!(
            search_criteria("perché").as_deref(),
            Some("CHARSET UTF-8 TEXT \"perché\"")
        );
        assert_eq!(
            search_criteria("a\"\r\nb\\ \"").as_deref(),
            Some("TEXT \"a\" TEXT \"b\"")
        );
    }

    #[test]
    fn search_response_ignores_progress_lines() {
        let raw = b"* OK Searched 41% of the mailbox, ETA 0:12\r\n* SEARCH 5 3 900\r\n* search 7 (MODSEQ 4)\r\n";
        assert_eq!(parse_search(raw), vec![3, 5, 7, 900]);
        assert!(parse_search(b"* SEARCH\r\n").is_empty());
        assert!(parse_search(b"\xff* SEARCHx").is_empty());
        assert!(is_empty_search(b"* SEARCH \r\n"));
        assert!(!is_empty_search(b"* SEARCH 4\r\n"));
        assert!(!is_empty_search(b"* BYE\r\n"));
    }

    #[test]
    fn files_from_content_type() {
        assert!(may_have_files(
            b"Content-Type: multipart/mixed; boundary=x\r\n\r\n"
        ));
        assert!(may_have_files(b"Content-Type: application/pdf\r\n\r\n"));
        assert!(!may_have_files(
            b"Content-Type: multipart/alternative; boundary=x\r\n\r\n"
        ));
        assert!(!may_have_files(b"Content-Type: text/plain\r\n\r\n"));
        assert!(!may_have_files(b"Subject: nothing\r\n\r\n"));
        assert!(!may_have_files(b"\xff\xfe"));
    }

    #[test]
    fn garbage_never_panics() {
        for raw in [
            &b""[..],
            b"\xff\xfe\x00",
            b"Subject",
            b"\r\n\r\n",
            b"From: <<<>>>\r\n\r\n",
        ] {
            let _ = parse_body(raw);
        }
    }
}
