//! Send one message over SMTP (implicit TLS or STARTTLS) with rustls.

use std::time::Duration;

use lettre::message::{
    Attachment as MailAttachment, Mailbox, MultiPart, SinglePart, header::ContentType,
};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{Message, SmtpTransport, Transport};

use crate::config::{Account, SmtpSecurity};
use crate::error::{Error, Result};
use crate::model::OutgoingMessage;

/// Build the RFC 5322 message. Returned separately so the worker can also store it in Sent.
pub fn build(account: &Account, out: &OutgoingMessage) -> Result<Message> {
    let from: Mailbox = if account.name.trim().is_empty() {
        account.email.trim().parse()?
    } else {
        format!("{} <{}>", account.name.trim(), account.email.trim()).parse()?
    };
    let mut b = Message::builder().from(from).subject(out.subject.trim());
    let mut recipients = 0usize;
    for to in out.to.iter().flat_map(|s| split_addresses(s)) {
        b = b.to(to.parse()?);
        recipients = recipients.saturating_add(1);
    }
    for cc in out.cc.iter().flat_map(|s| split_addresses(s)) {
        b = b.cc(cc.parse()?);
        recipients = recipients.saturating_add(1);
    }
    for bcc in out.bcc.iter().flat_map(|s| split_addresses(s)) {
        b = b.bcc(bcc.parse()?);
        recipients = recipients.saturating_add(1);
    }
    if recipients == 0 {
        return Err(Error::Address("nessun destinatario".into()));
    }
    if let Some(id) = out.in_reply_to.as_deref().filter(|s| !s.trim().is_empty()) {
        b = b.in_reply_to(angle(id));
    }
    if !out.references.is_empty() {
        let refs: Vec<String> = out.references.iter().map(|r| angle(r)).collect();
        b = b.references(refs.join(" "));
    }
    let sig_html = out
        .html_signature
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let text_part = if sig_html.is_none() && out.html_body.is_none() {
        None
    } else {
        let body_html = match &out.html_body {
            Some(h) => h.clone(),
            None => format!(
                "<div style=\"white-space:pre-wrap;font-family:sans-serif\">{}</div>",
                escape_html(out.body.trim_end())
            ),
        };
        let (plain, html) = match sig_html {
            Some(sig) => {
                let sig_text = crate::html::parse(sig).to_text();
                (
                    format!("{}\n\n-- \n{}", out.body.trim_end(), sig_text.trim()),
                    format!("{body_html}<br><div>{sig}</div>"),
                )
            }
            None => (out.body.clone(), body_html),
        };
        Some(MultiPart::alternative_plain_html(plain, html))
    };
    if out.attachments.is_empty() {
        return Ok(match text_part {
            Some(alt) => b.multipart(alt)?,
            None => b.header(ContentType::TEXT_PLAIN).body(out.body.clone())?,
        });
    }
    let total: usize = out.attachments.iter().map(|a| a.size()).sum();
    if total > MAX_ATTACHMENTS {
        return Err(Error::Smtp(format!(
            "allegati troppo grandi ({} MB, massimo {} MB)",
            total / (1024 * 1024),
            MAX_ATTACHMENTS / (1024 * 1024)
        )));
    }
    let mut mixed = match text_part {
        Some(alt) => MultiPart::mixed().multipart(alt),
        None => MultiPart::mixed().singlepart(
            SinglePart::builder()
                .header(ContentType::TEXT_PLAIN)
                .body(out.body.clone()),
        ),
    };
    for a in &out.attachments {
        let ct = ContentType::parse(&a.mime).unwrap_or_else(|_| {
            ContentType::parse("application/octet-stream").unwrap_or(ContentType::TEXT_PLAIN)
        });
        mixed = mixed.singlepart(MailAttachment::new(a.name.clone()).body(a.data.to_vec(), ct));
    }
    Ok(b.multipart(mixed)?)
}

/// Total size of the attachments of one message. Most servers refuse more than 25 MB.
pub const MAX_ATTACHMENTS: usize = 25 * 1024 * 1024;

/// Minimal HTML escaping for the plain body placed inside the HTML alternative.
pub fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

pub fn send(account: &Account, password: &str, msg: &Message) -> Result<()> {
    let host = account.smtp_host.trim();
    if host.is_empty() {
        return Err(Error::Config("server SMTP mancante".into()));
    }
    let builder = match account.smtp_security {
        SmtpSecurity::Tls => SmtpTransport::relay(host)?,
        SmtpSecurity::StartTls => SmtpTransport::starttls_relay(host)?,
    };
    let transport = builder
        .port(account.smtp_port)
        .credentials(Credentials::new(
            account.username.trim().to_string(),
            password.to_string(),
        ))
        .timeout(Some(Duration::from_secs(60)))
        .build();
    transport.send(msg)?;
    Ok(())
}

/// "a@b.it, Nome <c@d.it>; e@f.it" → three entries. Commas inside a quoted name are kept.
pub fn split_addresses(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut in_angle = false;
    for c in s.chars() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                cur.push(c);
            }
            '<' if !in_quotes => {
                in_angle = true;
                cur.push(c);
            }
            '>' if !in_quotes => {
                in_angle = false;
                cur.push(c);
            }
            ',' | ';' if !in_quotes && !in_angle => {
                let t = cur.trim();
                if !t.is_empty() {
                    out.push(t.to_string());
                }
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    let t = cur.trim();
    if !t.is_empty() {
        out.push(t.to_string());
    }
    out
}

fn angle(id: &str) -> String {
    let t = id.trim();
    if t.starts_with('<') {
        t.to_string()
    } else {
        format!("<{t}>")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account() -> Account {
        Account {
            id: "t".into(),
            name: "Tester".into(),
            email: "t@example.it".into(),
            username: "t@example.it".into(),
            imap_host: "imap.example.it".into(),
            imap_port: 993,
            smtp_host: "smtp.example.it".into(),
            smtp_port: 465,
            smtp_security: SmtpSecurity::Tls,
            color: 0,
            signature: String::new(),
            signature_kind: crate::config::SignatureKind::Text,
            signature_html: String::new(),
            style: None,
        }
    }

    #[test]
    fn splits_addresses() {
        assert_eq!(
            split_addresses("a@b.it, \"Rossi, Anna\" <c@d.it>; e@f.it"),
            vec!["a@b.it", "\"Rossi, Anna\" <c@d.it>", "e@f.it"]
        );
        assert!(split_addresses(" , ;").is_empty());
    }

    #[test]
    fn builds_a_reply() {
        let out = OutgoingMessage {
            to: vec!["x@y.it".into()],
            cc: vec![],
            bcc: vec!["hidden@y.it".into()],
            subject: "Re: ciao".into(),
            body: "ok".into(),
            in_reply_to: Some("1@y.it".into()),
            references: vec!["<0@y.it>".into(), "1@y.it".into()],
            html_signature: None,
            html_body: None,
            attachments: Vec::new(),
        };
        let m = build(&account(), &out).unwrap();
        let raw = String::from_utf8(m.formatted()).unwrap();
        assert!(raw.contains("In-Reply-To: <1@y.it>"));
        assert!(raw.contains("References: <0@y.it> <1@y.it>"));
        assert!(
            raw.contains("From: \"Tester\" <t@example.it>")
                || raw.contains("From: Tester <t@example.it>")
        );
    }

    #[test]
    fn html_signature_makes_a_multipart_message() {
        let out = OutgoingMessage {
            to: vec!["x@y.it".into()],
            subject: "s".into(),
            body: "Ciao <tutti>".into(),
            html_signature: Some("<b>Edoardo</b> Mancinelli".into()),
            ..Default::default()
        };
        let raw = String::from_utf8(build(&account(), &out).unwrap().formatted()).unwrap();
        assert!(raw.contains("multipart/alternative"), "{raw}");
        assert!(raw.contains("text/html"));
        assert!(raw.contains("Ciao &lt;tutti"), "{raw}");
        assert!(raw.contains("<b>Edoardo</b>"));
        assert!(raw.contains("-- \r\nEdoardo Mancinelli"), "{raw}");
        assert_eq!(escape_html("a&b<c>"), "a&amp;b&lt;c&gt;");
    }

    #[test]
    fn attachments_make_a_mixed_message() {
        let out = OutgoingMessage {
            to: vec!["x@y.it".into()],
            subject: "Fwd: fattura".into(),
            body: "In allegato.".into(),
            attachments: vec![crate::model::Attachment {
                name: "Fattura 28-2026.pdf".into(),
                mime: "application/pdf".into(),
                content_id: None,
                data: std::sync::Arc::from(&b"%PDF-1.4 test"[..]),
            }],
            ..Default::default()
        };
        let m = build(&account(), &out).unwrap();
        let raw = String::from_utf8(m.formatted()).unwrap();
        assert!(raw.contains("multipart/mixed"), "{raw}");
        assert!(raw.contains("application/pdf"));
        assert!(raw.contains("Fattura 28-2026.pdf"));
        assert!(raw.contains("In allegato."));
        let back = crate::imap_client::parse_body(&m.formatted());
        assert_eq!(back.attachments.len(), 1);
        assert_eq!(
            back.attachments.first().map(|a| a.data.as_ref()),
            Some(&b"%PDF-1.4 test"[..])
        );
    }

    #[test]
    fn rejects_no_recipients_and_bad_address() {
        let out = OutgoingMessage {
            subject: "s".into(),
            ..Default::default()
        };
        assert!(build(&account(), &out).is_err());
        let out = OutgoingMessage {
            to: vec!["non-un-indirizzo".into()],
            ..Default::default()
        };
        assert!(build(&account(), &out).is_err());
    }
}
