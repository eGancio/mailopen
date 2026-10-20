//! Plain data passed between the worker and the GUI.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FolderKind {
    Inbox,
    Drafts,
    Sent,
    Archive,
    Spam,
    Trash,
    Other,
}

impl FolderKind {
    pub fn label(self) -> &'static str {
        match self {
            FolderKind::Inbox => "Posta in arrivo",
            FolderKind::Drafts => "Bozze",
            FolderKind::Sent => "Inviata",
            FolderKind::Archive => "Archivio",
            FolderKind::Spam => "Spam",
            FolderKind::Trash => "Cestino",
            FolderKind::Other => "",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Folder {
    /// Exact IMAP mailbox name (what SELECT wants).
    pub name: String,
    pub kind: FolderKind,
    /// Whether the mailbox can be selected (containers like `[Gmail]` cannot).
    pub selectable: bool,
}

impl Folder {
    /// Display name: the standard label for known kinds, else the last path segment.
    pub fn display(&self) -> String {
        if self.kind != FolderKind::Other {
            return self.kind.label().to_string();
        }
        self.name
            .rsplit(['/', '.'])
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or(&self.name)
            .to_string()
    }

    /// Classify a mailbox from its special-use attributes (RFC 6154) and, failing that, its name.
    pub fn classify(name: &str, special_use: &[String]) -> FolderKind {
        for attr in special_use {
            match attr.to_ascii_lowercase().as_str() {
                "\\sent" => return FolderKind::Sent,
                "\\drafts" => return FolderKind::Drafts,
                "\\trash" => return FolderKind::Trash,
                "\\junk" => return FolderKind::Spam,
                "\\archive" | "\\all" => return FolderKind::Archive,
                "\\inbox" => return FolderKind::Inbox,
                _ => {}
            }
        }
        let lower = name.to_ascii_lowercase();
        let last = lower.rsplit(['/', '.']).next().unwrap_or(&lower);
        if lower == "inbox" {
            FolderKind::Inbox
        } else if matches!(
            last,
            "sent"
                | "sent items"
                | "sent mail"
                | "sent messages"
                | "posta inviata"
                | "inviata"
                | "inviati"
        ) {
            FolderKind::Sent
        } else if matches!(last, "drafts" | "draft" | "bozze") {
            FolderKind::Drafts
        } else if matches!(
            last,
            "trash" | "deleted" | "deleted items" | "deleted messages" | "cestino" | "bin"
        ) {
            FolderKind::Trash
        } else if matches!(last, "junk" | "spam" | "junk e-mail" | "posta indesiderata") {
            FolderKind::Spam
        } else if matches!(last, "archive" | "archivio" | "all mail") {
            FolderKind::Archive
        } else {
            FolderKind::Other
        }
    }
}

/// One row of the message list. Headers only: the body is fetched on demand.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageSummary {
    pub account_id: String,
    pub folder: String,
    pub uid: u32,
    pub from_name: String,
    pub from_addr: String,
    pub subject: String,
    /// Unix seconds (UTC). 0 when unknown.
    pub timestamp: i64,
    pub seen: bool,
    pub answered: bool,
    pub flagged: bool,
    pub size: u32,
    pub message_id: String,
    /// First ~120 characters of the text, one line, for the list.
    pub snippet: String,
    pub has_attachment: bool,
}

impl MessageSummary {
    pub fn key(&self) -> (String, String, u32) {
        (self.account_id.clone(), self.folder.clone(), self.uid)
    }

    /// What to show in the "from" column: the name, else the address.
    pub fn sender(&self) -> &str {
        if self.from_name.trim().is_empty() {
            &self.from_addr
        } else {
            &self.from_name
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MessageBody {
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub reply_to: Option<String>,
    /// Plain text, or the HTML part rendered to text.
    pub text: String,
    pub was_html: bool,
    pub attachments: Vec<String>,
    pub message_id: String,
    pub references: Vec<String>,
}

/// A message to send. Addresses are "Name <addr>" or bare addresses, one per entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OutgoingMessage {
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: String,
    pub body: String,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    /// When set, the message goes out as text + HTML, the HTML part ending with this signature.
    pub html_signature: Option<String>,
    /// HTML rendering of `body` (from the editor's markup). When `None`, the body is escaped.
    pub html_body: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_by_attribute_then_name() {
        assert_eq!(
            Folder::classify("Qualsiasi", &["\\Sent".into()]),
            FolderKind::Sent
        );
        assert_eq!(Folder::classify("[Gmail]/Sent Mail", &[]), FolderKind::Sent);
        assert_eq!(Folder::classify("INBOX.Cestino", &[]), FolderKind::Trash);
        assert_eq!(Folder::classify("INBOX", &[]), FolderKind::Inbox);
        assert_eq!(Folder::classify("Progetti/2026", &[]), FolderKind::Other);
    }

    #[test]
    fn display_uses_last_segment_for_other() {
        let f = Folder {
            name: "INBOX.Progetti.Spotlink".into(),
            kind: FolderKind::Other,
            selectable: true,
        };
        assert_eq!(f.display(), "Spotlink");
        let f = Folder {
            name: "INBOX".into(),
            kind: FolderKind::Inbox,
            selectable: true,
        };
        assert_eq!(f.display(), "Posta in arrivo");
    }
}
