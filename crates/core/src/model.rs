//! Plain data passed between the worker and the GUI.

use std::sync::Arc;

use crate::html::Doc;

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

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MessageBody {
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub reply_to: Option<String>,
    /// Plain text: the text part, or the HTML part rendered to text. Used for replies.
    pub text: String,
    /// What the reading pane draws: the HTML part when there is one, else the text.
    pub doc: Arc<Doc>,
    pub is_html: bool,
    /// The HTML part as sent (for printing), when there is one.
    pub html: Option<String>,
    /// Files to list, open and save.
    pub attachments: Vec<Attachment>,
    /// Parts the HTML shows in place (`cid:` images); not listed as attachments.
    pub inline: Vec<Attachment>,
    pub message_id: String,
    pub references: Vec<String>,
}

impl MessageBody {
    /// The inline part a `cid:` URL points to.
    pub fn inline_part(&self, cid_url: &str) -> Option<&Attachment> {
        let id = normalize_cid(cid_url.get(4..).unwrap_or(""));
        self.inline
            .iter()
            .chain(self.attachments.iter())
            .find(|a| a.content_id.as_deref() == Some(id.as_str()))
    }
}

/// `<Abc@x>` → `abc@x`.
pub fn normalize_cid(id: &str) -> String {
    id.trim().trim_matches(['<', '>']).to_ascii_lowercase()
}

/// A file inside a message.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Attachment {
    pub name: String,
    /// `type/subtype`, lowercased.
    pub mime: String,
    /// `Content-ID` without angle brackets, lowercased.
    pub content_id: Option<String>,
    pub data: Arc<[u8]>,
}

impl std::fmt::Debug for Attachment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Attachment")
            .field("name", &self.name)
            .field("mime", &self.mime)
            .field("content_id", &self.content_id)
            .field("size", &self.data.len())
            .finish()
    }
}

/// What a file is, for the icon and the viewer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FileKind {
    Image,
    Pdf,
    Text,
    Message,
    Document,
    Spreadsheet,
    Presentation,
    Archive,
    Audio,
    Video,
    Other,
}

impl Attachment {
    pub fn size(&self) -> usize {
        self.data.len()
    }

    /// Lowercased extension of the name, without the dot.
    pub fn extension(&self) -> String {
        extension_of(&self.name)
    }

    /// From the extension first (senders often say `application/octet-stream`), then the type.
    pub fn kind(&self) -> FileKind {
        file_kind(&self.name, &self.mime)
    }
}

/// Lowercased extension of a file name, without the dot.
pub fn extension_of(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(_, e)| e.trim().to_ascii_lowercase())
        .filter(|e| !e.is_empty() && e.len() <= 8)
        .unwrap_or_default()
}

/// What a file is, from its name first (senders often say `application/octet-stream`), then
/// its type.
pub fn file_kind(name: &str, mime: &str) -> FileKind {
    match extension_of(name).as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" => return FileKind::Image,
        "pdf" => return FileKind::Pdf,
        "txt" | "csv" | "log" | "md" | "json" | "xml" | "ics" | "vcf" | "ini" | "cfg" | "yaml"
        | "yml" | "toml" | "tsv" | "sql" => return FileKind::Text,
        "eml" => return FileKind::Message,
        "doc" | "docx" | "odt" | "rtf" | "pages" => return FileKind::Document,
        "xls" | "xlsx" | "xlsm" | "ods" | "numbers" => return FileKind::Spreadsheet,
        "ppt" | "pptx" | "odp" | "key" => return FileKind::Presentation,
        "zip" | "rar" | "7z" | "gz" | "tar" | "tgz" | "bz2" | "xz" => return FileKind::Archive,
        "mp3" | "wav" | "m4a" | "ogg" | "flac" | "aac" => return FileKind::Audio,
        "mp4" | "mov" | "avi" | "mkv" | "webm" | "wmv" => return FileKind::Video,
        "htm" | "html" | "svg" | "heic" | "tif" | "tiff" | "p7m" | "p7s" => {
            return FileKind::Other;
        }
        _ => {}
    }
    let m = mime;
    if matches!(
        m,
        "image/png" | "image/jpeg" | "image/jpg" | "image/gif" | "image/webp" | "image/bmp"
    ) {
        FileKind::Image
    } else if m == "application/pdf" {
        FileKind::Pdf
    } else if m == "message/rfc822" {
        FileKind::Message
    } else if m.starts_with("text/") && m != "text/html" {
        FileKind::Text
    } else if m.starts_with("audio/") {
        FileKind::Audio
    } else if m.starts_with("video/") {
        FileKind::Video
    } else if m.contains("zip") || m.contains("compressed") {
        FileKind::Archive
    } else if m.contains("word") || m.contains("opendocument.text") {
        FileKind::Document
    } else if m.contains("excel") || m.contains("spreadsheet") {
        FileKind::Spreadsheet
    } else if m.contains("powerpoint") || m.contains("presentation") {
        FileKind::Presentation
    } else {
        FileKind::Other
    }
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
    pub attachments: Vec<Attachment>,
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
