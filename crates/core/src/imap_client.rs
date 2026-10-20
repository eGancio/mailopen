//! A thin, synchronous IMAP session over rustls. One per account, owned by the worker.

use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use imap::types::{Fetch, Flag, NameAttribute};
use mail_parser::{Address, MessageParser, MimeHeaders, PartType};
use rustls_connector::{RustlsConnector, TlsStream};

use crate::config::Account;
use crate::error::{Error, Result};
use crate::model::{Folder, FolderKind, MessageBody, MessageSummary};

type Session = imap::Session<TlsStream<TcpStream>>;

const TIMEOUT: Duration = Duration::from_secs(120);
/// Bodies larger than this are not downloaded (the UI says so).
const MAX_BODY: usize = 25 * 1024 * 1024;
/// Width used when an HTML-only message is rendered to text.
const TEXT_WIDTH: usize = 100;
/// Bytes of body fetched together with the headers to build the list preview.
const SNIPPET_BYTES: usize = 3072;
const SNIPPET_CHARS: usize = 140;

pub struct ImapClient {
    session: Session,
    pub account: Account,
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
        Ok(parse_body(raw))
    }

    pub fn set_seen(&mut self, folder: &str, uid: u32, seen: bool) -> Result<()> {
        self.session.select(folder)?;
        let op = if seen {
            "+FLAGS.SILENT (\\Seen)"
        } else {
            "-FLAGS.SILENT (\\Seen)"
        };
        self.session.uid_store(uid.to_string(), op)?;
        Ok(())
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
    let mut raw = Vec::with_capacity(
        header
            .len()
            .saturating_add(partial_body.len())
            .saturating_add(2),
    );
    raw.extend_from_slice(header);
    if !header.ends_with(b"\r\n\r\n") {
        raw.extend_from_slice(b"\r\n");
    }
    raw.extend_from_slice(partial_body);
    let text = parse_body(&raw).text;
    let collapsed: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out: String = collapsed.chars().take(SNIPPET_CHARS).collect();
    if collapsed.chars().count() > SNIPPET_CHARS {
        out.push('…');
    }
    out
}

/// Parse a raw RFC 5322 message into what the reading pane shows.
pub fn parse_body(raw: &[u8]) -> MessageBody {
    let Some(msg) = MessageParser::default().parse(raw) else {
        return MessageBody {
            text: String::from_utf8_lossy(raw).into_owned(),
            ..MessageBody::default()
        };
    };
    // mail-parser's `body_text` already converts an HTML-only message to text; we prefer our
    // own rendering (html2text) when there is no genuine text/plain part.
    let has_plain = msg
        .text_bodies()
        .any(|p| matches!(p.body, PartType::Text(_)));
    let (text, was_html) = match (has_plain, msg.body_html(0)) {
        (false, Some(h)) => (
            html2text::from_read(h.as_bytes(), TEXT_WIDTH).unwrap_or_else(|_| h.into_owned()),
            true,
        ),
        _ => (
            msg.body_text(0).map(|t| t.into_owned()).unwrap_or_default(),
            false,
        ),
    };
    let attachments = msg
        .attachments()
        .map(|p| p.attachment_name().unwrap_or("allegato").to_string())
        .collect();
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
        was_html,
        attachments,
        message_id: msg.message_id().unwrap_or("").to_string(),
        references,
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
        assert!(!b.was_html);
        assert_eq!(b.message_id, "1@example.it");
        assert_eq!(b.references, vec!["0@example.it"]);
        assert!(b.attachments.is_empty());
    }

    #[test]
    fn html_only_is_rendered_to_text() {
        let raw = b"From: a@b.it\r\nSubject: h\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<html><body><p>Ciao <b>mondo</b></p></body></html>";
        let b = parse_body(raw);
        assert!(b.was_html);
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
